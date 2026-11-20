// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

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
 * A paired PC making a problem report asks for this phone's recent activity
 * log (`diag.ask`, docs/protocol/diag.md). The log goes only when the user
 * taps Share; Don't share, swiping the notification away, or a minute
 * passing sends nothing.
 */
object LogRequests {
    private const val CHANNEL = "report_requests"
    private const val TAG = "report_log"
    private const val ANSWER_WITHIN_MS = 60 * 1000L
    internal const val EXTRA_PC = "report_pc"
    internal const val EXTRA_SHARE = "report_share"

    /** Shows the request; false if notifications are off. */
    fun show(context: Context, pcId: String, pcName: String): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_report_requests), NotificationManager.IMPORTANCE_HIGH),
        )
        fun answer(share: Boolean) = PendingIntent.getBroadcast(
            context,
            pcId.hashCode() * 2 + if (share) 1 else 0,
            Intent(context, LogAnswerReceiver::class.java).putExtra(EXTRA_PC, pcId).putExtra(EXTRA_SHARE, share),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val from = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.report_log_request_title, from))
            .setContentText(context.getString(R.string.report_log_request_text))
            .setStyle(NotificationCompat.BigTextStyle().bigText(context.getString(R.string.report_log_request_text)))
            .addAction(0, context.getString(R.string.report_log_share), answer(true))
            .addAction(0, context.getString(R.string.report_log_dont_share), answer(false))
            .setDeleteIntent(answer(false))
            .setAutoCancel(true)
            .setTimeoutAfter(ANSWER_WITHIN_MS)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(TAG, pcId.hashCode(), notification) }.isSuccess
    }

    fun dismiss(context: Context, pcId: String) {
        runCatching { NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode()) }
    }
}

/** Share or Don't share on a PC's request for the log. */
class LogAnswerReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pc = intent.getStringExtra(LogRequests.EXTRA_PC) ?: return
        val share = intent.getBooleanExtra(LogRequests.EXTRA_SHARE, false)
        LogRequests.dismiss(context, pc)
        (context.applicationContext as NectarlinkApplication).core.answerLogRequest(pc, share)
    }
}
