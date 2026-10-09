// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.sms

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.database.Cursor
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.provider.Telephony
import android.telephony.SmsManager
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.nectarlink.core.SmsAttachment
import app.nectarlink.core.SmsMessage
import app.nectarlink.core.SmsPart
import app.nectarlink.core.SmsPartData
import app.nectarlink.core.SmsThread
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * The phone's text messages, for PCs (docs/protocol/sms.md): lists
 * conversations and messages (SMS and MMS), sends texts, and says when
 * anything changed. Only reads: Android lets only the default SMS app
 * change the messages store.
 */
internal class PhoneSms(context: Context, private val onChange: () -> Unit) {
    private val context = context.applicationContext
    private val resolver = context.contentResolver
    private val handler = Handler(Looper.getMainLooper())
    private var observer: ContentObserver? = null
    private val notify = Runnable { onChange() }

    fun start() {
        if (observer != null || !canRead(context)) return
        observer = object : ContentObserver(handler) {
            // Sending or receiving touches several rows: say it once.
            override fun onChange(selfChange: Boolean) {
                handler.removeCallbacks(notify)
                handler.postDelayed(notify, SETTLE_MS)
            }
        }.also { resolver.registerContentObserver(Uri.parse("content://mms-sms/"), true, it) }
    }

    fun stop() {
        observer?.let { resolver.unregisterContentObserver(it) }
        observer = null
        handler.removeCallbacks(notify)
    }

    // ---- Conversations ----

    fun threads(limit: Int): List<SmsThread> = guarded(emptyList()) {
        val addresses = canonicalAddresses()
        val contacts = Contacts()
        val uri = Uri.parse("content://mms-sms/conversations?simple=true")
        val columns = arrayOf(
            Telephony.Threads._ID,
            Telephony.Threads.DATE,
            Telephony.Threads.RECIPIENT_IDS,
            Telephony.Threads.SNIPPET,
            Telephony.Threads.READ,
            Telephony.Threads.HAS_ATTACHMENT,
        )
        resolver.query(uri, columns, null, null, "${Telephony.Threads.DATE} DESC")?.use { c ->
            buildList {
                while (size < limit && c.moveToNext()) {
                    val id = c.getLong(0)
                    val people = c.getString(2).orEmpty().split(' ').mapNotNull { it.toLongOrNull()?.let(addresses::get) }
                    if (people.isEmpty()) continue
                    val names = people.map { contacts.name(it).orEmpty() }
                    val snippet = c.getString(3)?.takeIf { it.isNotBlank() }
                        ?: if (c.getInt(5) != 0) PICTURE_SNIPPET else ""
                    add(
                        SmsThread(
                            id = id.toString(),
                            addresses = people,
                            names = names,
                            snippet = snippet,
                            date = c.getLong(1),
                            unread = if (c.getInt(4) == 0) unreadIn(id) else 0u,
                            photo = people.singleOrNull()?.let(contacts::photo),
                        ),
                    )
                }
            }
        }.orEmpty()
    }

    private fun unreadIn(thread: Long): UInt =
        resolver.query(
            Telephony.Sms.CONTENT_URI,
            arrayOf(Telephony.Sms._ID),
            "${Telephony.Sms.THREAD_ID} = ? AND ${Telephony.Sms.READ} = 0",
            arrayOf(thread.toString()),
            null,
        )?.use { it.count.toUInt() } ?: 0u

    /** Recipient IDs (as conversations list them) to numbers. */
    private fun canonicalAddresses(): Map<Long, String> =
        resolver.query(Uri.parse("content://mms-sms/canonical-addresses"), arrayOf("_id", "address"), null, null, null)?.use { c ->
            buildMap { while (c.moveToNext()) c.getString(1)?.let { put(c.getLong(0), it) } }
        }.orEmpty()

    // ---- Messages ----

    fun messages(thread: String, before: Long?, limit: Int): List<SmsMessage> = guarded(emptyList()) {
        val threadId = thread.toLongOrNull() ?: return@guarded emptyList()
        (sms(threadId, before, limit) + mms(threadId, before, limit))
            .sortedByDescending { it.date }
            .take(limit)
    }

    private fun sms(thread: Long, before: Long?, limit: Int): List<SmsMessage> {
        val columns = arrayOf(
            Telephony.Sms._ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY, Telephony.Sms.DATE, Telephony.Sms.TYPE,
        )
        val (selection, args) = page(Telephony.Sms.THREAD_ID, Telephony.Sms.DATE, thread, before)
        return resolver.query(Telephony.Sms.CONTENT_URI, columns, selection, args, "${Telephony.Sms.DATE} DESC")?.use { c ->
            buildList {
                while (size < limit && c.moveToNext()) {
                    val type = c.getInt(4)
                    add(
                        SmsMessage(
                            id = "sms:${c.getLong(0)}",
                            thread = thread.toString(),
                            address = c.getString(1).orEmpty(),
                            body = c.getString(2).orEmpty(),
                            date = c.getLong(3),
                            outgoing = type != Telephony.Sms.MESSAGE_TYPE_INBOX,
                            status = sentStatus(type),
                            parts = emptyList(),
                        ),
                    )
                }
            }
        }.orEmpty()
    }

