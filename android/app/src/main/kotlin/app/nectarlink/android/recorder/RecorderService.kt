// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.recorder

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
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.MediaRecorder
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.core.RecordingMarker
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.sqrt

data class RecorderSession(
    val active: Boolean = false,
    val paused: Boolean = false,
    val pcId: String? = null,
    val elapsedMs: Long = 0L,
    val level: Float = 0f,
    val markers: List<RecordingMarker> = emptyList(),
    val lastSavedId: String? = null,
)

/**
 * Records voice in 48 kHz mono AAC (`.m4a`) in a foreground service of type
 * `microphone` so recording continues with the screen off or while the user
 * switches apps, and sends the finished recording to the chosen PC on stop.
 */
class RecorderService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var encodeJob: Job? = null

    @Volatile
    private var running = false

    @Volatile
    private var paused = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val pcId = intent.getStringExtra(EXTRA_PC)
                if (!running && pcId != null) {
                    startRecordingSession(pcId)
                }
            }
            ACTION_PAUSE -> if (running && !paused) {
                paused = true
                _session.update { it.copy(paused = true, level = 0f) }
                updateNotification()
            }
            ACTION_RESUME -> if (running && paused) {
                paused = false
                _session.update { it.copy(paused = false) }
                updateNotification()
            }
            ACTION_STOP -> if (running) {
                running = false
            } else {
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    private fun startRecordingSession(pcId: String) {
        if (!hasPermission(this)) {
            stopSelf()
            return
        }
        createChannel(this)
        running = true
        paused = false
        _session.value = RecorderSession(
            active = true,
            paused = false,
            pcId = pcId,
            elapsedMs = 0L,
            level = 0f,
            markers = emptyList(),
            lastSavedId = _session.value.lastSavedId,
        )
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        } else {
            0
        }
        ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(pcId, paused = false, elapsedMs = 0L), type)

        val app = application as NectarlinkApplication
        val outFile = app.core.recordings.newRecordingFile()
        encodeJob = scope.launch(Dispatchers.IO) {
            val durationMs = recordM4a(outFile, pcId)
            val finalMarkers = _session.value.markers
            val savedId = if (outFile.exists() && outFile.length() > 0L) {
                app.core.onRecordingFinished(pcId, outFile, durationMs, finalMarkers).id
            } else {
                null
            }
            _session.value = RecorderSession(
                active = false,
                paused = false,
                pcId = pcId,
                elapsedMs = 0L,
                level = 0f,
                markers = emptyList(),
                lastSavedId = savedId,
            )
            ServiceCompat.stopForeground(this@RecorderService, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    @SuppressLint("MissingPermission")
    private fun recordM4a(outFile: File, pcId: String): Long {
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        ).coerceAtLeast(FRAME_SAMPLES * 2 * 4)

        val audioRecord = runCatching {
            AudioRecord(
                MediaRecorder.AudioSource.MIC,
                SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                minBuf,
            )
        }.getOrNull()

        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC)
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, CHANNELS).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, BIT_RATE)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16_384)
        }
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        codec.start()

        outFile.parentFile?.mkdirs()
        val muxer = MediaMuxer(outFile.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        var trackIndex = -1
        var muxerStarted = false
        val bufferInfo = MediaCodec.BufferInfo()

        val pcmShorts = ShortArray(FRAME_SAMPLES)
        var totalSamples = 0L
        var lastNotifSec = -1L

        fun drainEncoder(endOfStream: Boolean) {
            if (endOfStream) {
                val inIdx = codec.dequeueInputBuffer(10_000L)
                if (inIdx >= 0) {
                    val ptsUs = (totalSamples * 1_000_000L) / SAMPLE_RATE
                    codec.queueInputBuffer(inIdx, 0, 0, ptsUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                }
            }
            while (true) {
                val outIdx = codec.dequeueOutputBuffer(bufferInfo, if (endOfStream) 10_000L else 0L)
                when {
                    outIdx == MediaCodec.INFO_TRY_AGAIN_LATER -> break
                    outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        if (!muxerStarted) {
                            trackIndex = muxer.addTrack(codec.outputFormat)
                            muxer.start()
                            muxerStarted = true
                        }
                    }
                    outIdx >= 0 -> {
                        val encoded = codec.getOutputBuffer(outIdx)
                        if (bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                            bufferInfo.size = 0
                        }
                        if (bufferInfo.size > 0 && muxerStarted && encoded != null) {
                            encoded.position(bufferInfo.offset)
                            encoded.limit(bufferInfo.offset + bufferInfo.size)
                            muxer.writeSampleData(trackIndex, encoded, bufferInfo)
                        }
                        codec.releaseOutputBuffer(outIdx, false)
                        if (bufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
                    }
                }
            }
        }

        try {
            if (audioRecord?.state == AudioRecord.STATE_INITIALIZED) {
                runCatching { audioRecord.startRecording() }
            }
            while (running || totalSamples < SAMPLE_RATE / 4) {
                if (paused && running) {
                    Thread.sleep(30L)
                    continue
                }
                val read = if (audioRecord?.recordingState == AudioRecord.RECORDSTATE_RECORDING) {
                    audioRecord.read(pcmShorts, 0, FRAME_SAMPLES)
                } else {
                    Thread.sleep(20L)
                    pcmShorts.fill(0)
                    FRAME_SAMPLES
                }
                val count = if (read > 0) read else {
                    Thread.sleep(20L)
                    pcmShorts.fill(0)
                    FRAME_SAMPLES
                }

                // Compute RMS level for the live meter and add 1-LSB dither so silent virtual mics still mux cleanly.
                var sumSq = 0.0
                for (i in 0 until count) {
                    val s = pcmShorts[i].toInt()
                    sumSq += (s * s).toDouble()
                    if (s == 0 && (i and 1) == 0) {
                        pcmShorts[i] = 1
                    }
                }
                val rms = sqrt(sumSq / count.coerceAtLeast(1)) / 32768.0
                val meter = (rms * 4.5).toFloat().coerceIn(0f, 1f)

                val inIdx = codec.dequeueInputBuffer(10_000L)
                if (inIdx >= 0) {
                    val inBuf: ByteBuffer? = codec.getInputBuffer(inIdx)
                    if (inBuf != null) {
                        inBuf.clear()
                        inBuf.order(ByteOrder.nativeOrder())
                        val shortBuf = inBuf.asShortBuffer()
                        shortBuf.put(pcmShorts, 0, count)
                        val ptsUs = (totalSamples * 1_000_000L) / SAMPLE_RATE
                        codec.queueInputBuffer(inIdx, 0, count * 2, ptsUs, 0)
                    }
                }
                totalSamples += count
                val elapsedMs = (totalSamples * 1_000L) / SAMPLE_RATE
                _session.update { it.copy(elapsedMs = elapsedMs, level = meter) }

                val sec = elapsedMs / 1_000L
                if (sec != lastNotifSec) {
                    lastNotifSec = sec
                    val nm = getSystemService(NotificationManager::class.java)
                    nm?.notify(NOTIFICATION_ID, buildNotification(pcId, paused = paused, elapsedMs = elapsedMs))
                }

                drainEncoder(endOfStream = false)
            }
            drainEncoder(endOfStream = true)
        } catch (e: Exception) {
            Log.w(TAG, "recording interrupted", e)
        } finally {
            runCatching {
                if (audioRecord?.recordingState == AudioRecord.RECORDSTATE_RECORDING) {
                    audioRecord.stop()
                }
                audioRecord?.release()
            }
            runCatching {
                codec.stop()
                codec.release()
            }
            runCatching {
                if (muxerStarted) muxer.stop()
                muxer.release()
            }
        }
        return (totalSamples * 1_000L) / SAMPLE_RATE
    }

    private fun updateNotification() {
        val s = _session.value
        val pcId = s.pcId ?: return
        val nm = getSystemService(NotificationManager::class.java)
        nm?.notify(NOTIFICATION_ID, buildNotification(pcId, s.paused, s.elapsedMs))
    }

    private fun buildNotification(pcId: String, paused: Boolean, elapsedMs: Long): Notification {
        val core = (application as NectarlinkApplication).core
        val pcName = core.state.value.nameOf(pcId).orEmpty().ifEmpty { getString(R.string.your_pc) }

        val openIntent = Intent(this, MainActivity::class.java).apply {
            putExtra("record_pc", pcId)
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val openPending = PendingIntent.getActivity(
            this,
            1,
            openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val toggleAction = if (paused) ACTION_RESUME else ACTION_PAUSE
        val toggleLabel = getString(if (paused) R.string.recorder_resume else R.string.recorder_pause)
        val togglePending = PendingIntent.getService(
            this,
            2,
            Intent(this, RecorderService::class.java).setAction(toggleAction),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val stopPending = PendingIntent.getService(
            this,
            3,
            Intent(this, RecorderService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val title = if (paused) {
            getString(R.string.recorder_notification_paused, pcName)
        } else {
            getString(R.string.recorder_notification_recording, pcName)
        }

        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(formatDuration(elapsedMs))
            .setContentIntent(openPending)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, toggleLabel, togglePending)
            .addAction(0, getString(R.string.action_stop), stopPending)
            .build()
    }

    override fun onDestroy() {
        running = false
        encodeJob?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "RecorderService"
        private const val CHANNEL = "recorder"
        private const val NOTIFICATION_ID = 7
        private const val SAMPLE_RATE = 48_000
        private const val CHANNELS = 1
        private const val BIT_RATE = 128_000
        private const val FRAME_SAMPLES = 1024

        private const val ACTION_START = "app.nectarlink.android.recorder.START"
        private const val ACTION_PAUSE = "app.nectarlink.android.recorder.PAUSE"
        private const val ACTION_RESUME = "app.nectarlink.android.recorder.RESUME"
        private const val ACTION_STOP = "app.nectarlink.android.recorder.STOP"
        private const val EXTRA_PC = "pc"

        private val _session = MutableStateFlow(RecorderSession())
        val session: StateFlow<RecorderSession> = _session.asStateFlow()

        fun hasPermission(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
                PackageManager.PERMISSION_GRANTED

        fun start(context: Context, pcId: String) {
            val intent = Intent(context, RecorderService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_PC, pcId)
            ContextCompat.startForegroundService(context, intent)
        }

        fun pause(context: Context) {
            context.startService(Intent(context, RecorderService::class.java).setAction(ACTION_PAUSE))
        }

        fun resume(context: Context) {
            context.startService(Intent(context, RecorderService::class.java).setAction(ACTION_RESUME))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, RecorderService::class.java).setAction(ACTION_STOP))
        }

        fun addMarker(label: String? = null) {
            _session.update { s ->
                if (!s.active) return@update s
                val cleaned = label?.trim()?.take(80)?.ifEmpty { null }
                val marker = RecordingMarker(atMs = s.elapsedMs.coerceAtLeast(0L).toULong(), label = cleaned)
                s.copy(markers = s.markers + marker)
            }
        }

        fun formatDuration(ms: Long): String {
            val totalSecs = (ms.coerceAtLeast(0L)) / 1_000L
            val mins = totalSecs / 60L
            val secs = totalSecs % 60L
            return String.format(java.util.Locale.US, "%02d:%02d", mins, secs)
        }

        private fun createChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java) ?: return
            val channel = NotificationChannel(
                CHANNEL,
                context.getString(R.string.channel_recorder),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                setShowBadge(false)
            }
            nm.createNotificationChannel(channel)
        }
    }
}
