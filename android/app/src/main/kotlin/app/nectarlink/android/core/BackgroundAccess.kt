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
object BackgroundAccess {
    fun isUnrestricted(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) ?: true

    /** Android's prompt to let Nectarlink run unrestricted. */
    @SuppressLint("BatteryLife") // A companion that must stay reachable is the case this exists for.
    fun requestIntent(context: Context): Intent =
        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${context.packageName}".toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
}
