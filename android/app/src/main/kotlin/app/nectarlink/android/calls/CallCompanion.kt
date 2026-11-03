// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.app.AppOpsManager
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.telecom.Call
import android.telecom.CallAudioState
import android.telecom.InCallService
import android.telecom.VideoProfile
import app.nectarlink.core.CallControls

/**
 * NectarLink as a calling companion: Telecom binds this while there are
 * calls, without NectarLink being the phone app, so a PC can mute, switch to
 * the speaker, hold, and press keys on the call in progress. It shows
 * nothing on the phone. Telecom binds companions that hold
 * MANAGE_ONGOING_CALLS (Android 12+), an app-op Elevated's shell grants.
 */
class CallCompanion : InCallService() {
    /**
     * The call being controlled: the active one, else the held one, else the
     * latest. A second call waiting to be answered doesn't take over the one
     * in progress, so hanging up, holding and keys reach the call a PC shows.
     */
    private val call: Call?
        get() = calls.let { all -> all.getOrNull(controlledIndex(all.map { it.stateNow() })) }

    private val callback = object : Call.Callback() {
        override fun onStateChanged(call: Call, state: Int) = changed()
        override fun onDetailsChanged(call: Call, details: Call.Details) = changed()
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onCallAdded(call: Call) {
        call.registerCallback(callback)
        changed()
    }

    override fun onCallRemoved(call: Call) {
        call.unregisterCallback(callback)
        changed()
    }

    @Deprecated("Deprecated in Java")
    override fun onCallAudioStateChanged(audioState: CallAudioState?) = changed()

    /** Only a call in progress has controls; ringing and ending report themselves. */
    private fun changed() {
        val currentCall = call
        val state = currentCall?.stateNow()
        PhoneCalls.companionUpdate(state, number(), name())
        if (state == Call.STATE_ACTIVE || state == Call.STATE_HOLDING) PhoneCalls.controlsChanged()
    }

    /** The call's number, from its `tel:` handle (null when hidden). */
    private fun number(): String? =
        call?.details?.handle?.takeIf { it.scheme == "tel" }?.schemeSpecificPart?.takeIf { it.isNotBlank() }

    /**
     * The name Telecom found for the caller (Android 11+), else the one the
     * calling app gave (an app's own call, such as WhatsApp's, may have no
     * number to look up).
     */
    private fun name(): String? {
        val details = call?.details ?: return null
        val contact = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) details.contactDisplayName else null
        return contact?.takeIf { it.isNotBlank() } ?: details.callerDisplayName?.takeIf { it.isNotBlank() }
    }

    private fun controls(): CallControls? {
        val call = call ?: return null
        @Suppress("DEPRECATION")
        val audio = callAudioState ?: return null
        return CallControls(
            muted = audio.isMuted,
            speaker = audio.route == CallAudioState.ROUTE_SPEAKER,
            held = call.stateNow() == Call.STATE_HOLDING,
            canHold = call.details.can(Call.Details.CAPABILITY_HOLD),
        )
    }

    private fun answer(): Boolean {
        val call = call?.takeIf { it.stateNow() == Call.STATE_RINGING } ?: return false
        call.answer(VideoProfile.STATE_AUDIO_ONLY)
        return true
    }

    private fun hangUp(): Boolean {
        val call = call ?: return false
        if (call.stateNow() == Call.STATE_RINGING) call.reject(false, null) else call.disconnect()
        return true
    }

    @Suppress("DEPRECATION") // setAudioRoute: still how a companion picks the speaker.
    private fun speaker(on: Boolean) {
        setAudioRoute(if (on) CallAudioState.ROUTE_SPEAKER else CallAudioState.ROUTE_WIRED_OR_EARPIECE)
    }

    private fun hold(on: Boolean): Boolean {
        val call = call ?: return false
        if (on) call.hold() else call.unhold()
        return true
    }

    private val main = Handler(Looper.getMainLooper())

    private fun dtmf(digit: Char): Boolean {
        val call = call ?: return false
        call.playDtmfTone(digit)
        // Stopped at once, the tone can be too short for the other end to hear.
        main.postDelayed({ call.stopDtmfTone() }, DTMF_MS)
        return true
    }

    companion object {
        /** How long a keypad tone plays, like a short press on the phone's own keypad. */
        private const val DTMF_MS = 150L
        @Volatile private var instance: CallCompanion? = null

        /** Whether Telecom has bound it (there's a call, and the phone allows companions). */
        val bound: Boolean get() = instance != null

        /** Whether Telecom will bind it: Android 12+, with the app-op allowed. */
        fun allowed(context: Context): Boolean {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return false
            val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
            return runCatching {
                // Deprecated on 37 for attribution-aware checks; this one is only the app's own.
                @Suppress("DEPRECATION")
                ops.unsafeCheckOpNoThrow(OP, Process.myUid(), context.packageName) == AppOpsManager.MODE_ALLOWED
            }.getOrDefault(false)
        }

        /** The app-op's name for `appops set`. */
        const val OP_NAME = "MANAGE_ONGOING_CALLS"
        private const val OP = "android:manage_ongoing_calls"

        /** The call in progress' mute, speaker and hold, when bound. */
        fun controls(): CallControls? = instance?.controls()

        /** The call's number as Telecom has it, when bound. */
        fun number(): String? = instance?.number()

        /** The caller's name as Telecom has it, when bound. */
        fun name(): String? = instance?.name()

        fun answer(): Boolean = instance?.answer() ?: false

        fun hangUp(): Boolean = instance?.hangUp() ?: false

        fun mute(on: Boolean): Boolean = instance?.run { setMuted(on); true } ?: false

        fun speaker(on: Boolean): Boolean = instance?.run { speaker(on); true } ?: false

        fun hold(on: Boolean): Boolean = instance?.hold(on) ?: false

        fun dtmf(digit: Char): Boolean = instance?.dtmf(digit) ?: false
    }
}

/**
 * Which of Telecom's calls, by their states in Telecom's order, a PC
 * controls: the last active one, else the last held one, else the last not
 * yet disconnected; -1 when there's none.
 */
internal fun controlledIndex(states: List<Int>): Int =
    states.indexOfLast { it == Call.STATE_ACTIVE }.takeIf { it >= 0 }
        ?: states.indexOfLast { it == Call.STATE_HOLDING }.takeIf { it >= 0 }
        ?: states.indexOfLast { it != Call.STATE_DISCONNECTED }

/** The call's state: `Details.getState` from Android 12, `getState` before. */
private fun Call.stateNow(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) details.state else @Suppress("DEPRECATION") state
