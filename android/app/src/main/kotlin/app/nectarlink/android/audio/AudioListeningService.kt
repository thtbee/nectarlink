// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mic.MicService
import app.nectarlink.android.mirror.SoundCapture
import app.nectarlink.services.client.NectarlinkServiceContract
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** Keeps managed PC audio listening alive while the mixer panel is closed. */
class AudioListeningService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var watcher: Job? = null
    private var phoneCapture: SoundCapture? = null
    private var foreground = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.audio_mixer_channel), NotificationManager.IMPORTANCE_LOW)
                .apply { setShowBadge(false) },
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> startListening()
            ACTION_STOP -> AudioRouting.setListening(false)
            ACTION_MIC_MUTE -> {
                val mic = AudioRouting.state.value.mic
                if (mic.streamingTo != null) AudioRouting.setMicMuted(!mic.muted)
            }
            ACTION_MIC_OFF -> {
                AudioRouting.selectMicTarget(null)
                MicService.stopFromUser(this)
            }
            else -> stopSelf(startId)
        }
        if (!foreground) stopSelf(startId)
        return START_NOT_STICKY
    }

    private fun startListening() {
        val initial = AudioRouting.state.value
        val promoted = runCatching {
            ServiceCompat.startForeground(
                this, NOTIFICATION_ID, notification(initial), ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
            )
        }.isSuccess
        if (!promoted) return
        foreground = true
        AudioRouting.setListening(true)
        AudioMixerTileService.requestUpdate(this)
        if (watcher != null) return
        watcher = scope.launch {
            AudioRouting.state.map { NotificationState(it) }.distinctUntilChanged().collect { state ->
                if (!state.listening) {
                    stopPhoneCapture()
                    stopSelf()
                } else {
                    syncPhoneCapture(state.wantPhoneCapture)
                    runCatching {
                        getSystemService(NotificationManager::class.java)?.notify(NOTIFICATION_ID, buildNotification(state))
                    }
                }
            }
        }
    }

    private fun syncPhoneCapture(want: Boolean) {
        if (want && Elevated.hasCapability(NectarlinkServiceContract.CAP_DEVICE_AUDIO)) {
            if (phoneCapture == null) {
                val capture = SoundCapture.managed(
                    context = this,
                    projection = null,
                    quietPhone = true,
                    onPcm = AudioRouting::sendPhonePcm,
                    onPeak = AudioRouting::updatePhonePeak,
                    onEnded = {
                        scope.launch {
                            phoneCapture = null
                        }
                    },
                )
                phoneCapture = capture
                capture.start()
            }
        } else {
            stopPhoneCapture()
        }
    }

    private fun stopPhoneCapture() {
        phoneCapture?.stop()
        phoneCapture = null
    }

    private fun notification(state: AudioRouting.State) = buildNotification(NotificationState(state))

    private fun buildNotification(state: NotificationState): Notification {
        val panel = PendingIntent.getActivity(
            this, 0, AudioMixerActivity.openIntent(this),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.audio_mixer_notification_title))
            .setContentText(
                state.outputName?.let { getString(R.string.audio_mixer_on_pc, it) }
                    ?: getString(R.string.audio_mixer_listening),
            )
            .setContentIntent(panel)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), action(ACTION_STOP, 1))
        if (state.micActive) {
            builder.addAction(
                0,
                getString(if (state.micMuted) R.string.audio_mixer_mic_unmute else R.string.audio_mixer_mic_mute),
                action(ACTION_MIC_MUTE, 2),
            )
            builder.addAction(0, getString(R.string.audio_mixer_mic_off), action(ACTION_MIC_OFF, 3))
        }
        return builder.build()
    }

    private fun action(action: String, requestCode: Int) = PendingIntent.getService(
        this, requestCode, Intent(this, AudioListeningService::class.java).setAction(action),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    override fun onDestroy() {
        stopPhoneCapture()
        scope.cancel()
        if (foreground) {
            AudioRouting.setListening(false)
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        }
        AudioMixerTileService.requestUpdate(this)
        super.onDestroy()
    }

    private data class NotificationState(
        val listening: Boolean,
        val outputName: String?,
        val micActive: Boolean,
        val micMuted: Boolean,
        val wantPhoneCapture: Boolean,
    ) {
        constructor(state: AudioRouting.State) : this(
            state.listening,
            state.outputPeer()?.let { state.nameOf(it.deviceId) },
            state.mic.streamingTo != null,
            state.mic.muted,
            state.listening && state.output is AudioRouting.Output.Pc && state.includePhone,
        )
    }

    companion object {
        private const val CHANNEL = "audio_listening"
        private const val NOTIFICATION_ID = 15
        private const val ACTION_START = "app.nectarlink.android.audio.START"
        private const val ACTION_STOP = "app.nectarlink.android.audio.STOP"
        private const val ACTION_MIC_MUTE = "app.nectarlink.android.audio.MIC_MUTE"
        private const val ACTION_MIC_OFF = "app.nectarlink.android.audio.MIC_OFF"

        fun start(context: Context): Boolean = runCatching {
            ContextCompat.startForegroundService(context, Intent(context, AudioListeningService::class.java).setAction(ACTION_START))
        }.isSuccess

        fun stop(context: Context) {
            AudioRouting.setListening(false)
            AudioMixerTileService.requestUpdate(context)
        }
    }
}
