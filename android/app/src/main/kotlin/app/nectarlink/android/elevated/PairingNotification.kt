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
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Where the user types wireless debugging's pairing code: Android closes
 * its pairing dialog when another app comes to the front, so the code is
 * typed into this notification's reply field instead, with the dialog
 * still open behind it.
 */
object PairingNotification {
    private const val CHANNEL = "elevated"
    private const val ID = 7
    private const val KEY_CODE = "code"
    /** How long the receiver waits for pairing before letting Android move on. */
    private const val RECEIVER_MS = 8_000L
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Asks for the code; false if notifications are off for the app. [collapsed]
     * is all a collapsed notification shows, and many phones keep it collapsed,
     * which hides the Enter code button: by default it says how to get to it.
     */
    fun show(
        context: Context,
        text: String = context.getString(R.string.elevated_pair_text),
        collapsed: String = context.getString(R.string.elevated_pair_collapsed),
    ): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return false
        }
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_elevated), NotificationManager.IMPORTANCE_HIGH),
        )
        val reply = PendingIntent.getBroadcast(
            context, ID, Intent(context, Receiver::class.java),
            PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val action = NotificationCompat.Action.Builder(0, context.getString(R.string.elevated_pair_action), reply)
            .addRemoteInput(RemoteInput.Builder(KEY_CODE).setLabel(context.getString(R.string.elevated_pair_hint)).build())
            .build()
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.elevated_pair_title))
            .setContentText(collapsed)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .addAction(action)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(ID, notification) }.isSuccess
    }

    fun dismiss(context: Context) = NotificationManagerCompat.from(context).cancel(ID)

    /** The code the user typed. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val code = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_CODE)?.toString()?.filter(Char::isDigit)
            val app = context.applicationContext
            if (code.isNullOrEmpty()) {
                show(app)
                return
            }
            val pairing = app.getString(R.string.elevated_pairing)
            show(app, pairing, collapsed = pairing)
            val pending = goAsync()
            // Android stops a receiver after about ten seconds (an "isn't
            // responding" error), and finding the pairing dialog alone can take
            // that long: let it go in time; the pairing carries on.
            val released = AtomicBoolean(false)
            val release = { if (released.compareAndSet(false, true)) pending.finish() }
            scope.launch {
                delay(RECEIVER_MS)
                release()
            }
            scope.launch {
                try {
                    if (Elevated.pair(code)) {
                        dismiss(app)
                    } else {
                        val why = when ((Elevated.state.value as? Elevated.State.Failed)?.reason) {
                            Elevated.Reason.NoPairingDialog -> R.string.elevated_no_dialog
                            Elevated.Reason.WrongCode -> R.string.elevated_wrong_code
                            else -> R.string.elevated_failed
                        }
                        val message = app.getString(why)
                        show(app, message, collapsed = message)
                    }
                } finally {
                    release()
                }
            }
        }
    }
}
