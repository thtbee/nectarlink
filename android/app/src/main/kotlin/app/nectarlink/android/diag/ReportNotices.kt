// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.core.ReportNotice

/**
 * Someone replied to a report this phone sent, or closed it. Only the
 * report's number shows; tapping opens it on GitHub.
 */
object ReportNotices {
    private const val CHANNEL = "report_replies"
    private const val TAG = "report_reply"

    fun show(context: Context, notice: ReportNotice) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val (issue, title) = when (notice) {
            is ReportNotice.Replied -> notice.issue to R.string.report_reply_title
            is ReportNotice.Closed -> notice.issue to R.string.report_closed_title
        }
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_report_replies), NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = PendingIntent.getActivity(
            context,
            issue.toInt(),
            Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/thtbee/nectarlink/issues/$issue"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(title))
            .setContentText(context.getString(R.string.report_notice_text, issue.toLong()))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(TAG, issue.toInt(), notification) }
    }
}
