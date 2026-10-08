// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationManagerCompat
import app.nectarlink.android.NectarlinkApplication

/**
 * Hands this phone's notifications to the core, which mirrors them to the
 * PCs the user allows (docs/protocol/notifications.md), and carries out
 * what a PC asks for: dismissing, replying, running actions.
 *
 * Runs only while the user has granted notification access; Android binds
 * it (and keeps the process alive) for as long as that lasts.
 */
class NotificationListener : NotificationListenerService() {
    private val core get() = (application as NectarlinkApplication).core
    private val reader by lazy { NotificationReader(this) }
    private val limiter = LiveRateLimiter { notification -> core.notificationPosted(notification) }

    /** Keys already sent, so an update that shouldn't alert again doesn't. */
    private val sent = mutableSetOf<String>()

    override fun onListenerConnected() {
        instance = this
        limiter.clear()
        sent.clear()
        val items = activeNotifications.orEmpty().mapNotNull { reader.read(it, currentRanking, update = false) }
        sent += items.map { it.key }
        core.notificationAccessChanged(true, items)
    }

    override fun onListenerDisconnected() {
        limiter.clear()
        if (instance === this) instance = null
        core.notificationAccessChanged(false, emptyList())
    }

    override fun onNotificationPosted(sbn: StatusBarNotification, rankingMap: RankingMap?) {
        val notification = reader.read(sbn, rankingMap, update = sbn.key in sent)
        if (notification == null) {
            // Something we mirrored may have turned into something we don't
            // (e.g. it became ongoing); take it away on the PC.
            limiter.cancel(sbn.key)
            if (sent.remove(sbn.key)) core.notificationRemoved(sbn.key)
            return
        }
        sent += sbn.key
        limiter.onPosted(notification)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        limiter.cancel(sbn.key)
        if (sent.remove(sbn.key)) core.notificationRemoved(sbn.key)
    }

    override fun onDestroy() {
        limiter.clear()
        if (instance === this) instance = null
        super.onDestroy()
    }

    /** What a PC asks for, on the listener that's connected now. */
    internal fun dismiss(key: String) = cancelNotification(key)

    internal fun find(key: String): StatusBarNotification? =
        runCatching { getActiveNotifications(arrayOf(key)) }.getOrNull()?.firstOrNull()

    companion object {
        /** The connected listener, if notification access is granted. */
        @Volatile
        internal var instance: NotificationListener? = null
            private set

        /** Whether the user granted notification access to this app. */
        fun hasAccess(context: Context): Boolean =
            NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)

        fun component(context: Context) = ComponentName(context, NotificationListener::class.java)

        /** Android's notification access screen, on Nectarlink's own switch where possible. */
        fun settingsIntent(context: Context): Intent =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
                    Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
                    component(context).flattenToString(),
                )
            } else {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
            }.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
