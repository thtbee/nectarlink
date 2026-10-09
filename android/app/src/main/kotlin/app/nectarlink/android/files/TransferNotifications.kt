// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.Manifest
import android.app.DownloadManager
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
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus

/**
 * Transfers in the notification shade: progress with Cancel while one
 * runs, and "Received" (tap to open) when files from a PC are saved.
 */
internal object TransferNotifications {
    private const val CHANNEL_PROGRESS = "transfers"
    private const val CHANNEL_RECEIVED = "received"
    const val ACTION_CANCEL = "app.nectarlink.action.CANCEL_TRANSFER"
    const val EXTRA_ID = "transfer"

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_PROGRESS, context.getString(R.string.channel_transfers), NotificationManager.IMPORTANCE_LOW),
        )
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_RECEIVED, context.getString(R.string.channel_received), NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    private fun idOf(transfer: Transfer) = transfer.id.hashCode()

    private fun title(context: Context, transfer: Transfer): String = transferTitle(context.resources, transfer)

    /** Shows or updates a running transfer, or removes it once it ended. */
    fun update(context: Context, transfer: Transfer, pcName: String) {
        if (!allowed(context)) return
        val manager = NotificationManagerCompat.from(context)
        val status = transfer.status
        if (status !is TransferStatus.Running && status !is TransferStatus.Waiting) {
            manager.cancel(idOf(transfer))
            if (status is TransferStatus.Failed) failed(context, transfer, pcName)
            return
        }
        val incoming = transfer.direction == TransferDirection.INCOMING
        val text = when {
            status is TransferStatus.Waiting -> context.getString(R.string.transfer_waiting, pcName)
            incoming -> context.getString(R.string.transfer_receiving, pcName)
            else -> context.getString(R.string.transfer_sending, pcName)
        }
        val percent = if (transfer.total == 0uL) 100 else (transfer.done.toDouble() / transfer.total.toDouble() * 100).toInt()
        val cancel = PendingIntent.getBroadcast(
            context,
            idOf(transfer),
            Intent(context, CancelReceiver::class.java).setAction(ACTION_CANCEL).putExtra(EXTRA_ID, transfer.id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL_PROGRESS)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title(context, transfer))
            .setContentText(text)
            .setProgress(100, percent, status is TransferStatus.Waiting)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .addAction(0, context.getString(R.string.action_cancel), cancel)
            .build()
        post(context, idOf(transfer), notification)
    }

    /**
     * Files from a PC are saved: tap to open the first one, or the
     * Downloads list when a folder came. When `openOnArrival` is true for a
     * single safe document, attempts to open it immediately with the default app.
     */
    fun received(context: Context, transfer: Transfer, files: List<ReceivedFiles.Published>, pcName: String) {
        if (files.isEmpty()) return
        val first = files[0]
        val isHandoffDoc = transfer.openOnArrival &&
            files.size == 1 &&
            transfer.files.toInt() == 1 &&
            app.nectarlink.core.isSafeHandoffDocument(first.name)
        val directView = Intent(Intent.ACTION_VIEW)
            .setDataAndType(first.uri, first.mime)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
        if (isHandoffDoc) {
            runCatching { context.startActivity(directView) }
        }
        if (!allowed(context)) return
        val open = if (transfer.files.toInt() == transfer.names.size) {
            Intent.createChooser(
                Intent(Intent.ACTION_VIEW).setDataAndType(first.uri, first.mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
                null,
            )
        } else {
            Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)
        }
        val tap = PendingIntent.getActivity(
            context, idOf(transfer), open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val from = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val text = if (isHandoffDoc) {
            context.getString(R.string.handoff_doc_received_from, from)
        } else {
            context.getString(R.string.transfer_received_from, from)
        }
        val builder = NotificationCompat.Builder(context, CHANNEL_RECEIVED)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title(context, transfer))
            .setContentText(text)
            .setContentIntent(tap)
            .setAutoCancel(true)
        if (isHandoffDoc) {
            builder.addAction(0, context.getString(R.string.clip_action_open), tap)
        }
        post(context, idOf(transfer), builder.build())
    }

    private fun failed(context: Context, transfer: Transfer, pcName: String) {
        val reason = (transfer.status as? TransferStatus.Failed)?.reason
        val text = when (reason) {
            "denied" -> context.getString(R.string.transfer_denied, pcName)
            "unreachable" -> context.getString(R.string.transfer_unreachable, pcName)
            "noSpace" -> context.getString(R.string.transfer_no_space)
            else -> context.getString(R.string.transfer_failed)
        }
        val notification = NotificationCompat.Builder(context, CHANNEL_RECEIVED)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title(context, transfer))
            .setContentText(text)
            .setAutoCancel(true)
            .build()
        post(context, idOf(transfer), notification)
    }

    private fun allowed(context: Context) =
        android.os.Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun post(context: Context, id: Int, notification: android.app.Notification) {
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        NotificationManagerCompat.from(context).notify(id, notification)
    }

    /** "Cancel" on a progress notification. */
    class CancelReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val id = intent.getStringExtra(EXTRA_ID) ?: return
            (context.applicationContext as NectarlinkApplication).core.cancelTransfer(id)
        }
    }
}