    private fun sentStatus(type: Int): String? = when (type) {
        Telephony.Sms.MESSAGE_TYPE_SENT -> "sent"
        Telephony.Sms.MESSAGE_TYPE_OUTBOX, Telephony.Sms.MESSAGE_TYPE_QUEUED -> "pending"
        Telephony.Sms.MESSAGE_TYPE_FAILED -> "failed"
        else -> null
    }

    private fun mms(thread: Long, before: Long?, limit: Int): List<SmsMessage> {
        val columns = arrayOf(Telephony.Mms._ID, Telephony.Mms.DATE, Telephony.Mms.MESSAGE_BOX)
        // MMS dates are in seconds.
        val (selection, args) = page(Telephony.Mms.THREAD_ID, Telephony.Mms.DATE, thread, before?.div(1000))
        return resolver.query(Telephony.Mms.CONTENT_URI, columns, selection, args, "${Telephony.Mms.DATE} DESC")?.use { c ->
            buildList {
                while (size < limit && c.moveToNext()) {
                    val id = c.getLong(0)
                    val box = c.getInt(2)
                    val outgoing = box != Telephony.Mms.MESSAGE_BOX_INBOX
                    val (body, parts) = mmsParts(id)
                    add(
                        SmsMessage(
                            id = "mms:$id",
                            thread = thread.toString(),
                            address = mmsAddress(id, outgoing),
                            body = body,
                            date = c.getLong(1) * 1000,
                            outgoing = outgoing,
                            status = when (box) {
                                Telephony.Mms.MESSAGE_BOX_SENT -> "sent"
                                Telephony.Mms.MESSAGE_BOX_OUTBOX -> "pending"
                                Telephony.Mms.MESSAGE_BOX_FAILED -> "failed"
                                else -> null
                            },
                            parts = parts,
                        ),
                    )
                }
            }
        }.orEmpty()
    }

    /** Selection for a thread's messages, before a date when paging back. */
    private fun page(threadColumn: String, dateColumn: String, thread: Long, before: Long?): Pair<String, Array<String>> =
        if (before == null) {
            "$threadColumn = ?" to arrayOf(thread.toString())
        } else {
            "$threadColumn = ? AND $dateColumn < ?" to arrayOf(thread.toString(), before.toString())
        }

    /** An MMS's text, and its pictures (and other attachments). */
    private fun mmsParts(message: Long): Pair<String, List<SmsPart>> {
        val text = StringBuilder()
        val parts = mutableListOf<SmsPart>()
        resolver.query(
            Uri.parse("content://mms/part"),
            arrayOf("_id", "ct", "text"),
            "mid = ?",
            arrayOf(message.toString()),
            null,
        )?.use { c ->
            while (c.moveToNext()) {
                val mime = c.getString(1).orEmpty()
                when {
                    mime == "text/plain" -> c.getString(2)?.let { if (text.isNotEmpty()) text.append('\n'); text.append(it) }
                    mime == "application/smil" -> {}
                    else -> parts += SmsPart(id = "mms-part:${c.getLong(0)}", mime = mime, size = 0u)
                }
            }
        }
        return text.toString() to parts
    }

    /** Who sent an MMS (received), or its first recipient (sent). */
    private fun mmsAddress(message: Long, outgoing: Boolean): String {
        val type = if (outgoing) PDU_TO else PDU_FROM
        return resolver.query(
            Uri.parse("content://mms/$message/addr"),
            arrayOf("address"),
            "type = ?",
            arrayOf(type.toString()),
            null,
        )?.use { if (it.moveToFirst()) it.getString(0) else null }.orEmpty()
    }

