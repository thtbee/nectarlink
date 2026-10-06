// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.links

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.net.toUri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R

/**
 * Links a PC sent: Android doesn't let an app in the background open
 * screens, so each link waits in a notification the user taps to open.
 */
object LinkNotifications {
    private const val CHANNEL = "links"
    private const val TAG = "link"

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_links), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** Shows a link from a PC; false if notifications are off for the app. */
    fun show(context: Context, pcName: String, url: String): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        val uri = url.toUri()
        val open = PendingIntent.getActivity(
            context,
            url.hashCode(),
            Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        // "example.com/path", without the scheme, reads better.
        val shown = url.substringAfter("://").removeSuffix("/")
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.link_from, pcName.ifEmpty { context.getString(R.string.your_pc) }))
            .setContentText(shown)
            .setStyle(NotificationCompat.BigTextStyle().bigText(shown))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(TAG, url.hashCode(), notification) }.isSuccess
    }
}
