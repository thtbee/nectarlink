// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.Manifest
import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
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
import app.nectarlink.core.Call
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallPhase
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

/**
 * The phone's calls, for PCs (docs/protocol/calls.md): follows the call
 * state while the app runs, and answers, declines or silences a ringing
 * call when a PC asks. The call's audio stays on the phone.
 */
internal class PhoneCalls(context: Context, private val onChange: (Call) -> Unit) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    /** Contact lookups and reports, in order, off the main thread. */
    private val worker = Executors.newSingleThreadExecutor()
    private var receiver: BroadcastReceiver? = null

    /** The call in progress (changed on the main thread, read by PCs' requests). */
    @Volatile
    private var current: Tracked? = null

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
        if (receiver != null || !canFollow(context)) return
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

    fun stop() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
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
                    val tracked = Tracked(id = now.toString(), incoming = false, number = number, answered = true, since = now)
                    current = tracked
                    report(tracked, CallPhase.ACTIVE, withContact = false)
                }
                !call.answered -> {
                    call.answered = true
                    call.since = System.currentTimeMillis()
                    report(call, CallPhase.ACTIVE, withContact = false)
                }
            }
            TelephonyManager.EXTRA_STATE_IDLE -> if (call != null) {
                current = null
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
        val missed = missedCheck && wasMissed()
        onChange(
            Call(
                id = id,
                state = phase,
                incoming = incoming,
                number = number,
                name = name,
                photo = photo.takeIf { phase == CallPhase.RINGING },
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

    /** Whether the latest call in the log is a missed one (not turned down or blocked). */
    @SuppressLint("MissingPermission")
    private fun wasMissed(): Boolean {
        if (!granted(context, Manifest.permission.READ_CALL_LOG)) return true
        return runCatching {
            context.contentResolver.query(
                CallLog.Calls.CONTENT_URI.buildUpon().appendQueryParameter(CallLog.Calls.LIMIT_PARAM_KEY, "1").build(),
                arrayOf(CallLog.Calls.TYPE),
                null,
                null,
                "${CallLog.Calls.DATE} DESC",
            )?.use { if (it.moveToFirst()) it.getInt(0) == CallLog.Calls.MISSED_TYPE else true } ?: true
        }.getOrDefault(true)
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
        /** How long the call log takes to have the call that just ended. */
        private const val LOG_DELAY_MS = 1500L

        /** Following calls, with numbers, contact names, and answering. */
        val permissions: Array<String> = arrayOf(
            Manifest.permission.READ_PHONE_STATE,
            Manifest.permission.READ_CALL_LOG,
            Manifest.permission.ANSWER_PHONE_CALLS,
            Manifest.permission.READ_CONTACTS,
        )

        private fun granted(context: Context, permission: String) =
            ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

        /** Whether the app can follow calls at all. */
        fun canFollow(context: Context) = granted(context, Manifest.permission.READ_PHONE_STATE)

        /** Whether it can answer and decline them (declining needs Android 9). */
        fun canControl(context: Context) =
            canFollow(context) && granted(context, Manifest.permission.ANSWER_PHONE_CALLS)

        /** Whether everything the card asks for is granted. */
        fun hasAll(context: Context) = permissions.all { granted(context, it) }
    }
}