    fun part(id: String): SmsPartData? = guarded(null) {
        val partId = id.removePrefix("mms-part:").takeIf { id.startsWith("mms-part:") }?.toLongOrNull() ?: return@guarded null
        val uri = Uri.parse("content://mms/part/$partId")
        val mime = resolver.query(uri, arrayOf("ct"), null, null, null)?.use { if (it.moveToFirst()) it.getString(0) else null }
            ?: return@guarded null
        val data = resolver.openInputStream(uri)?.use { input ->
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                out.write(buffer, 0, n)
                if (out.size() > MAX_PART_BYTES) return@guarded null
            }
            out.toByteArray()
        } ?: return@guarded null
        SmsPartData(mime, data)
    }

    // ---- Sending ----

    /** Sends a text or MMS to each recipient; Android keeps it in Sent. */
    fun send(to: List<String>, body: String, attachments: List<SmsAttachment> = emptyList()): Boolean =
        send(context, to, body, attachments)

    private inline fun <T> guarded(fallback: T, block: () -> T): T = try {
        block()
    } catch (e: Exception) {
        Log.w(TAG, "can't read or send messages", e)
        fallback
    }

    /** Contact names and photos, looked up once per number. */
    private inner class Contacts {
        private val allowed = granted(context, Manifest.permission.READ_CONTACTS)
        private val found = HashMap<String, Pair<String?, String?>>()

        private fun lookup(number: String): Pair<String?, String?> = found.getOrPut(number) {
            if (!allowed) return@getOrPut null to null
            val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
            runCatching {
                resolver.query(
                    uri,
                    arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME, ContactsContract.PhoneLookup.PHOTO_THUMBNAIL_URI),
                    null,
                    null,
                    null,
                )?.use { c: Cursor -> if (c.moveToFirst()) c.getString(0) to c.getString(1) else null }
            }.getOrNull() ?: (null to null)
        }

        fun name(number: String): String? = lookup(number).first

        fun photo(number: String): ByteArray? {
            val uri = lookup(number).second?.let(Uri::parse) ?: return null
            return runCatching {
                val bitmap = resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) } ?: return null
                try {
                    generateSequence(85) { it - 15 }.takeWhile { it >= 40 }
                        .map { q -> ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, q, it) }.toByteArray() }
                        .firstOrNull { it.size <= MAX_PHOTO_BYTES }
                } finally {
                    bitmap.recycle()
                }
            }.getOrNull()
        }
    }

    companion object {
        private const val TAG = "PhoneSms"
        private const val SETTLE_MS = 1000L
        private const val MAX_PART_BYTES = 900 * 1024
        private const val MAX_PHOTO_BYTES = 16 * 1024
        private const val MMS_CACHE_DIR = "mms"
        private const val MMS_CACHE_TTL_MS = 5 * 60 * 1000L
        private const val RECENT_SMS_WINDOW_MS = 60_000L
        /** What a conversation whose latest message is a picture says. */
        private const val PICTURE_SNIPPET = "Picture"
        // MMS address types (PduHeaders.FROM and TO).
        private const val PDU_FROM = 137
        private const val PDU_TO = 151

        val permissions: Array<String> = arrayOf(
            Manifest.permission.READ_SMS,
            Manifest.permission.SEND_SMS,
            Manifest.permission.READ_CONTACTS,
        )

        private fun granted(context: Context, permission: String) =
            ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

        fun canRead(context: Context) = granted(context, Manifest.permission.READ_SMS)

        fun canSend(context: Context) = granted(context, Manifest.permission.SEND_SMS)

        fun hasAll(context: Context) = permissions.all { granted(context, it) }

        /**
         * Checks whether `Telephony.Sms` already contains a message with `body`
         * within 60 seconds of `aroundTimeMs` (so notifications from the default
         * SMS app aren't duplicated as `NotificationConversation`s when the SMS
         * provider already supplies them).
         */
        fun isRecentSmsInProvider(context: Context, body: String, aroundTimeMs: Long): Boolean {
            if (!canRead(context)) return false
            val trimmed = body.trim()
            if (trimmed.isEmpty()) return false
            val now = System.currentTimeMillis()
            val minDate = minOf(aroundTimeMs, now) - RECENT_SMS_WINDOW_MS
            val maxDate = maxOf(aroundTimeMs, now) + RECENT_SMS_WINDOW_MS
            return runCatching {
                context.contentResolver.query(
                    Telephony.Sms.CONTENT_URI,
                    arrayOf(Telephony.Sms.BODY),
                    "${Telephony.Sms.DATE} >= ? AND ${Telephony.Sms.DATE} <= ?",
                    arrayOf(minDate.toString(), maxDate.toString()),
                    "${Telephony.Sms.DATE} DESC",
                )?.use { c ->
                    while (c.moveToNext()) {
                        if (c.getString(0)?.trim() == trimmed) return@use true
                    }
                    false
                } ?: false
            }.getOrDefault(false)
        }

        /** Sends an SMS or MMS (when `attachments` is non-empty) to `to`. */
        fun send(
            context: Context,
            to: List<String>,
            body: String,
            attachments: List<SmsAttachment> = emptyList(),
        ): Boolean = try {
            if (!granted(context, Manifest.permission.SEND_SMS)) {
                false
            } else {
                val recipients = to.map { it.trim() }.filter { it.isNotEmpty() }
                if (recipients.isEmpty()) {
                    false
                } else {
                    val manager = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                        context.getSystemService(SmsManager::class.java)
                    } else {
                        @Suppress("DEPRECATION")
                        SmsManager.getDefault()
                    }
                    if (attachments.isEmpty()) {
                        if (body.isBlank()) {
                            false
                        } else {
                            val parts = manager.divideMessage(body)
                            for (address in recipients) {
                                manager.sendMultipartTextMessage(address, null, parts, null, null)
                            }
                            true
                        }
                    } else {
                        val now = System.currentTimeMillis()
                        val mmsDir = File(context.cacheDir, MMS_CACHE_DIR).apply { mkdirs() }
                        mmsDir.listFiles()?.forEach { f ->
                            if (f.name.endsWith(".mms") && now - f.lastModified() > MMS_CACHE_TTL_MS) {
                                f.delete()
                            }
                        }
                        val pduBytes = buildMmsSendReqPdu(recipients, body, attachments, "nl-$now")
                        val pduFile = File(mmsDir, "send-$now.mms")
                        pduFile.writeBytes(pduBytes)
                        val contentUri = FileProvider.getUriForFile(context, "${context.packageName}.files", pduFile)
                        manager.sendMultimediaMessage(context, contentUri, null, null, null)
                        true
                    }
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "can't send message", e)
            false
        }

        /**
         * Builds an OMA MMS `M-Send.req` binary PDU (`application/vnd.wap.mms-message`)
         * with `application/vnd.wap.multipart.mixed` parts for optional UTF-8 text and
         * image attachments.
         */
        internal fun buildMmsSendReqPdu(
            to: List<String>,
            body: String,
            attachments: List<SmsAttachment>,
            transactionId: String = "nl-${System.currentTimeMillis()}",
        ): ByteArray {
            val out = ByteArrayOutputStream()
            // X-Mms-Message-Type (0x8C): m-send-req (0x80)
            out.write(0x8C)
            out.write(0x80)
            // X-Mms-Transaction-Id (0x98): text-string + NUL
            out.write(0x98)
            out.write(transactionId.toByteArray(Charsets.US_ASCII))
            out.write(0x00)
            // X-Mms-MMS-Version (0x8D): v1.2 (0x92)
            out.write(0x8D)
            out.write(0x92)
            // From (0x89): value-length 0x01, Insert-address-token (0x81)
            out.write(0x89)
            out.write(0x01)
            out.write(0x81)
            // To (0x97): one per recipient
            for (raw in to) {
                val addr = raw.trim()
                if (addr.isEmpty()) continue
                val formatted = if ('@' in addr || addr.endsWith("/TYPE=PLMN")) addr else "$addr/TYPE=PLMN"
                out.write(0x97)
                out.write(formatted.toByteArray(Charsets.US_ASCII))
                out.write(0x00)
            }
            // Content-Type (0x84): application/vnd.wap.multipart.mixed (0xA3)
            out.write(0x84)
            out.write(0xA3)

            // Multipart body: uintvar count of parts
            val hasText = body.isNotBlank()
            val partCount = (if (hasText) 1 else 0) + attachments.size
            writeUintvar(out, partCount)

            if (hasText) {
                // Content-Type well-known text/plain (0x83) + UTF-8 data bytes
                val textHeader = byteArrayOf(0x83.toByte())
                val textBytes = body.toByteArray(Charsets.UTF_8)
                writeUintvar(out, textHeader.size)
                writeUintvar(out, textBytes.size)
                out.write(textHeader)
                out.write(textBytes)
            }

            for (att in attachments) {
                val mime = att.mime.trim().ifEmpty { "image/jpeg" }
                val mimeHeader = mime.toByteArray(Charsets.US_ASCII) + byteArrayOf(0x00)
                writeUintvar(out, mimeHeader.size)
                writeUintvar(out, att.data.size)
                out.write(mimeHeader)
                out.write(att.data)
            }

            return out.toByteArray()
        }

        /** Encodes a non-negative integer as a WAP WSP `uintvar` (7 bits per byte, MSB continuation). */
        internal fun writeUintvar(out: ByteArrayOutputStream, value: Int) {
            require(value >= 0) { "uintvar must be non-negative" }
            val chunks = IntArray(5)
            var count = 0
            var v = value
            do {
                chunks[count++] = v and 0x7F
                v = v ushr 7
            } while (v > 0)
            for (i in count - 1 downTo 1) {
                out.write(chunks[i] or 0x80)
            }
            out.write(chunks[0])
        }
    }
}
