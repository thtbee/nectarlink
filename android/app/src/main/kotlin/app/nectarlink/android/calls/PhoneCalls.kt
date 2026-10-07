// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.database.Cursor
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.CallLog
import android.provider.ContactsContract
import android.telecom.TelecomManager
import android.telephony.TelephonyManager
import android.util.Log
import androidx.core.content.ContextCompat
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.core.Call
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallLogEntry
import app.nectarlink.core.CallPhase
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

/**
 * The phone's calls, for PCs (docs/protocol/calls.md): follows the call
 * state while the app runs, answers, declines or silences a ringing
 * call when a PC asks, lists recent calls (`call.log`), and places calls
 * (`call.dial`). The call's audio stays on the phone.
 */
internal class PhoneCalls(
    context: Context,
    private val onChange: (Call) -> Unit,
    private val onLogChanged: () -> Unit = {},
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    /** Contact lookups and reports, in order, off the main thread. */
    private val worker = Executors.newSingleThreadExecutor()
    private var receiver: BroadcastReceiver? = null
    private var logObserver: ContentObserver? = null
    private val notifyLog = Runnable { onLogChanged() }

    /** The call in progress (changed on the main thread, read by PCs' requests). */
    @Volatile
    private var current: Tracked? = null

    /** Number dialed from a PC, used when Android's OFFHOOK broadcast omits the number. */
    @Volatile
    private var lastDialed: Pair<String, Long>? = null

    private data class Tracked(
        val id: String,
        val incoming: Boolean,
        var number: String?,
        var answered: Boolean = false,
        /** Declined from a PC: not a missed call. */
        var declined: Boolean = false,
        /** When it was answered (Unix ms). */
        var since: Long? = null,
    )

    fun start() {
        instance = this
        if (receiver == null && canFollow(context)) {
            receiver = object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    val state = intent.getStringExtra(TelephonyManager.EXTRA_STATE) ?: return
                    // Only sent with the call log permission; a second broadcast
                    // carries it when there is one.
                    @Suppress("DEPRECATION")
                    val number = intent.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)
                    stateChanged(state, number?.takeIf { it.isNotBlank() })
                }
            }.also {
                ContextCompat.registerReceiver(
                    context,
                    it,
                    IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED),
                    // A system broadcast: other apps can't send it here.
                    ContextCompat.RECEIVER_NOT_EXPORTED,
                )
            }
        }
        if (logObserver == null && canReadLog(context)) {
            runCatching {
                logObserver = object : ContentObserver(main) {
                    override fun onChange(selfChange: Boolean) {
                        main.removeCallbacks(notifyLog)
                        main.postDelayed(notifyLog, LOG_SETTLE_MS)
                    }
                }.also {
                    context.contentResolver.registerContentObserver(CallLog.Calls.CONTENT_URI, true, it)
                }
            }.onFailure { Log.w(TAG, "couldn't watch call log", it) }
        }
    }

    fun stop() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
        logObserver?.let { runCatching { context.contentResolver.unregisterContentObserver(it) } }
        logObserver = null
        main.removeCallbacks(notifyLog)
        current = null
    }

    private fun stateChanged(state: String, number: String?) {
        val call = current
        when (state) {
            TelephonyManager.EXTRA_STATE_RINGING -> when {
                call == null -> {
                    val tracked = Tracked(id = System.currentTimeMillis().toString(), incoming = true, number = number)
                    current = tracked
                    report(tracked, CallPhase.RINGING, withContact = true)
                }
                // The same call again, now with its number.
                call.number == null && number != null && !call.answered -> {
                    call.number = number
                    report(call, CallPhase.RINGING, withContact = true)
                }
            }
            TelephonyManager.EXTRA_STATE_OFFHOOK -> when {
                call == null -> {
                    // Calling out.
                    val now = System.currentTimeMillis()
                    val dialed = lastDialed?.takeIf { now - it.second < 15_000L }?.first
                    val outNumber = number ?: dialed
                    val tracked = Tracked(id = now.toString(), incoming = false, number = outNumber, answered = true, since = now)
                    current = tracked
                    report(tracked, CallPhase.ACTIVE, withContact = true)
                }
                !call.answered -> {
                    if (call.number == null && number != null) {
                        call.number = number
                    }
                    call.answered = true
                    call.since = System.currentTimeMillis()
                    report(call, CallPhase.ACTIVE, withContact = false)
                }
            }
            TelephonyManager.EXTRA_STATE_IDLE -> if (call != null) {
                current = null
                lastDialed = null
                restoreRinger()
                if (call.incoming && !call.answered && !call.declined) {
                    // Missed, or turned down on the phone: the call log knows,
                    // once it's written.
                    main.postDelayed({ report(call, CallPhase.ENDED, withContact = true, missedCheck = true) }, LOG_DELAY_MS)
                } else {
                    report(call, CallPhase.ENDED, withContact = false)
                }
            }
        }
    }

    private fun report(call: Tracked, phase: CallPhase, withContact: Boolean, missedCheck: Boolean = false) {
        // A snapshot: the call may change while this waits its turn.
        val number = call.number
        val incoming = call.incoming
        val since = call.since
        worker.execute { send(call.id, phase, incoming, number, since, withContact, missedCheck) }
    }

    private fun send(
        id: String,
        phase: CallPhase,
        incoming: Boolean,
        number: String?,
        since: Long?,
        withContact: Boolean,
        missedCheck: Boolean,
    ) {
        val (name, photo) = if (withContact) contact(number) else null to null
        val missed = missedCheck && wasMissed(id, number)
        onChange(
            Call(
                id = id,
                state = phase,
                incoming = incoming,
                number = number,
                name = name,
                photo = photo.takeIf { phase == CallPhase.RINGING || (!incoming && phase == CallPhase.ACTIVE) },
                missed = missed,
                since = since.takeIf { phase == CallPhase.ACTIVE },
                controls = if (phase == CallPhase.ACTIVE) CallCompanion.controls() else null,
            ),
        )
    }

    /** The companion's mute, speaker or hold changed: PCs hear of it. */
    private fun resendActive() {
        main.post {
            current?.takeIf { it.answered }?.let { report(it, CallPhase.ACTIVE, withContact = true) }
        }
    }

    /** The contact's name and photo for a number, when allowed and known. */
    private fun contact(number: String?): Pair<String?, ByteArray?> {
        if (number == null || !granted(context, Manifest.permission.READ_CONTACTS)) return null to null
        return runCatching {
            val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
            val columns = arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME, ContactsContract.PhoneLookup.PHOTO_THUMBNAIL_URI)
            context.contentResolver.query(uri, columns, null, null, null)?.use { cursor ->
                if (!cursor.moveToFirst()) return@use null to null
                val photo = cursor.getString(1)?.let { photoBytes(Uri.parse(it)) }
                cursor.getString(0) to photo
            } ?: (null to null)
        }.getOrElse {
            Log.i(TAG, "no contact for a call", it)
            null to null
        }
    }

    /** A JPEG of at most 64 KB. */
    private fun photoBytes(uri: Uri): ByteArray? {
        val bitmap = context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) } ?: return null
        return try {
            generateSequence(90) { it - 15 }.takeWhile { it >= 45 }
                .map { quality -> ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, quality, it) }.toByteArray() }
                .firstOrNull { it.size <= MAX_PHOTO_BYTES }
        } finally {
            bitmap.recycle()
        }
    }

    /** Whether the call that just ended is in the log as a missed one (not turned down or blocked). */
    @SuppressLint("MissingPermission")
    private fun wasMissed(id: String, number: String?): Boolean {
        if (!granted(context, Manifest.permission.READ_CALL_LOG)) return true
        val started = id.toLongOrNull() ?: 0L
        return runCatching {
            context.contentResolver.query(
                CallLog.Calls.CONTENT_URI.buildUpon().appendQueryParameter(CallLog.Calls.LIMIT_PARAM_KEY, "1").build(),
                arrayOf(CallLog.Calls.TYPE, CallLog.Calls.DATE, CallLog.Calls.NUMBER),
                null,
                null,
                "${CallLog.Calls.DATE} DESC",
            )?.use { cursor ->
                if (!cursor.moveToFirst()) return@use true
                val date = cursor.getLong(1)
                val loggedNumber = cursor.getString(2)
                if (started > 0L && date < started - 10_000L) return@use true
                if (number != null && !loggedNumber.isNullOrBlank() && !sameNumber(number, loggedNumber)) {
                    return@use true
                }
                cursor.getInt(0) == CallLog.Calls.MISSED_TYPE
            } ?: true
        }.getOrDefault(true)
    }

    private fun sameNumber(a: String, b: String): Boolean {
        val da = a.filter { it.isDigit() }.takeLast(9)
        val db = b.filter { it.isDigit() }.takeLast(9)
        return da.isNotEmpty() && da == db
    }

    /**
     * Runs what a PC asked; false if the phone couldn't. The companion
     * (when Telecom has bound it) controls the call itself; otherwise
     * Telecom answers and ends it, and in-call controls aren't there.
     */
    @SuppressLint("MissingPermission")
    fun command(id: String, command: CallCommand): Boolean {
        val call = current?.takeIf { it.id == id } ?: return false
        return runCatching {
            when (command) {
                CallCommand.Answer -> CallCompanion.answer() || telecom()?.let {
                    // Deprecated for dialer apps, which get the call directly;
                    // still how any other app answers one.
                    @Suppress("DEPRECATION")
                    it.acceptRingingCall()
                    true
                } == true
                CallCommand.Decline -> {
                    call.declined = !call.answered
                    CallCompanion.hangUp() || hangUpWithTelecom()
                }
                CallCommand.Silence -> telecom()?.let { silence(it); true } == true
                is CallCommand.Mute -> CallCompanion.mute(command.on)
                is CallCommand.Speaker -> CallCompanion.speaker(command.on)
                is CallCommand.Hold -> CallCompanion.hold(command.on)
                is CallCommand.Dtmf -> command.digit.singleOrNull()?.let(CallCompanion::dtmf) ?: false
                is CallCommand.Volume -> {
                    context.getSystemService(AudioManager::class.java).adjustStreamVolume(
                        AudioManager.STREAM_VOICE_CALL,
                        if (command.up) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER,
                        0,
                    )
                    true
                }
            }
        }.getOrElse {
            Log.w(TAG, "a call action failed", it)
            false
        }
    }

    /** Recent calls from the phone's call history (`call.log`). */
    @SuppressLint("MissingPermission")
    fun callLog(before: Long?, limit: Int): List<CallLogEntry> {
        check(canReadLog(context)) { "Call log permission not granted" }
        return try {
            val resolver = context.contentResolver
            val contactsAllowed = granted(context, Manifest.permission.READ_CONTACTS)
            val contactCache = HashMap<String, Pair<String?, ByteArray?>>()
            fun lookupContact(number: String, cachedName: String?, cachedPhotoUri: String?): Pair<String?, ByteArray?> =
                contactCache.getOrPut(number) {
                    var name = cachedName?.trim()?.takeIf { it.isNotEmpty() }
                    var photoUri = cachedPhotoUri?.trim()?.takeIf { it.isNotEmpty() }
                    if ((name == null || photoUri == null) && contactsAllowed) {
                        runCatching {
                            val uri = Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number))
                            val cols = arrayOf(
                                ContactsContract.PhoneLookup.DISPLAY_NAME,
                                ContactsContract.PhoneLookup.PHOTO_THUMBNAIL_URI,
                            )
                            resolver.query(uri, cols, null, null, null)?.use { c: Cursor ->
                                if (c.moveToFirst()) {
                                    if (name == null) name = c.getString(0)?.trim()?.takeIf { it.isNotEmpty() }
                                    if (photoUri == null) photoUri = c.getString(1)?.trim()?.takeIf { it.isNotEmpty() }
                                }
                            }
                        }
                    }
                    val photo = photoUri?.let(Uri::parse)?.let { PhoneContacts.smallPhoto(resolver, it) }
                    name to photo
                }

            val columns = arrayOf(
                CallLog.Calls._ID,
                CallLog.Calls.NUMBER,
                CallLog.Calls.CACHED_NAME,
                CallLog.Calls.TYPE,
                CallLog.Calls.DATE,
                CallLog.Calls.DURATION,
                CallLog.Calls.CACHED_PHOTO_URI,
            )
            val (selection, args) = if (before == null) {
                null to null
            } else {
                "${CallLog.Calls.DATE} < ?" to arrayOf(before.toString())
            }
            val max = limit.coerceIn(1, MAX_LOG_PAGE)
            resolver.query(CallLog.Calls.CONTENT_URI, columns, selection, args, "${CallLog.Calls.DATE} DESC")?.use { c ->
                buildList {
                    while (size < max && c.moveToNext()) {
                        val number = c.getString(1)?.trim().orEmpty()
                        if (number.isEmpty()) continue
                        val direction = when (c.getInt(3)) {
                            CallLog.Calls.OUTGOING_TYPE -> "outgoing"
                            CallLog.Calls.MISSED_TYPE -> "missed"
                            CallLog.Calls.REJECTED_TYPE, CallLog.Calls.BLOCKED_TYPE -> "rejected"
                            else -> "incoming"
                        }
                        val (name, photo) = lookupContact(number, c.getString(2), c.getString(6))
                        add(
                            CallLogEntry(
                                id = c.getLong(0).toString(),
                                number = number,
                                name = name,
                                direction = direction,
                                date = c.getLong(4),
                                duration = c.getLong(5).coerceIn(0L, UInt.MAX_VALUE.toLong()).toUInt(),
                                photo = photo,
                            ),
                        )
                    }
                }
            }.orEmpty()
        } catch (e: Exception) {
            Log.w(TAG, "can't read call log", e)
            emptyList()
        }
    }

    /**
     * Places a call (`TelecomManager.placeCall` / `Intent.ACTION_CALL` when
     * `CALL_PHONE` is granted), or falls back to opening the dialer with the
     * number pre-filled (`Intent.ACTION_DIAL`).
     */
    @SuppressLint("MissingPermission")
    fun dial(number: String): Boolean {
        val trimmed = number.trim()
        if (trimmed.isEmpty()) return false
        lastDialed = trimmed to System.currentTimeMillis()
        val uri = Uri.fromParts("tel", trimmed, null)
        return runCatching {
            if (granted(context, Manifest.permission.CALL_PHONE)) {
                val tm = context.getSystemService(TelecomManager::class.java)
                if (tm != null) {
                    try {
                        tm.placeCall(uri, android.os.Bundle.EMPTY)
                        return@runCatching true
                    } catch (_: SecurityException) {
                        // Fall back to the intents below if Telecom refused.
                    }
                }
                try {
                    context.startActivity(Intent(Intent.ACTION_CALL, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    return@runCatching true
                } catch (_: SecurityException) {
                    // Fall back to the dialer below if the OS blocked direct calling.
                }
            }
            context.startActivity(Intent(Intent.ACTION_DIAL, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        }.getOrElse {
            Log.w(TAG, "couldn't dial a call", it)
            false
        }
    }

    /** Telecom, when the app may answer and end calls. */
    private fun telecom(): TelecomManager? =
        context.getSystemService(TelecomManager::class.java)?.takeIf { granted(context, Manifest.permission.ANSWER_PHONE_CALLS) }

    @SuppressLint("MissingPermission")
    private fun hangUpWithTelecom(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return false
        @Suppress("DEPRECATION")
        return telecom()?.endCall() == true
    }

    private var mutedRinger = false

    @SuppressLint("MissingPermission")
    private fun silence(telecom: TelecomManager) {
        // Telecom's own silencing needs a permission only system apps get on
        // some versions; muting the ring stream for this call does the same.
        try {
            telecom.silenceRinger()
        } catch (_: SecurityException) {
            val audio = context.getSystemService(AudioManager::class.java)
            audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0)
            mutedRinger = true
        }
    }

    private fun restoreRinger() {
        if (!mutedRinger) return
        mutedRinger = false
        runCatching {
            context.getSystemService(AudioManager::class.java).adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0)
        }
    }

    companion object {
        private const val TAG = "PhoneCalls"

        @Volatile private var instance: PhoneCalls? = null

        /** The companion's view of the call changed (mute, speaker, hold). */
        internal fun controlsChanged() {
            instance?.resendActive()
        }
        private const val MAX_PHOTO_BYTES = 64 * 1024
        private const val MAX_LOG_PAGE = 100
        /** How long the call log takes to have the call that just ended. */
        private const val LOG_DELAY_MS = 1500L
        private const val LOG_SETTLE_MS = 500L

        /** Following calls, with numbers, contact names, answering, and calling. */
        val permissions: Array<String> = arrayOf(
            Manifest.permission.READ_PHONE_STATE,
            Manifest.permission.READ_CALL_LOG,
            Manifest.permission.ANSWER_PHONE_CALLS,
            Manifest.permission.CALL_PHONE,
            Manifest.permission.READ_CONTACTS,
        )

        private fun granted(context: Context, permission: String) =
            ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

        /** Whether the app can follow calls at all. */
        fun canFollow(context: Context) = granted(context, Manifest.permission.READ_PHONE_STATE)

        /** Whether it can answer and decline them (declining needs Android 9). */
        fun canControl(context: Context) =
            canFollow(context) && granted(context, Manifest.permission.ANSWER_PHONE_CALLS)

        /** Whether it can read the phone's call history (`call.log`). */
        fun canReadLog(context: Context) = granted(context, Manifest.permission.READ_CALL_LOG)

        /** Whether it can place calls from a PC (`call.dial`). */
        fun canDial(context: Context) =
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_TELEPHONY) &&
                (granted(context, Manifest.permission.CALL_PHONE) || canFollow(context))

        /** Whether everything the card asks for is granted. */
        fun hasAll(context: Context) = permissions.all { granted(context, it) }
    }
}
