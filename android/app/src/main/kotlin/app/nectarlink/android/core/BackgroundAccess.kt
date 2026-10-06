// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.os.PowerManager
import android.provider.Settings
import androidx.core.net.toUri

/**
 * Running unrestricted in the background. Without it, battery savers (and
 * Samsung's app freezer in particular) pause Nectarlink between events, so
 * a PC's requests (replies, dismissals, find my phone) time out.
 */
/**
 * The local network: Android 17 lets an app reach devices on it (the PC)
 * only with the user's permission.
 */
object LocalNetwork {
    /** Android 17's permission (API 37). */
    const val PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

    fun needed(): Boolean = android.os.Build.VERSION.SDK_INT >= 37

    fun granted(context: android.content.Context): Boolean =
        !needed() || context.checkSelfPermission(PERMISSION) == android.content.pm.PackageManager.PERMISSION_GRANTED
}

object BackgroundAccess {
    fun isUnrestricted(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) ?: true

    /** Android's prompt to let Nectarlink run unrestricted. */
    @SuppressLint("BatteryLife") // A companion that must stay reachable is the case this exists for.
    fun requestIntent(context: Context): Intent =
        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${context.packageName}".toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
}
