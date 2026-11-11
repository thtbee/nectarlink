// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.edit
import app.nectarlink.android.R

/**
 * Asks once for a star on GitHub, with a notification: the first time the app
 * opens after an update, or an hour after a fresh install. Tapping it opens the
 * repo and swiping it away ends it; it's never posted twice on this install.
 */
internal object StarNotification {
    private const val CHANNEL = "project"
    private const val ID = 0x5A7A
    private const val HOUR_MS = 60 * 60 * 1000L
    private const val REPO = "https://github.com/thtbee/nectarlink"

    /** Where the Support buttons go: the project's tip page. */
    const val SUPPORT = "https://buymeacoffee.com/thtbee_"

    fun supportIntent(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        1,
        Intent(Intent.ACTION_VIEW, Uri.parse(SUPPORT)),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    // Its own file: "appearance" is backed up, and a reinstall should ask again.
    private const val PREFS = "star_notification"
    private const val POSTED = "posted"

    /** When to ask: right away for someone who updated, else an hour after installing. */
    fun askAt(firstInstall: Long, lastUpdate: Long, now: Long): Long =
        if (lastUpdate > firstInstall) now else firstInstall + HOUR_MS

    /** Call when the app opens. Posts it if it's time, or sets an alarm for when it is. */
    fun onAppOpened(context: Context) {
        val app = context.applicationContext
        if (posted(app)) return
        val info = runCatching { app.packageManager.getPackageInfo(app.packageName, 0) }.getOrNull() ?: return
        val now = System.currentTimeMillis()
        val at = askAt(info.firstInstallTime, info.lastUpdateTime, now)
        if (at <= now) {
            post(app)
        } else {
            app.getSystemService(AlarmManager::class.java)?.set(AlarmManager.RTC, at, alarm(app))
        }
    }

    private fun posted(context: Context) =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(POSTED, false)

    private fun alarm(context: Context) = PendingIntent.getBroadcast(
        context,
        0,
        Intent(context, Receiver::class.java),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun post(context: Context) {
        if (posted(context)) return
        val manager = NotificationManagerCompat.from(context)
        // Without permission it wouldn't be seen, so the next open tries again.
        if (!manager.areNotificationsEnabled()) return
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(
                CHANNEL,
                context.getString(R.string.star_channel),
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(Intent.ACTION_VIEW, Uri.parse(REPO)),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.star_title))
            .setContentText(context.getString(R.string.star_text))
            .setStyle(NotificationCompat.BigTextStyle().bigText(context.getString(R.string.star_text)))
            .setContentIntent(open)
            .addAction(0, context.getString(R.string.support_action), supportIntent(context))
            .setAutoCancel(true)
            .build()
        if (context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED &&
            android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU
        ) {
            return
        }
        manager.notify(ID, notification)
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(POSTED, true) }
    }

    /** The hour after installing is up. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            post(context.applicationContext)
        }
    }
}
