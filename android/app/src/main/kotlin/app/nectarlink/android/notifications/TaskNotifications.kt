// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.core.TaskNotify

/**
 * Shows and updates a PC's watched command (`nectarlink notify-when` / `task.notify`)
 * on this phone: an ongoing chronometer notification (promoted to a Live Update on
 * Android 16+) while the command runs, replaced in place by an alerting completion
 * notification when it finishes.
 */
object TaskNotifications {
    private const val CHANNEL_RUNNING = "pc_tasks_running"
    private const val CHANNEL_DONE = "pc_tasks"
    private const val NOTIFICATION_ID = 0x5441534B // "TASK"

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_RUNNING,
                context.getString(R.string.channel_pc_tasks_running),
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) },
        )
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_DONE,
                context.getString(R.string.channel_pc_tasks),
                NotificationManager.IMPORTANCE_HIGH,
            ),
        )
    }

    /** Shows or updates a task notification from a paired PC; false if notifications are disabled. */
    fun show(context: Context, fromId: String, pcName: String, task: TaskNotify): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        createChannels(context)
        val from = pcName.ifBlank { context.getString(R.string.your_pc) }
        val open = PendingIntent.getActivity(
            context,
            task.id.hashCode(),
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val runTag = "pc_task_run:$fromId:${task.id}"
        val doneTag = "pc_task_done:$fromId:${task.id}"
        val manager = NotificationManagerCompat.from(context)
        if (!task.active && task.exitCode == null) {
            return runCatching {
                manager.cancel(runTag, NOTIFICATION_ID)
            }.isSuccess
        }
        return if (task.active) {
            val notification = buildRunning(context, from, task, open, runTag)
            runCatching {
                manager.notify(runTag, NOTIFICATION_ID, notification)
            }.isSuccess
        } else {
            val notification = buildDone(context, from, task, open, doneTag)
            runCatching {
                manager.cancel(runTag, NOTIFICATION_ID)
                manager.notify(doneTag, NOTIFICATION_ID, notification)
            }.isSuccess
        }
    }

    @SuppressLint("NewApi")
    private fun buildRunning(
        context: Context,
        from: String,
        task: TaskNotify,
        open: PendingIntent,
        groupKey: String,
    ): Notification {
        val title = task.title.trim().ifEmpty { "Task" }
        val body = context.getString(R.string.task_running_on, from)
        val now = System.currentTimeMillis()
        val elapsedMs = task.elapsedMs.toLong().coerceAtLeast(0L)
        val whenMs = (now - elapsedMs).coerceAtLeast(1L)
        val builder = Notification.Builder(context, CHANNEL_RUNNING)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            .setSubText(from)
            .setGroup(groupKey)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(true)
            .setWhen(whenMs)
            .setUsesChronometer(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA) {
            runCatching { builder.setRequestPromotedOngoing(true) }
            builder.setShortCriticalText(shortChipText(title))
        }
        return builder.build()
    }

    private fun buildDone(
        context: Context,
        from: String,
        task: TaskNotify,
        open: PendingIntent,
        groupKey: String,
    ): Notification {
        val title = completionTitle(task.title, task.exitCode)
        val body = completionBody(from, task.elapsedMs.toLong().coerceAtLeast(0L))
        return Notification.Builder(context, CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            .setSubText(from)
            .setGroup(groupKey)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setContentIntent(open)
            .setOngoing(false)
            .setAutoCancel(true)
            .setShowWhen(true)
            .setWhen(System.currentTimeMillis())
            .setCategory(Notification.CATEGORY_STATUS)
            .build()
    }

    internal fun completionTitle(rawTitle: String, exitCode: Int?): String {
        val base = rawTitle.trim().ifEmpty { "Task" }
        return if (exitCode == null || exitCode == 0) {
            "$base finished"
        } else {
            "$base failed (exit $exitCode)"
        }
    }

    internal fun completionBody(from: String, elapsedMs: Long): String {
        val took = formatTook(elapsedMs)
        return "$took · $from"
    }

    internal fun formatTook(elapsedMs: Long): String {
        val totalSecs = (elapsedMs.coerceAtLeast(0L) + 500L) / 1000L
        val hours = totalSecs / 3600L
        val mins = (totalSecs % 3600L) / 60L
        val secs = totalSecs % 60L
        return when {
            hours > 0L -> "Took ${hours}h %02dm".format(java.util.Locale.US, mins)
            mins > 0L -> "Took ${mins}m %02ds".format(java.util.Locale.US, secs)
            else -> "Took ${secs}s"
        }
    }

    internal fun shortChipText(title: String): String {
        val trimmed = title.trim().ifEmpty { "Running" }
        return if (trimmed.length <= 7) trimmed else trimmed.take(6).trimEnd() + "…"
    }
}
