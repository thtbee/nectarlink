// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.Context
import android.net.wifi.WifiManager
import android.os.PowerManager
import app.nectarlink.android.elevated.Elevated

/**
 * The phone's power while it streams to a PC: its screen ([SCREEN]) or apps
 * in windows of their own ([APPS]). Each says what it wants with [set], and
 * the phone does the most any of them asks for, so neither undoes the other.
 *
 * - While anything streams, a CPU wake lock and a Wi-Fi lock keep the phone
 *   on the network with its screen off. Without them it dozes, misses the
 *   PC's keep-alives, and the PC drops the connection after 8 s; the stream
 *   then restarts every few seconds.
 * - Stay awake keeps the screen on; screen off keeps it on too, with the
 *   panel turned off through Elevated, so apps on the PC keep running.
 */
object StreamPower {
    const val SCREEN = "screen"
    const val APPS = "apps"

    private data class Want(val stayAwake: Boolean, val screenOff: Boolean)

    private val wants = HashMap<String, Want>()
    private var cpu: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null
    private var screen: PowerManager.WakeLock? = null
    /** What Elevated was last told; the panel is on until something turns it off. */
    private var panelOff = false

    /** [source] streams ([streaming]), and wants the phone kept awake or its screen off. */
    @Synchronized
    fun set(context: Context, source: String, streaming: Boolean, stayAwake: Boolean = false, screenOff: Boolean = false) {
        if (streaming) wants[source] = Want(stayAwake, screenOff) else wants.remove(source)
        apply(context.applicationContext)
    }

    @Suppress("DEPRECATION")
    private fun apply(context: Context) {
        val streaming = wants.isNotEmpty()
        app.nectarlink.android.banking.AssistOnDemand.streaming(context, streaming)
        val off = wants.values.any { it.screenOff }
        val awake = off || wants.values.any { it.stayAwake }
        val power = context.getSystemService(PowerManager::class.java)
        val wifiManager = context.getSystemService(WifiManager::class.java)

        cpu = hold(cpu, streaming) { power?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "nectarlink:stream") }
        // HIGH_PERF: Wi-Fi out of power save with the screen off (newer
        // Android treats it as low latency, which only helps with it on).
        wifi = if (streaming) {
            (wifi ?: wifiManager?.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "nectarlink:stream"))
                ?.apply {
                    setReferenceCounted(false)
                    if (!isHeld) runCatching { acquire() }
                }
        } else {
            wifi?.let { if (it.isHeld) runCatching { it.release() } }
            null
        }
        screen = hold(screen, awake) {
            power?.newWakeLock(
                PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP,
                "nectarlink:mirror",
            )
        }
        if (panelOff != off) {
            panelOff = off
            Elevated.setScreenOff(off)
        }
    }

    /** Holds [lock] (made with [make] the first time) while [on], releases it otherwise. */
    private fun hold(lock: PowerManager.WakeLock?, on: Boolean, make: () -> PowerManager.WakeLock?): PowerManager.WakeLock? {
        if (!on) {
            lock?.let { if (it.isHeld) runCatching { it.release() } }
            return null
        }
        val held = (lock ?: make()?.apply { setReferenceCounted(false) }) ?: return null
        // Renewed on every change; a stream left open longer lets go by itself.
        runCatching { held.acquire(HOLD_MAX_MS) }
        return held
    }

    private const val HOLD_MAX_MS = 4 * 60 * 60 * 1000L
}
