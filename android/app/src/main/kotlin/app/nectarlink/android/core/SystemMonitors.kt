// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.provider.Settings
import androidx.core.content.ContextCompat
import app.nectarlink.core.Battery
import app.nectarlink.core.DeviceInfo
import app.nectarlink.core.DeviceKind

/** How this phone presents itself to PCs. */
fun deviceInfo(context: Context): DeviceInfo {
    val name = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
        ?.takeIf { it.isNotBlank() } ?: Build.MODEL
    val tablet = context.resources.configuration.smallestScreenWidthDp >= 600
    return DeviceInfo(
        name = name,
        kind = if (tablet) DeviceKind.TABLET else DeviceKind.PHONE,
        os = "android",
        osVersion = Build.VERSION.RELEASE,
        model = "${Build.MANUFACTURER.replaceFirstChar(Char::uppercase)} ${Build.MODEL}",
        accent = null,
    )
}

/** Reports the battery whenever it changes meaningfully. */
class BatteryMonitor(private val context: Context, private val onChange: (Battery) -> Unit) {
    private var last: Battery? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            if (level < 0 || scale <= 0) return
            val plugged = when (intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)) {
                BatteryManager.BATTERY_PLUGGED_AC -> "ac"
                BatteryManager.BATTERY_PLUGGED_USB -> "usb"
                BatteryManager.BATTERY_PLUGGED_WIRELESS -> "wireless"
                else -> null
            }
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            val battery = Battery((level * 100 / scale).coerceIn(0, 100).toUByte(), charging, plugged)
            if (battery != last) {
                last = battery
                onChange(battery)
            }
        }
    }

    fun start() {
        // ACTION_BATTERY_CHANGED is sticky: the current state arrives at once.
        ContextCompat.registerReceiver(
            context, receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    fun stop() {
        runCatching { context.unregisterReceiver(receiver) }
    }
}

/**
 * Tells the core when connectivity changes. Android doesn't let native code
 * watch the network, so iroh needs this to re-check paths quickly.
 */
class NetworkMonitor(context: Context, private val onChange: () -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = onChange()
        override fun onLost(network: Network) = onChange()
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) = Unit
    }

    fun start() {
        connectivity?.registerDefaultNetworkCallback(callback)
    }

    fun stop() {
        runCatching { connectivity?.unregisterNetworkCallback(callback) }
    }
}
