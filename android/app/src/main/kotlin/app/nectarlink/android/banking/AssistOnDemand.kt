// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import android.content.ComponentName
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import androidx.core.content.edit
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mirror.InputService
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Assist only while a PC uses the phone (#400). Some apps (banking ones, say)
 * crash or refuse to open while any accessibility service is on. With Elevated
 * set up, NectarLink needs its accessibility service (Assist) for two things
 * only, pressing Send in chat apps and typing characters outside ASCII, and
 * only while a PC shows the phone's screen, an app window, or types on it.
 *
 * With this on, the service is turned on when one of those starts and off a
 * little after the last one ends, through the same secure setting Pause for
 * banking apps uses. It's only ever turned back on if this turned it off, so
 * a user who switched Assist off themselves keeps it off. Without Elevated,
 * Assist is how a PC controls the phone at all, so it stays on.
 */
object AssistOnDemand {
    private const val TAG = "AssistOnDemand"
    private const val PREFS = "assist_on_demand"
    private const val ENABLED = "enabled"
    /** Saved while the service is off because of this, to turn it back on. */
    private const val OFF_BY_US = "off_by_us"

    /** How long the service stays on after the last session ends, so switching windows doesn't flap it. */
    private const val OFF_DELAY_MS = 15_000L

    private val main = Handler(Looper.getMainLooper())
    private var streaming = false
    private val typingPcs = HashSet<String>()
    private var appContext: Context? = null

    private val _enabled = MutableStateFlow(false)
    /** Whether the user turned this on. */
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()

    /** Whether an off is waiting on [OFF_DELAY_MS], so more calls don't keep pushing it back. */
    private var offPending = false
    private val turnOff = Runnable {
        synchronized(this) { offPending = false }
        appContext?.let { reconcile(it, now = true) }
    }

    private fun cancelOff() {
        main.removeCallbacks(turnOff)
        offPending = false
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** At process start: reads the setting and brings the service in line. */
    fun init(context: Context) {
        appContext = context.applicationContext
        _enabled.value = prefs(context).getBoolean(ENABLED, false)
        reconcile(context)
    }

    fun isEnabled(context: Context): Boolean = prefs(context).getBoolean(ENABLED, false)

    /** The user's switch. Turning it off gives the service back if this had turned it off. */
    @Synchronized
    fun setEnabled(context: Context, on: Boolean) {
        appContext = context.applicationContext
        prefs(context).edit(commit = true) { putBoolean(ENABLED, on) }
        _enabled.value = on
        if (!on) {
            cancelOff()
            giveBack(context)
        } else {
            reconcile(context)
        }
    }

    /** Whether the phone streams to a PC: its screen or app windows ([app.nectarlink.android.mirror.StreamPower]). */
    @Synchronized
    fun streaming(context: Context, on: Boolean) {
        if (streaming == on) return
        streaming = on
        reconcile(context)
    }

    /** Whether [pcId] types on this phone without mirroring it. */
    @Synchronized
    fun typing(context: Context, pcId: String, on: Boolean) {
        val changed = if (on) typingPcs.add(pcId) else typingPcs.remove(pcId)
        if (changed) reconcile(context)
    }

    /**
     * Turns the service on or off for what's going on now. Off waits
     * [OFF_DELAY_MS] unless [now]. Nothing changes while paused for a banking
     * app: Pause turns the service off itself and back to how it found it.
     */
    @Synchronized
    fun reconcile(context: Context, now: Boolean = false) {
        appContext = context.applicationContext
        if (!isEnabled(context) || !BankingPause.canChangeSettings(context)) return
        val action = AssistDecider.decide(
            paused = BankingPause.isPaused(context),
            inUse = streaming || typingPcs.isNotEmpty(),
            elevatedSetUp = Elevated.isSetUp(),
            on = isOn(context),
            offByUs = prefs(context).getBoolean(OFF_BY_US, false),
            now = now,
        )
        if (action != AssistDecider.Action.SCHEDULE_OFF) cancelOff()
        when (action) {
            AssistDecider.Action.NONE -> Unit
            AssistDecider.Action.TURN_ON -> giveBack(context)
            AssistDecider.Action.SCHEDULE_OFF -> if (!offPending) {
                offPending = true
                main.postDelayed(turnOff, OFF_DELAY_MS)
            }
            AssistDecider.Action.TURN_OFF -> runCatching {
                // Saved first, so the service comes back even if the app stops right after.
                prefs(context).edit(commit = true) { putBoolean(OFF_BY_US, true) }
                set(context, false)
                Log.i(TAG, "Assist off until a PC uses the phone")
            }.onFailure { Log.w(TAG, "couldn't turn Assist off", it) }
        }
    }

    /** Turns the service back on if this turned it off. */
    private fun giveBack(context: Context) {
        if (!prefs(context).getBoolean(OFF_BY_US, false)) return
        runCatching {
            set(context, true)
            prefs(context).edit(commit = true) { remove(OFF_BY_US) }
            Log.i(TAG, "Assist on for a PC")
        }.onFailure { Log.w(TAG, "couldn't turn Assist back on", it) }
    }

    private fun component(context: Context): String = ComponentName(context, InputService::class.java).flattenToString()

    private fun isOn(context: Context): Boolean = AccessibilityServices.contains(
        Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
        component(context),
    )

    private fun set(context: Context, on: Boolean) {
        val resolver = context.contentResolver
        val ours = component(context)
        val others = AccessibilityServices.without(
            Settings.Secure.getString(resolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            ours,
        )
        val services = if (on) others + ours else others
        Settings.Secure.putString(resolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES, AccessibilityServices.join(services))
        Settings.Secure.putInt(resolver, Settings.Secure.ACCESSIBILITY_ENABLED, if (services.isEmpty()) 0 else 1)
    }
}
