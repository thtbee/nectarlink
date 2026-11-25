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
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.core.ReportAnswer
import app.nectarlink.core.ReportNotice
import app.nectarlink.core.ReportReplyOutcome
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Someone answered a report this phone sent, or closed it: what they wrote,
 * and a field to reply in, without a GitHub account. Tapping opens Your
 * reports at it.
 */
object ReportNotices {
    private const val CHANNEL = "report_replies"
    private const val TAG = "report_reply"
    private const val KEY_TEXT = "reply"
    internal const val EXTRA_RID = "rid"
    /** On MainActivity's intent: open Your reports at this report. */
    const val EXTRA_OPEN_REPORTS = "open_reports"

    private val _open = MutableStateFlow<String?>(null)
    /** A report to show in Your reports, set by a notification; "" for the list. */
    val open: StateFlow<String?> = _open

    fun requestOpen(rid: String) {
        _open.value = rid
    }

    fun opened() {
        _open.value = null
    }

    fun show(context: Context, notice: ReportNotice, rid: String, latest: ReportAnswer?, canReply: Boolean) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val (issue, title) = when (notice) {
            is ReportNotice.Replied -> notice.issue to R.string.report_reply_title
            is ReportNotice.Closed -> notice.issue to R.string.report_closed_title
        }
        createChannel(context)
        val text = latest?.let { context.getString(R.string.report_answer_line, who(context, it), excerpt(it.text)) }
            ?: context.getString(R.string.report_notice_open_text, issue.toLong())
        val builder = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(title))
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(openReports(context, rid))
            .setAutoCancel(true)
        if (canReply) {
            val reply = PendingIntent.getBroadcast(
                context,
                rid.hashCode(),
                Intent(context, ReplyReceiver::class.java).putExtra(EXTRA_RID, rid),
                PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(
                NotificationCompat.Action.Builder(0, context.getString(R.string.report_reply_action), reply)
                    .addRemoteInput(RemoteInput.Builder(KEY_TEXT).setLabel(context.getString(R.string.report_reply_hint)).build())
                    .setAllowGeneratedReplies(false)
                    .build(),
            )
        }
        runCatching { NotificationManagerCompat.from(context).notify(TAG, rid.hashCode(), builder.build()) }
    }

    /** After a reply from the notification: sent, or why not. */
    private fun replied(context: Context, rid: String, outcome: ReportReplyOutcome) {
        val manager = NotificationManagerCompat.from(context)
        if (outcome == ReportReplyOutcome.SENT) {
            runCatching { manager.cancel(TAG, rid.hashCode()) }
            return
        }
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        createChannel(context)
        val why = when (outcome) {
            ReportReplyOutcome.LIMITED -> R.string.report_reply_limited
            ReportReplyOutcome.REFUSED -> R.string.report_reply_refused
            else -> R.string.report_reply_later
        }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.report_reply_not_sent))
            .setContentText(context.getString(why))
            .setStyle(NotificationCompat.BigTextStyle().bigText(context.getString(why)))
            .setContentIntent(openReports(context, rid))
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .build()
        runCatching { manager.notify(TAG, rid.hashCode(), notification) }
    }

    private fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_report_replies), NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    private fun openReports(context: Context, rid: String): PendingIntent = PendingIntent.getActivity(
        context,
        rid.hashCode(),
        Intent(context, MainActivity::class.java)
            .putExtra(EXTRA_OPEN_REPORTS, rid)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Who wrote an answer, as the app says it. */
    fun who(context: Context, answer: ReportAnswer): String = when (answer.who) {
        "team" -> "NectarLink"
        "you" -> context.getString(R.string.report_who_you)
        "other" -> context.getString(R.string.report_who_other)
        else -> answer.login
    }

    private fun excerpt(text: String): String {
        val line = text.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
        return if (line.length <= 200) line else line.take(200).trimEnd() + "…"
    }

    /** A reply typed in the notification. */
    class ReplyReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val rid = intent.getStringExtra(EXTRA_RID) ?: return
            val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_TEXT)?.toString()?.trim()
            if (text.isNullOrEmpty()) return
            val pending = goAsync()
            val app = context.applicationContext
            Thread {
                try {
                    replied(app, rid, Reports.reply(rid, text))
                } finally {
                    pending.finish()
                }
            }.apply { name = "report-reply" }.start()
        }
    }
}
