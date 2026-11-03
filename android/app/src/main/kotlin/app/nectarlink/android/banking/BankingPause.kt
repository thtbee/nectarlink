// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import android.Manifest
import android.app.AppOpsManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import android.os.Process
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.edit
import app.nectarlink.android.R
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mirror.InputService
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Pause for banking apps: many banking apps refuse to run while Developer
 * options, USB or Wireless debugging are on, or while an accessibility
 * service is enabled, all of which NectarLink's Elevated and Assist use.
 * While an app the user chose is in front, NectarLink turns those off, and
 * turns back on exactly what it turned off once the user leaves.
 *
 * Changing them needs WRITE_SECURE_SETTINGS (Elevated grants it once), and
 * knowing the app in front needs usage access. What was turned off is saved
 * before anything changes, so it comes back after a crash or a reboot too.
 */
object BankingPause {
    private const val TAG = "BankingPause"
    private const val PREFS = "banking_pause"
    private const val APPS = "apps"
    /** Saved while paused: what was on before, to turn back on. */
    private const val PAUSED = "paused"
    private const val WAS_DEVELOPER = "was_developer"
    private const val WAS_USB_DEBUGGING = "was_usb_debugging"
    private const val WAS_WIRELESS_DEBUGGING = "was_wireless_debugging"
    private const val WAS_ACCESSIBILITY = "was_accessibility"

    private const val DEVELOPER = Settings.Global.DEVELOPMENT_SETTINGS_ENABLED
    private const val USB_DEBUGGING = Settings.Global.ADB_ENABLED
    private const val WIRELESS_DEBUGGING = "adb_wifi_enabled"

    private const val CHANNEL = "banking_pause"
    private const val NOTIFICATION_ID = 41
    private const val ACTION_RESUME = "app.nectarlink.action.BANKING_RESUME"

    private val _paused = MutableStateFlow(false)
    /** Whether things are paused for a banking app now. */
    val paused: StateFlow<Boolean> = _paused.asStateFlow()

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Brings the in-memory state in line with what's saved (at process start). */
    fun init(context: Context) {
        _paused.value = prefs(context).getBoolean(PAUSED, false)
    }

    fun isPaused(context: Context): Boolean = prefs(context).getBoolean(PAUSED, false)

    /** The banking apps the user chose, by package. */
    fun apps(context: Context): Set<String> = prefs(context).getStringSet(APPS, emptySet()).orEmpty().toSet()

    fun setApps(context: Context, apps: Set<String>) {
        prefs(context).edit { putStringSet(APPS, apps.toSet()) }
        // Removing the app that's paused for leaves nothing to stay paused for.
        if (apps.isEmpty() && isPaused(context)) resume(context)
    }

