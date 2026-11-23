// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.ui.Preferences
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * A PC plugged in over USB starts the app here, through adb (the shell holds the
 * DUMP permission this receiver asks for), when nothing answers on the tunnel's port.
 * With USB only on, the app stops while no PC is on the cable and this brings it back.
 * It starts the core, whose listener the PC then reaches; the foreground service, which
 * Android may not let a broadcast start, is the PC's next request (usb.rs).
 */
class UsbWakeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ConnectionService.ACTION_USB_WAKE) return
        // Not for a phone set to run only while it's open.
        if (!Preferences.isUsbOnlyEnabled(context) && !Preferences.isBackgroundServiceEnabled(context)) return
        val app = context.applicationContext as NectarlinkApplication
        val pending = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                app.core.start()
                withTimeoutOrNull(START_WAIT_MS) { app.core.state.first { it.status !is CoreStatus.Starting } }
            } finally {
                pending.finish()
            }
        }
    }

    private companion object {
        const val START_WAIT_MS = 8_000L
    }
}
