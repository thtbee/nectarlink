// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.app.AppOpsManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Process
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
    private val worker = java.util.concurrent.Executors.newSingleThreadExecutor { r ->
        Thread(r, "nectarlink-notif-reader").apply { isDaemon = true }
    }

    /** Keys already sent, so an update that shouldn't alert again doesn't. */
    private val sent = java.util.Collections.synchronizedSet(mutableSetOf<String>())

    override fun onListenerConnected() {
        instance = this
        if (isPcActive()) applyHostHints(true)
        work {
            val snapshot = runCatching { activeNotifications?.toList().orEmpty() }.getOrDefault(emptyList())
            val ranking = runCatching { currentRanking }.getOrNull()
            limiter.clear()
            sent.clear()
            val items = snapshot.mapNotNull { reader.read(it, ranking, update = false) }
            sent.addAll(items.map { it.key })
            core.notificationAccessChanged(true, items)
        }
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
        work {
            limiter.clear()
            sent.clear()
            core.notificationAccessChanged(false, emptyList())
        }
    }

    override fun onNotificationPosted(sbn: StatusBarNotification, rankingMap: RankingMap?) {
        val key = sbn.key
        work {
            val notification = reader.read(sbn, rankingMap, update = key in sent)
            if (notification == null) {
                // Something we mirrored may have turned into something we don't
                // (e.g. it became ongoing); take it away on the PC.
                limiter.cancel(key)
                if (sent.remove(key)) core.notificationRemoved(key)
                return@work
            }
            sent.add(key)
            limiter.onPosted(notification)
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        val key = sbn.key
        work {
            limiter.cancel(key)
            if (sent.remove(key)) core.notificationRemoved(key)
        }
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        // Android's onDestroy calls onListenerDisconnected, which still needs
        // the worker; shut it down after (queued work still runs).
        super.onDestroy()
        worker.shutdown()
    }

    /**
     * Runs [task] on the worker. Android can still deliver a callback after
     * onDestroy (one queued just before it); that one is dropped, since the
     * disconnect already told the core notification access is gone.
     */
    private fun work(task: () -> Unit) {
        try {
            worker.execute(task)
        } catch (_: java.util.concurrent.RejectedExecutionException) {
        }
    }

    internal fun applyHostHints(pcActive: Boolean) {
        val quiet = pcActive && app.nectarlink.android.ui.Preferences.isQuietWhileUsingPc(this)
        runCatching {
            requestListenerHints(if (quiet) HINT_HOST_DISABLE_NOTIFICATION_EFFECTS else 0)
        }
    }

    /** Re-reads active notifications after Elevated grants sensitive notification access. */
    internal fun refreshActive() {
        work {
            val items = runCatching {
                val ranking = currentRanking
                activeNotifications.orEmpty().mapNotNull { reader.read(it, ranking, update = true) }
            }.getOrDefault(emptyList())
            sent.addAll(items.map { it.key })
            core.notificationsRefreshed(items)
        }
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

        @Volatile
        private var pcActiveUntilMs: Long = 0L
        private val mainHandler = android.os.Handler(android.os.Looper.getMainLooper())
        private val expirePcActive = Runnable {
            if (!isPcActive()) {
                instance?.applyHostHints(false)
            }
        }

        fun isPcActive(): Boolean =
            android.os.SystemClock.elapsedRealtime() < pcActiveUntilMs

        /**
         * Updates whether the user is actively using a paired PC (`pc_active`
         * toggle). When active and the user turned on "Quiet while you use the
         * PC", Android's `HINT_HOST_DISABLE_NOTIFICATION_EFFECTS` silences the
         * phone's own notification sound and vibration; notifications still
         * reach the PC either way.
         */
        fun setPcActive(active: Boolean) {
            mainHandler.removeCallbacks(expirePcActive)
            if (active) {
                pcActiveUntilMs = android.os.SystemClock.elapsedRealtime() + 65_000L
                instance?.applyHostHints(true)
                mainHandler.postDelayed(expirePcActive, 66_000L)
            } else {
                pcActiveUntilMs = 0L
                instance?.applyHostHints(false)
            }
        }

        /** Applies the quiet setting again after it changed. */
        fun refreshHostHints() {
            instance?.applyHostHints(isPcActive())
        }

        /** Whether the user granted notification access to this app. */
        fun hasAccess(context: Context): Boolean =
            NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)

        /**
         * Whether this listener may read notifications Android 15+ marks
         * sensitive (such as one-time verification codes). Before Android 15
         * listeners aren't redacted by this app-op.
         */
        fun canReadSensitive(context: Context): Boolean {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return true
            val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
            return runCatching {
                // Deprecated on 37 for attribution-aware checks; this one is only the app's own.
                @Suppress("DEPRECATION")
                ops.unsafeCheckOpNoThrow(OP_SENSITIVE, Process.myUid(), context.packageName) == AppOpsManager.MODE_ALLOWED
            }.getOrDefault(false)
        }

        /** The app-op's name for `appops set`. */
        const val OP_SENSITIVE_NAME = "RECEIVE_SENSITIVE_NOTIFICATIONS"
        private const val OP_SENSITIVE = "android:receive_sensitive_notifications"

        fun component(context: Context) = ComponentName(context, NotificationListener::class.java)

        /** Android's notification access screen, on NectarLink's own switch where possible. */
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
