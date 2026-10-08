// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.ui.MainActivity
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps this phone reachable by its paired PCs while the app is in the
 * background: a foreground service (type "connected device") with an
 * ongoing notification, plus the Wi-Fi multicast lock that local discovery
 * needs on Android.
 */
class ConnectionService : LifecycleService() {
    private var multicastLock: WifiManager.MulticastLock? = null

    override fun onCreate() {
        super.onCreate()
        createChannels(this)
        app.nectarlink.android.files.TransferNotifications.createChannels(this)
        app.nectarlink.android.media.PcMedia.createChannel(this)
        app.nectarlink.android.links.LinkNotifications.createChannel(this)
        app.nectarlink.android.webcam.WebcamService.createChannel(this)
        app.nectarlink.android.webcam.WebcamRequests.createChannel(this)
        val core = (application as NectarlinkApplication).core
        if (!startInForeground(core.state.value)) {
            stopSelf()
            return
        }
        multicastLock = getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-discovery")
            ?.apply { setReferenceCounted(false); acquire() }

        lifecycleScope.launch {
            core.state.map { Status.of(it) to (it.status is app.nectarlink.android.core.CoreStatus.Ready) }
                .distinctUntilChanged()
                .collect { (status, ready) ->
                    if (ready && status.paired == 0) {
                        stopSelf()
                    } else {
                        getSystemService(NotificationManager::class.java)
                            .notify(ONGOING_ID, ongoingNotification(status))
                    }
                }
        }
        lifecycleScope.launch {
            core.state.map { it.ringingFrom }.distinctUntilChanged().collect { from ->
                val manager = getSystemService(NotificationManager::class.java)
                if (from != null) manager.notify(RINGING_ID, ringingNotification(from)) else manager.cancel(RINGING_ID)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_STOP_RINGING) {
            (application as NectarlinkApplication).core.stopRinging()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        multicastLock?.runCatching { release() }
        super.onDestroy()
    }

    private fun startInForeground(state: CoreState): Boolean = runCatching {
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        } else {
            0
        }
        ServiceCompat.startForeground(this, ONGOING_ID, ongoingNotification(Status.of(state)), type)
    }.onFailure {
        android.util.Log.w("ConnectionService", "can't enter foreground", it)
    }.isSuccess

    /** What the ongoing notification says. */
    private data class Status(val connected: List<String>, val paired: Int) {
        companion object {
            fun of(state: CoreState) = Status(state.devices.filter { it.online }.map { it.name }, state.devices.size)
        }
    }

    private fun ongoingNotification(status: Status): Notification {
        val text = when {
            status.connected.size == 1 -> getString(R.string.notification_connected_to, status.connected[0])
            status.connected.size > 1 -> getString(R.string.notification_connected_many, status.connected.size)
            status.paired == 0 -> getString(R.string.notification_not_paired)
            else -> getString(R.string.notification_waiting)
        }
        val builder = NotificationCompat.Builder(this, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(text)
            .setContentIntent(openApp())
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        if (status.connected.isNotEmpty()) {
            val send = PendingIntent.getActivity(
                this, 2, SendActivity.sendClipboardIntent(this), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(0, getString(R.string.action_send_clipboard), send)
        }
        return builder.build()
    }

    private fun ringingNotification(from: String): Notification {
        val stop = PendingIntent.getService(
            this, 1, Intent(this, ConnectionService::class.java).setAction(ACTION_STOP_RINGING),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val title = if (from.isEmpty()) getString(R.string.ringing_title_unknown) else getString(R.string.ringing_title, from)
        return NotificationCompat.Builder(this, CHANNEL_RINGING)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentIntent(openApp())
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setOngoing(true)
            .addAction(0, getString(R.string.action_stop_ringing), stop)
            .setDeleteIntent(stop)
            .build()
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
    )

    companion object {
        private const val ONGOING_ID = 1
        private const val RINGING_ID = 2
        private const val CHANNEL_CONNECTION = "connection"
        private const val CHANNEL_RINGING = "ringing"
        private const val ACTION_STOP_RINGING = "app.nectarlink.action.STOP_RINGING"

        /** Starts the service when allowed by Android's foreground-service rules. */
        fun start(context: Context) {
            runCatching {
                ContextCompat.startForegroundService(context, Intent(context, ConnectionService::class.java))
            }.onFailure {
                android.util.Log.w("ConnectionService", "can't start in background", it)
            }
        }

        fun stop(context: Context) {
            runCatching {
                context.stopService(Intent(context, ConnectionService::class.java))
            }
        }

        private fun createChannels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_CONNECTION, context.getString(R.string.channel_connection), NotificationManager.IMPORTANCE_LOW,
                ).apply { setShowBadge(false) },
            )
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_RINGING, context.getString(R.string.channel_ringing), NotificationManager.IMPORTANCE_HIGH,
                ).apply { setSound(null, null) },
            )
        }
    }
}