    /** Whether NectarLink may change these settings (granted when Elevated first starts). */
    fun canChangeSettings(context: Context): Boolean =
        context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) == PackageManager.PERMISSION_GRANTED

    /** Whether NectarLink can see which app is in front (Settings › Usage access). */
    fun hasUsageAccess(context: Context): Boolean {
        val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        } else {
            @Suppress("DEPRECATION")
            ops.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        }
        return mode == AppOpsManager.MODE_ALLOWED
    }

    /** Ready to work: something chosen, and both permissions. */
    fun isReady(context: Context): Boolean =
        apps(context).isNotEmpty() && canChangeSettings(context) && hasUsageAccess(context)

    /**
     * Turns off Developer options, USB and Wireless debugging and NectarLink's
     * accessibility service, saving what was on first. Does nothing if
     * already paused or not allowed.
     */
    @Synchronized
    fun pause(context: Context) {
        if (isPaused(context) || !canChangeSettings(context)) return
        val resolver = context.contentResolver
        val accessibility = isAccessibilityEnabled(context)
        // Saved before anything changes, so a crash halfway still restores.
        prefs(context).edit(commit = true) {
            putBoolean(PAUSED, true)
            putBoolean(WAS_DEVELOPER, global(context, DEVELOPER))
            putBoolean(WAS_USB_DEBUGGING, global(context, USB_DEBUGGING))
            putBoolean(WAS_WIRELESS_DEBUGGING, global(context, WIRELESS_DEBUGGING))
            putBoolean(WAS_ACCESSIBILITY, accessibility)
        }
        _paused.value = true
        Elevated.pauseForBanking()
        app.nectarlink.android.mirror.MirrorService.stopCurrent()
        app.nectarlink.android.mirror.AppWindows.closeAllCurrent()
        runCatching {
            if (accessibility) setAccessibility(context, false)
            Settings.Global.putInt(resolver, WIRELESS_DEBUGGING, 0)
            Settings.Global.putInt(resolver, USB_DEBUGGING, 0)
            Settings.Global.putInt(resolver, DEVELOPER, 0)
        }.onFailure { Log.w(TAG, "couldn't pause everything", it) }
        Log.i(TAG, "paused for a banking app")
        showNotification(context)
    }

    /** Turns back on exactly what [pause] turned off. */
    @Synchronized
    fun resume(context: Context) {
        val saved = prefs(context)
        if (!saved.getBoolean(PAUSED, false)) return
        val resolver = context.contentResolver
        // Each one on its own, and remembered until it's back on: forgetting a
        // setting that couldn't be restored would leave it off for good.
        val restores: List<Pair<String, () -> Boolean>> = listOf(
            WAS_DEVELOPER to { Settings.Global.putInt(resolver, DEVELOPER, 1) },
            WAS_USB_DEBUGGING to { Settings.Global.putInt(resolver, USB_DEBUGGING, 1) },
            WAS_WIRELESS_DEBUGGING to { Settings.Global.putInt(resolver, WIRELESS_DEBUGGING, 1) },
            WAS_ACCESSIBILITY to {
                setAccessibility(context, true)
                true
            },
        )
        val notBack = restores.filter { (key, restore) ->
            val back = !saved.getBoolean(key, false) || runCatching { restore() }.getOrDefault(false)
            if (!back) Log.w(TAG, "couldn't turn $key back on")
            !back
        }.map { it.first }
        saved.edit(commit = true) {
            restores.map { it.first }.filterNot { it in notBack }.forEach { remove(it) }
        }
        if (notBack.isNotEmpty()) return
        saved.edit(commit = true) { remove(PAUSED) }
        _paused.value = false
        Log.i(TAG, "turned back on after a banking app")
        context.getSystemService(NotificationManager::class.java)?.cancel(NOTIFICATION_ID)
        // Elevated waited while paused; with debugging back it starts again.
        Elevated.onNetworkChanged()
    }

    /** At process start: a pause left over from before (a crash, a reboot) with nothing to wait for ends now. */
    fun restoreIfStranded(context: Context) {
        if (isPaused(context) && !isReady(context)) resume(context)
    }

    private fun global(context: Context, name: String): Boolean =
        runCatching { Settings.Global.getInt(context.contentResolver, name, 0) == 1 }.getOrDefault(false)

    private fun accessibilityComponent(context: Context): String =
        ComponentName(context, InputService::class.java).flattenToString()

    private fun isAccessibilityEnabled(context: Context): Boolean {
        val enabled = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
        return AccessibilityServices.contains(enabled, accessibilityComponent(context))
    }

    private fun setAccessibility(context: Context, on: Boolean) {
        val resolver = context.contentResolver
        val ours = accessibilityComponent(context)
        val others = AccessibilityServices.without(
            Settings.Secure.getString(resolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            ours,
        )
        val services = if (on) others + ours else others
        Settings.Secure.putString(resolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES, AccessibilityServices.join(services))
        Settings.Secure.putInt(resolver, Settings.Secure.ACCESSIBILITY_ENABLED, if (services.isEmpty()) 0 else 1)
    }

    // ---- The app in front ----

    /** Follows the app in front from usage events. */
    class Foreground(context: Context) {
        private val usage = context.getSystemService(UsageStatsManager::class.java)
        private val power = context.getSystemService(PowerManager::class.java)
        private val keyguard = context.getSystemService(android.app.KeyguardManager::class.java)
        /** The first read looks back an hour, to know what's in front already. */
        private var lastQueryMs = System.currentTimeMillis() - 3_600_000
        private var wasLocked = false
        /** The app last brought to the front, as far as the events say. */
        var current: String? = null
            private set

        /** Reads the events since the last call; returns the app in front (or [PauseDecider.LOCKED_PACKAGE] when locked or off). */
        fun poll(): String? {
            val now = System.currentTimeMillis()
            if (power?.isInteractive == false || keyguard?.isKeyguardLocked == true) {
                wasLocked = true
                current = null
                lastQueryMs = now
                return PauseDecider.LOCKED_PACKAGE
            }
            val fromMs = if (wasLocked) {
                wasLocked = false
                (now - 5_000).coerceAtMost(lastQueryMs)
            } else {
                lastQueryMs
            }
            val events = runCatching { usage?.queryEvents(fromMs, now) }.getOrNull()
            lastQueryMs = now
            if (events != null) {
                val event = UsageEvents.Event()
                while (events.hasNextEvent()) {
                    events.getNextEvent(event)
                    // ACTIVITY_RESUMED (Android 10+) has the same value.
                    @Suppress("DEPRECATION")
                    if (event.eventType == UsageEvents.Event.MOVE_TO_FOREGROUND) {
                        val pkg = event.packageName
                        if (current != null || pkg !in PauseDecider.NEUTRAL_PACKAGES) {
                            current = pkg
                        }
                    }
                }
            }
            return current ?: PauseDecider.LOCKED_PACKAGE
        }
    }

    // ---- The notification while paused ----

    private fun showNotification(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.banking_channel), NotificationManager.IMPORTANCE_LOW)
                .apply { setShowBadge(false) },
        )
        val resume = PendingIntent.getBroadcast(
            context, 0, Intent(context, ResumeReceiver::class.java).setAction(ACTION_RESUME),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.banking_paused_title))
            .setContentText(context.getString(R.string.banking_paused_text))
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .addAction(0, context.getString(R.string.banking_turn_back_on), resume)
            .build()
        runCatching { manager.notify(NOTIFICATION_ID, notification) }
    }

    /** The notification's Turn back on. */
    class ResumeReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == ACTION_RESUME) resume(context.applicationContext)
        }
    }
}
