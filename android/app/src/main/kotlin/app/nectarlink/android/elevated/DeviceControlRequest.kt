// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R

/**
 * Another paired phone asked to use this device as its touchpad and keyboard
 * target and isn't let in yet (#651): a notification that asks, with Allow
 * turning it on for that device (Settings › Devices turns it off again).
 */
object DeviceControlRequest {
    private const val CHANNEL = "device_control"
    private const val ACTION_ALLOW = "app.nectarlink.android.CONTROL_ALLOW"
    private const val ACTION_DISMISS = "app.nectarlink.android.CONTROL_DISMISS"
    private const val EXTRA_DEVICE = "device"

    fun show(context: Context, deviceId: String, deviceName: String) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_device_control), NotificationManager.IMPORTANCE_HIGH),
        )
        val id = notificationId(deviceId)
        fun action(name: String) = PendingIntent.getBroadcast(
            context,
            id * 2 + if (name == ACTION_ALLOW) 0 else 1,
            Intent(context, Receiver::class.java).setAction(name).putExtra(EXTRA_DEVICE, deviceId),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.device_control_request_title, deviceName))
            .setContentText(context.getString(R.string.device_control_request_text))
            .setStyle(NotificationCompat.BigTextStyle().bigText(context.getString(R.string.device_control_request_text)))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            .setAutoCancel(true)
            .addAction(0, context.getString(R.string.action_allow), action(ACTION_ALLOW))
            .addAction(0, context.getString(R.string.action_not_now), action(ACTION_DISMISS))
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(id, notification) }
    }

    fun dismiss(context: Context, deviceId: String) =
        NotificationManagerCompat.from(context).cancel(notificationId(deviceId))

    private fun notificationId(deviceId: String) = 0x4E43_0000 or (deviceId.hashCode() and 0xFFFF)

    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val deviceId = intent.getStringExtra(EXTRA_DEVICE) ?: return
            when (intent.action) {
                ACTION_ALLOW -> (context.applicationContext as NectarlinkApplication).core.setControlAllowed(deviceId, true)
                else -> dismiss(context, deviceId)
            }
        }
    }
}
