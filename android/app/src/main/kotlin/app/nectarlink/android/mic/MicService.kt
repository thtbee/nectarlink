// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mic

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.audiofx.AutomaticGainControl
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Phone as microphone (docs/protocol/mic.md): streams the microphone to a PC as 48 kHz mono
 * 16-bit PCM, 10 ms at a time, in a foreground service of type `microphone` so it keeps going
 * with the screen off. It records for calls (echo cancellation and noise suppression where the
 * phone has them) and stops when the PC stops listening, the connection ends, or the user taps
 * Stop.
 */
class MicService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var job: Job? = null

    @Volatile
    private var running = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val pcId = intent.getStringExtra(EXTRA_PC)
                if (pcId != null && !running) {
                    start(pcId)
                } else if (running) {
                    val activePc = _streaming.value ?: pcId.orEmpty()
                    val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
                    runCatching { ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(activePc), type) }
                } else {
                    stopAfterForeground("")
                }
            }
            ACTION_STOP -> {
                val pcId = _streaming.value
                running = false
                // The phone's user stopped it: tell the PC.
                if (intent.getBooleanExtra(EXTRA_FROM_USER, false) && pcId != null) {
                    (application as NectarlinkApplication).core.micStop(pcId)
                }
                if (job == null) stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    private fun stopAfterForeground(pcId: String) {
        createChannel(this)
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
        runCatching { ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(pcId), type) }
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    @SuppressLint("MissingPermission")
    private fun start(pcId: String) {
        if (!hasPermission(this)) {
            stopAfterForeground(pcId)
            return
        }
        createChannel(this)
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        } else {
            0
        }
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(pcId), type)
        running = true
        _streaming.value = pcId
        val core = (application as NectarlinkApplication).core
        job = scope.launch {
            try {
                val stream = core.micOpen(pcId) ?: return@launch
                val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
                val recorder = AudioRecord(
                    MediaRecorder.AudioSource.VOICE_COMMUNICATION,
                    RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    maxOf(minimum, CHUNK_BYTES * 4),
                )
                if (recorder.state != AudioRecord.STATE_INITIALIZED) {
                    Log.w(TAG, "microphone not available")
                    recorder.release()
                    stream.end()
                    stream.close()
                    core.micStop(pcId)
                    return@launch
                }
                // Voice calls level the voice on most phones already; ask for it
                // where the phone offers it, so a quiet voice still comes through.
                val gain = if (AutomaticGainControl.isAvailable()) {
                    runCatching { AutomaticGainControl.create(recorder.audioSessionId)?.apply { enabled = true } }
                        .getOrNull()
                } else {
                    null
                }
                try {
                    if (stream.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(RATE.toUInt(), 1u)) ==
                        MirrorSendResult.CLOSED
                    ) {
                        return@launch
                    }
                    recorder.startRecording()
                    val chunk = ByteArray(CHUNK_BYTES)
                    var samples = 0L
                    while (running) {
                        val read = recorder.read(chunk, 0, chunk.size)
                        if (read <= 0) continue
                        val timeUs = samples * 1_000_000L / RATE
                        samples += read / 2
                        // Dropped packets (a slow link) only lose that moment; a closed stream ends it.
                        if (stream.send(VideoPacketKind.FRAME, timeUs.toULong(), chunk.copyOf(read)) ==
                            MirrorSendResult.CLOSED
                        ) {
                            break
                        }
                    }
                } finally {
                    runCatching { gain?.release() }
                    runCatching { recorder.stop() }
                    recorder.release()
                    stream.end()
                    stream.close()
                }
            } catch (e: Exception) {
                Log.w(TAG, "microphone stream failed", e)
            } finally {
                running = false
                _streaming.value = null
                job = null
                ServiceCompat.stopForeground(this@MicService, ServiceCompat.STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
    }

    private fun notification(pcId: String): Notification {
        val core = (application as NectarlinkApplication).core
        val pcName = core.state.value.nameOf(pcId).orEmpty().ifEmpty { getString(R.string.your_pc) }
        val stop = PendingIntent.getService(
            this,
            1,
            Intent(this, MicService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_FROM_USER, true),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.mic_notification_title, pcName))
            .setContentText(getString(R.string.mic_notification_text))
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    override fun onDestroy() {
        running = false
        scope.cancel()
        _streaming.value = null
        super.onDestroy()
    }

    companion object {
        private const val TAG = "MicService"
        private const val CHANNEL = "mic"
        private const val NOTIFICATION_ID = 9
        private const val RATE = 48_000

        /** 10 ms of 16-bit mono at [RATE]. */
        private const val CHUNK_BYTES = RATE / 100 * 2

        private const val ACTION_START = "app.nectarlink.android.mic.START"
        private const val ACTION_STOP = "app.nectarlink.android.mic.STOP"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_FROM_USER = "from_user"

        private val _streaming = MutableStateFlow<String?>(null)

        /** The PC this phone's microphone streams to, if any. */
        val streaming: StateFlow<String?> = _streaming.asStateFlow()

        fun hasPermission(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
                PackageManager.PERMISSION_GRANTED

        /** Starts streaming to [pcId]; only from the foreground (Android's rule for the mic). */
        fun start(context: Context, pcId: String) {
            if (!hasPermission(context) || _streaming.value != null) return
            val intent = Intent(context, MicService::class.java).setAction(ACTION_START).putExtra(EXTRA_PC, pcId)
            ContextCompat.startForegroundService(context, intent)
        }

        /** The per-device toggle that lets a PC ask for the microphone (docs/protocol/mic.md). */
        const val TOGGLE_NAME = "mic"

        /** [pcId] stopped listening; another PC's stream carries on. */
        fun stop(context: Context, pcId: String) {
            if (_streaming.value != pcId) return
            context.startService(Intent(context, MicService::class.java).setAction(ACTION_STOP))
        }

        /** The phone's user stopped it in the app: the PC is told. */
        fun stopFromUser(context: Context) {
            if (_streaming.value == null) return
            context.startService(
                Intent(context, MicService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_FROM_USER, true),
            )
        }

        private fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_mic), NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }
}
