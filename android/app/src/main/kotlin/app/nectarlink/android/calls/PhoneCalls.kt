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
import android.telecom.PhoneAccount
import android.telecom.PhoneAccountHandle
import android.telecom.TelecomManager
import android.telephony.SubscriptionManager
import android.telephony.TelephonyManager
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.core.Call
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallLogEntry
import app.nectarlink.core.CallPhase
import app.nectarlink.core.CallSim
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

    /** Contacts found for the call in progress, by number (only touched on [worker]). */
    private val contacts = HashMap<String, Pair<String?, ByteArray?>>()

    /** Number dialed from a PC, used when Android's OFFHOOK broadcast omits the number. */
    @Volatile
    private var lastDialed: Pair<String, Long>? = null

    private data class Tracked(
        val id: String,
        val incoming: Boolean,
        var number: String?,
        var name: String? = null,
        var answered: Boolean = false,
        /** Declined from a PC: not a missed call. */
        var declined: Boolean = false,
        /** When it was answered (Unix ms). */
        var since: Long? = null,
        /** Started from [CallCompanion] without a telephony broadcast (for example, READ_PHONE_STATE not granted). */
        var fromCompanion: Boolean = false,
    )

    fun start() {
        instance = this
        if (!canFollow(context)) {
            receiver?.let { runCatching { context.unregisterReceiver(it) } }
            receiver = null
        } else if (receiver == null) {
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
        if (!canReadLog(context)) {
            logObserver?.let { runCatching { context.contentResolver.unregisterContentObserver(it) } }
            logObserver = null
            main.removeCallbacks(notifyLog)
        } else if (logObserver == null) {
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
        // The IDLE broadcast that would unmute it no longer arrives.
        restoreRinger()
    }

    private fun stateChanged(state: String, number: String?) {
        val call = current
        when (state) {
            TelephonyManager.EXTRA_STATE_RINGING -> when {
                call == null -> {
                    val tracked = Tracked(
                        id = System.currentTimeMillis().toString(),
                        incoming = true,
                        number = number ?: CallCompanion.number(),
                        name = CallCompanion.name(),
                    )
                    current = tracked
                    report(tracked, CallPhase.RINGING)
                }
                // The same call again, now with its number.
                call.number == null && number != null && !call.answered -> {
                    call.fromCompanion = false
                    call.number = number
                    report(call, CallPhase.RINGING)
                }
                else -> call.fromCompanion = false
            }
            TelephonyManager.EXTRA_STATE_OFFHOOK -> when {
                call == null -> {
                    // Calling out.
                    val now = System.currentTimeMillis()
                    val dialed = lastDialed?.takeIf { now - it.second < 15_000L }?.first
                    // Android's broadcast never has the number of a call made on
                    // the phone: the one dialed from a PC, or Telecom's.
                    val outNumber = number ?: dialed ?: CallCompanion.number()
                    val tracked = Tracked(
                        id = now.toString(),
                        incoming = false,
                        number = outNumber,
                        name = CallCompanion.name(),
                        answered = true,
                        since = now,
                    )
                    current = tracked
                    report(tracked, CallPhase.ACTIVE)
                }
                !call.answered -> {
                    call.fromCompanion = false
                    if (call.number == null && number != null) {
                        call.number = number
                    }
                    call.answered = true
                    call.since = System.currentTimeMillis()
                    report(call, CallPhase.ACTIVE)
                }
                else -> call.fromCompanion = false
            }
            TelephonyManager.EXTRA_STATE_IDLE -> if (call != null) {
                current = null
                lastDialed = null
                restoreRinger()
                if (call.incoming && !call.answered && !call.declined) {
                    // Missed, or turned down on the phone: the call log knows,
                    // once it's written.
                    main.postDelayed({ report(call, CallPhase.ENDED, missedCheck = true) }, LOG_DELAY_MS)
                } else {
                    report(call, CallPhase.ENDED)
                }
            }
        }
    }

    private fun report(call: Tracked, phase: CallPhase, missedCheck: Boolean = false) {
        // A snapshot: the call may change while this waits its turn.
        val number = call.number
        val fallbackName = call.name
        val incoming = call.incoming
        val since = call.since
        worker.execute { send(call.id, phase, incoming, number, fallbackName, since, missedCheck) }
    }

    private fun send(
        id: String,
        phase: CallPhase,
        incoming: Boolean,
        number: String?,
        fallbackName: String?,
        since: Long?,
        missedCheck: Boolean,
    ) {
        // Every report carries the name (an answered call kept it before
        // only on the phone); looked up once per number while the call lasts.
        val (found, photo) = number?.let { contacts.getOrPut(it) { contact(it) } } ?: (null to null)
        val rawName = found ?: fallbackName ?: CallCompanion.name()
        val safeNumber = number?.let { truncateUtf8(it, MAX_NUMBER_BYTES) }?.takeIf { it.isNotEmpty() }
        val safeName = rawName?.let { truncateUtf8(it, MAX_TEXT_BYTES) }?.takeIf { it.isNotEmpty() }
        if (phase == CallPhase.ENDED) contacts.clear()
        val missed = missedCheck && wasMissed(id, number)
        onChange(
            Call(
                id = truncateUtf8(id, MAX_ID_BYTES),
                state = phase,
                incoming = incoming,
                number = safeNumber,
                name = safeName,
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
            current?.takeIf { it.answered }?.let { report(it, CallPhase.ACTIVE) }
        }
    }

    /** The companion observed a call state, number or caller display name change. */
    private fun onCompanionUpdate(state: Int?, number: String?, name: String?) {
        main.post {
            val call = current
            if (call != null) {
                if (call.fromCompanion && (state == null || state == android.telecom.Call.STATE_DISCONNECTED)) {
                    stateChanged(TelephonyManager.EXTRA_STATE_IDLE, null)
                    return@post
                }
                if (call.fromCompanion && !call.answered &&
                    (state == android.telecom.Call.STATE_ACTIVE || state == android.telecom.Call.STATE_HOLDING)
                ) {
                    if (call.number == null && number != null) call.number = number
                    if (name != null) call.name = name
                    call.answered = true
                    call.since = System.currentTimeMillis()
                    report(call, CallPhase.ACTIVE)
                    return@post
                }
                var updated = false
                if (call.number == null && number != null) {
                    call.number = number
                    updated = true
                }
                if (name != null && call.name != name) {
                    call.name = name
                    updated = true
                }
                if (updated) {
                    report(call, if (call.answered) CallPhase.ACTIVE else CallPhase.RINGING)
                }
            } else {
                when (state) {
                    android.telecom.Call.STATE_RINGING -> {
                        val tracked = Tracked(
                            id = System.currentTimeMillis().toString(),
                            incoming = true,
                            number = number,
                            name = name,
                            fromCompanion = true,
                        )
                        current = tracked
                        report(tracked, CallPhase.RINGING)
                    }
                    android.telecom.Call.STATE_ACTIVE, android.telecom.Call.STATE_HOLDING -> {
                        val now = System.currentTimeMillis()
                        val dialed = lastDialed?.takeIf { now - it.second < 15_000L }?.first
                        val tracked = Tracked(
                            id = now.toString(),
                            incoming = false,
                            number = number ?: dialed,
                            name = name,
                            answered = true,
                            since = now,
                            fromCompanion = true,
                        )
                        current = tracked
                        report(tracked, CallPhase.ACTIVE)
                    }
                }
            }
        }
    }

    /** The contact's name and photo for a number, when allowed and known. */
    private fun contact(number: String): Pair<String?, ByteArray?> {
        if (!granted(context, Manifest.permission.READ_CONTACTS)) return null to null
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
        if (!canReadLog(context)) return emptyList()
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
    fun dial(number: String): Boolean = dial(number, null)

    /**
     * The SIMs (and calling apps) this phone calls with (#112), so a PC can
     * pick one rather than leave Android asking on the phone. Each is named
     * by its slot and carrier where Android says ("SIM 1 · Airtel").
     */
    @SuppressLint("MissingPermission")
    fun sims(): List<CallSim> {
        if (!canListSims(context)) return emptyList()
        return runCatching {
            val tm = context.getSystemService(TelecomManager::class.java) ?: return emptyList()
            val default = tm.getDefaultOutgoingPhoneAccount(PhoneAccount.SCHEME_TEL)
            tm.callCapablePhoneAccounts.mapNotNull { handle ->
                val rawId = handle.id?.trim().orEmpty()
                if (rawId.isEmpty()) return@mapNotNull null
                val id = truncateUtf8(rawId, MAX_ID_BYTES)
                CallSim(id = id, label = simLabel(tm, handle), isDefault = handle == default)
            }
        }.getOrElse {
            Log.w(TAG, "can't list the SIMs", it)
            emptyList()
        }
    }

    /** Places a call to [number] on the SIM [sim] (an id from [sims]), or on the default when it's gone. */
    @SuppressLint("MissingPermission")
    fun dialOn(number: String, sim: String): Boolean {
        val handle = if (canListSims(context)) {
            runCatching {
                context.getSystemService(TelecomManager::class.java)?.callCapablePhoneAccounts?.firstOrNull {
                    val rawId = it.id?.trim().orEmpty()
                    rawId == sim || truncateUtf8(rawId, MAX_ID_BYTES) == sim
                }
            }.getOrNull()
        } else {
            null
        }
        return dial(number, handle)
    }

    /**
     * "SIM 1 · Orange": the slot, and the name the SIM has in the phone's
     * settings. That comes from the SIM's subscription, not its phone account:
     * reading an account needs Read phone numbers on newer Android (#334),
     * which the app doesn't ask for, so it's only a fallback for accounts
     * without a SIM, like a calling app's.
     */
    @SuppressLint("MissingPermission")
    private fun simLabel(tm: TelecomManager, handle: PhoneAccountHandle): String {
        val info = runCatching {
            val subId = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                context.getSystemService(TelephonyManager::class.java)?.getSubscriptionId(handle)
            } else {
                handle.id.toIntOrNull()
            } ?: return@runCatching null
            context.getSystemService(SubscriptionManager::class.java)?.getActiveSubscriptionInfo(subId)
        }.getOrNull()
        val slot = info?.simSlotIndex?.takeIf { it >= 0 }?.plus(1)
        val carrier = (info?.displayName ?: info?.carrierName)?.toString()?.trim()?.takeIf { it.isNotEmpty() }
            ?: runCatching { tm.getPhoneAccount(handle)?.label?.toString()?.trim() }.getOrNull().orEmpty()
        val label = when {
            slot != null && carrier.isNotEmpty() -> "SIM $slot · $carrier"
            slot != null -> "SIM $slot"
            carrier.isNotEmpty() -> carrier
            else -> handle.id?.trim().orEmpty()
        }.take(MAX_SIM_LABEL)
        return truncateUtf8(label, MAX_TEXT_BYTES)
    }

    @SuppressLint("MissingPermission")
    private fun dial(number: String, account: PhoneAccountHandle?): Boolean {
        val trimmed = number.trim()
        if (trimmed.isEmpty()) return false
        lastDialed = trimmed to System.currentTimeMillis()
        val uri = Uri.fromParts("tel", trimmed, null)
        // A code with * or # (call forwarding, USSD) runs the moment it's
        // dialled: it only goes into the dialer, for the phone's user to press.
        val code = trimmed.any { it == '*' || it == '#' }
        return runCatching {
            if (!code && granted(context, Manifest.permission.CALL_PHONE)) {
                val tm = context.getSystemService(TelecomManager::class.java)
                if (tm != null) {
                    try {
                        val extras = android.os.Bundle()
                        if (account != null) extras.putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, account)
                        tm.placeCall(uri, extras)
                        return@runCatching true
                    } catch (_: SecurityException) {
                        // Fall back to the intents below if Telecom refused.
                    }
                }
                try {
                    val call = Intent(Intent.ACTION_CALL, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    if (account != null) call.putExtra(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, account)
                    context.startActivity(call)
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

    /**
     * Whether the ring stream is muted to silence a call. Kept on disk: if the
     * app is stopped before the call ends, the next start unmutes it instead
     * of leaving the phone silent for every call after.
     */
    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var mutedRinger: Boolean
        get() = prefs.getBoolean(MUTED_RINGER, false)
        set(value) = prefs.edit { putBoolean(MUTED_RINGER, value) }

    init {
        restoreRinger()
    }

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
        private const val PREFS = "phone_calls"
        private const val MUTED_RINGER = "muted_ringer"
        private const val TAG = "PhoneCalls"
        /** The protocol's limit for a label (`calls::MAX_TEXT_BYTES`), in characters to be safe. */
        private const val MAX_SIM_LABEL = 64
        internal const val MAX_ID_BYTES = 128
        internal const val MAX_NUMBER_BYTES = 64
        internal const val MAX_TEXT_BYTES = 256

        @Volatile private var instance: PhoneCalls? = null

        /** The companion's view of the call changed (mute, speaker, hold). */
        internal fun controlsChanged() {
            instance?.resendActive()
        }

        /** The companion observed a call state, number, or caller display name change. */
        internal fun companionUpdate(state: Int?, number: String?, name: String?) {
            instance?.onCompanionUpdate(state, number, name)
        }

        /** Truncates [text] on a code-point boundary so its UTF-8 encoding is at most [maxBytes]. */
        internal fun truncateUtf8(text: String, maxBytes: Int): String {
            if (text.toByteArray(Charsets.UTF_8).size <= maxBytes) return text
            var used = 0
            var i = 0
            while (i < text.length) {
                val cp = text.codePointAt(i)
                val bytes = when {
                    cp <= 0x7F -> 1
                    cp <= 0x7FF -> 2
                    cp <= 0xFFFF -> 3
                    else -> 4
                }
                if (used + bytes > maxBytes) break
                used += bytes
                i += Character.charCount(cp)
            }
            return text.substring(0, i)
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

        /** Whether it can list its SIMs and call on the one a PC picks (`call.sims`). */
        fun canListSims(context: Context) =
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_TELEPHONY) &&
                granted(context, Manifest.permission.CALL_PHONE) &&
                granted(context, Manifest.permission.READ_PHONE_STATE)

        /** Whether everything the card asks for is granted. */
        fun hasAll(context: Context) = permissions.all { granted(context, it) }
    }
}
