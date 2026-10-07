// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.contacts

import android.Manifest
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.util.Log
import androidx.core.content.ContextCompat
import app.nectarlink.core.Contact
import app.nectarlink.core.ContactNumber
import java.io.ByteArrayOutputStream

/**
 * The phone's contacts with phone numbers, for PCs (docs/protocol/contacts.md):
 * lists and searches contacts (favorites first, then alphabetical) and
 * reports when the address book changes.
 */
internal class PhoneContacts(context: Context, private val onChange: () -> Unit) {
    private val context = context.applicationContext
    private val resolver = context.contentResolver
    private val handler = Handler(Looper.getMainLooper())
    private var observer: ContentObserver? = null
    private val notify = Runnable { onChange() }

    fun start() {
        if (observer != null || !canRead(context)) return
        runCatching {
            observer = object : ContentObserver(handler) {
                override fun onChange(selfChange: Boolean) {
                    handler.removeCallbacks(notify)
                    handler.postDelayed(notify, SETTLE_MS)
                }
            }.also {
                resolver.registerContentObserver(ContactsContract.Contacts.CONTENT_URI, true, it)
            }
        }.onFailure { Log.w(TAG, "couldn't watch contacts", it) }
    }

    fun stop() {
        observer?.let { runCatching { resolver.unregisterContentObserver(it) } }
        observer = null
        handler.removeCallbacks(notify)
    }

    private class Draft(
        val id: String,
        val name: String,
        val starred: Boolean,
        val photoUri: String?,
        val numbers: MutableList<ContactNumber> = ArrayList(2),
        val seenNumbers: HashSet<String> = HashSet(2),
    )

    fun list(query: String?, offset: Int, limit: Int): List<Contact> {
        check(canRead(context)) { "Contacts permission not granted" }
        return try {
            val drafts = LinkedHashMap<Long, Draft>()
            val columns = arrayOf(
                Phone.CONTACT_ID,
                Phone.DISPLAY_NAME_PRIMARY,
                Phone.STARRED,
                Phone.PHOTO_THUMBNAIL_URI,
                Phone.NUMBER,
                Phone.TYPE,
                Phone.LABEL,
            )
            val order = "${Phone.STARRED} DESC, ${Phone.DISPLAY_NAME_PRIMARY} COLLATE LOCALIZED ASC, ${Phone.CONTACT_ID} ASC"
            resolver.query(Phone.CONTENT_URI, columns, null, null, order)?.use { c ->
                while (c.moveToNext()) {
                    val contactId = c.getLong(0)
                    val name = c.getString(1)?.trim().orEmpty()
                    val number = c.getString(4)?.trim().orEmpty()
                    if (name.isEmpty() || number.isEmpty()) continue
                    val draft = drafts.getOrPut(contactId) {
                        Draft(
                            id = contactId.toString(),
                            name = name,
                            starred = c.getInt(2) != 0,
                            photoUri = c.getString(3),
                        )
                    }
                    if (draft.numbers.size >= MAX_NUMBERS) continue
                    val key = number.filter(Char::isDigit).ifEmpty { number }
                    if (draft.seenNumbers.add(key)) {
                        draft.numbers += ContactNumber(
                            number = number,
                            label = phoneLabel(c.getInt(5), c.getString(6)),
                        )
                    }
                }
            }
            val q = query?.trim()?.takeIf { it.isNotEmpty() }
            val qDigits = q?.filter(Char::isDigit).orEmpty()
            val filtered = if (q == null) {
                drafts.values.asSequence()
            } else {
                drafts.values.asSequence().filter { d ->
                    d.name.contains(q, ignoreCase = true) ||
                        d.numbers.any { n ->
                            n.number.contains(q, ignoreCase = true) ||
                                (qDigits.isNotEmpty() && n.number.filter(Char::isDigit).contains(qDigits))
                        }
                }
            }
            filtered
                .drop(offset.coerceAtLeast(0))
                .take(limit.coerceIn(1, MAX_PAGE))
                .map { d ->
                    Contact(
                        id = d.id,
                        name = d.name,
                        numbers = d.numbers,
                        starred = d.starred,
                        photo = d.photoUri?.let(Uri::parse)?.let { smallPhoto(resolver, it) },
                    )
                }
                .toList()
        } catch (e: Exception) {
            Log.w(TAG, "can't read contacts", e)
            emptyList()
        }
    }

    private fun phoneLabel(type: Int, custom: String?): String? = when (type) {
        Phone.TYPE_CUSTOM -> custom?.trim()?.takeIf { it.isNotEmpty() }
        Phone.TYPE_MOBILE -> "Mobile"
        Phone.TYPE_HOME -> "Home"
        Phone.TYPE_WORK -> "Work"
        Phone.TYPE_MAIN -> "Main"
        Phone.TYPE_FAX_WORK, Phone.TYPE_FAX_HOME -> "Fax"
        Phone.TYPE_PAGER -> "Pager"
        Phone.TYPE_OTHER -> "Other"
        else -> null
    }

    companion object {
        private const val TAG = "PhoneContacts"
        private const val SETTLE_MS = 500L
        private const val MAX_PAGE = 200
        private const val MAX_NUMBERS = 8
        private const val MAX_PHOTO_BYTES = 16 * 1024
        private const val PHOTO_EDGE = 96

        val permissions: Array<String> = arrayOf(Manifest.permission.READ_CONTACTS)

        fun canRead(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CONTACTS) ==
                PackageManager.PERMISSION_GRANTED

        /** A contact thumbnail as a JPEG of at most 16 KiB. */
        internal fun smallPhoto(resolver: ContentResolver, uri: Uri): ByteArray? = runCatching {
            val raw = resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) } ?: return null
            val bitmap = if (raw.width > PHOTO_EDGE || raw.height > PHOTO_EDGE) {
                Bitmap.createScaledBitmap(raw, PHOTO_EDGE, PHOTO_EDGE, true).also {
                    if (it !== raw) raw.recycle()
                }
            } else {
                raw
            }
            try {
                generateSequence(85) { it - 15 }.takeWhile { it >= 40 }
                    .map { q ->
                        ByteArrayOutputStream().also {
                            bitmap.compress(Bitmap.CompressFormat.JPEG, q, it)
                        }.toByteArray()
                    }
                    .firstOrNull { it.size <= MAX_PHOTO_BYTES }
            } finally {
                bitmap.recycle()
            }
        }.getOrNull()
    }
}
