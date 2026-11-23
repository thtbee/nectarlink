// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.audio.AudioListeningService
import app.nectarlink.android.audio.AudioMixerActivity
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.ui.Preferences
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Brings the connection back after the phone restarts or NectarLink is
 * updated, without waiting for the app to be opened. Starting the core starts
 * the connection service when a PC is paired (Core.start). After an update,
 * the new version also says so in a notification.
 */
class StartupReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val updated = when (intent.action) {
            Intent.ACTION_MY_PACKAGE_REPLACED -> true
            Intent.ACTION_BOOT_COMPLETED -> false
            else -> return
        }
        val app = context.applicationContext as NectarlinkApplication
        if (!Preferences.isBackgroundServiceEnabled(context)) {
            if (updated) app.updater.onUpdated()
            return
        }
        val pending = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                app.core.start()
                // Kept alive until the core is up, so the service starts inside the
                // window Android allows after these broadcasts.
                withTimeoutOrNull(START_WAIT_MS) { app.core.state.first { it.status !is CoreStatus.Starting } }
                maybeRestoreAudioListening(context)
                if (updated) app.updater.onUpdated()
            } finally {
                pending.finish()
            }
        }
    }

    private fun maybeRestoreAudioListening(context: Context) {
        val wantsListening = Preferences.audioMixerPrefs(context).getBoolean("listening", false)
        if (!wantsListening) return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            if (AudioListeningService.start(context)) return
        }
        // Android 15+ blocks mediaPlayback FGS from BOOT_COMPLETED; post a tap-to-start reminder.
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        manager.createNotificationChannel(
            NotificationChannel(
                AUDIO_REMINDER_CHANNEL,
                context.getString(R.string.audio_mixer_channel),
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) },
        )
        val tap = PendingIntent.getActivity(
            context,
            AUDIO_REMINDER_ID,
            AudioMixerActivity.openIntent(context, startListening = true),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notice = NotificationCompat.Builder(context, AUDIO_REMINDER_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.audio_mixer_boot_reminder_title))
            .setContentText(context.getString(R.string.audio_mixer_boot_reminder_text))
            .setContentIntent(tap)
            .setAutoCancel(true)
            .setSilent(true)
            .build()
        runCatching { manager.notify(AUDIO_REMINDER_ID, notice) }
    }

    private companion object {
        const val START_WAIT_MS = 8_000L
        const val AUDIO_REMINDER_CHANNEL = "audio_mixer_boot_reminder"
        const val AUDIO_REMINDER_ID = 4322
    }
}
