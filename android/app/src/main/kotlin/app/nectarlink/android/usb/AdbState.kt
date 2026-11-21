// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.usb

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.provider.Settings
import androidx.core.content.ContextCompat

/**
 * Whether a computer could be reaching this phone through adb right now:
 * USB debugging on with a cable in, or wireless debugging on. A PC's USB
 * tunnel only ever arrives that way, while any app on the phone can reach
 * the tunnel's port, so the core takes pairing through the tunnel (without
 * pairing mode) only while this says yes. It never asks the user anything.
 */
internal object AdbState {
    private const val ADB_WIFI = "adb_wifi_enabled"
    private const val USB_STATE = "android.hardware.usb.action.USB_STATE"

    fun mayBeConnected(context: Context): Boolean {
        val resolver = context.contentResolver
        val usbDebugging = runCatching { Settings.Global.getInt(resolver, Settings.Global.ADB_ENABLED, 0) == 1 }
            .getOrDefault(true)
        val wirelessDebugging = runCatching { Settings.Global.getInt(resolver, ADB_WIFI, 0) == 1 }
            .getOrDefault(true)
        return decide(usbDebugging, wirelessDebugging, cableIn(context))
    }

    /**
     * USB debugging needs a cable in; wireless debugging doesn't. When the
     * cable's state is unknown ([cableIn] null) it counts as in, so nothing a
     * phone fails to report ever stops a real PC.
     */
    internal fun decide(usbDebugging: Boolean, wirelessDebugging: Boolean, cableIn: Boolean?): Boolean =
        wirelessDebugging || (usbDebugging && cableIn != false)

    /** Whether a USB cable to a computer is in, from the last USB state Android sent; null when it never said. */
    private fun cableIn(context: Context): Boolean? = runCatching {
        val state: Intent? = ContextCompat.registerReceiver(
            context,
            null,
            IntentFilter(USB_STATE),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        state?.takeIf { it.hasExtra("connected") }?.getBooleanExtra("connected", true)
    }.getOrNull()
}
