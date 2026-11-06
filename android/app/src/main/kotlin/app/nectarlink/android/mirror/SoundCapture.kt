// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Build
import android.os.SystemClock
import android.util.Log
import androidx.core.content.ContextCompat
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig
import java.net.Socket

/**
 * The phone's sound while its screen is shared: what apps play (media, games,
 * and apps that don't say), captured via Android 10+
 * [AudioPlaybackCaptureConfiguration] or via the Elevated `shell` helper's
 * internal submix (`"A"` stream) as 48 kHz stereo PCM in 10 ms packets.
 */
class SoundCapture(
    private val projection: MediaProjection?,
    private val stream: MirrorStream,
    /** Called once the sound stops (the PC stopped listening, or capture failed). */
    private val onEnded: () -> Unit = {},
) {
    @Volatile private var running = false
    @Volatile private var elevatedSocket: Socket? = null
    private var thread: Thread? = null

    /**
     * After [start]: whether the sound comes through Android's playback
     * capture, which leaves the phone's speaker playing (the Elevated helper's
     * remote submix takes the sound off it).
     */
    var usesPlaybackCapture = false
        private set

    /** Whether any sound got as far as being sent. */
    @Volatile
    var sentSound = false
        private set

    @SuppressLint("MissingPermission")
    fun start() {
        val proj = projection
        if (proj != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val record = runCatching {
                val config = AudioPlaybackCaptureConfiguration.Builder(proj)
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                    .addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                    .build()
                val format = AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(RATE)
                    .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                    .build()
                val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT)
                AudioRecord.Builder()
                    .setAudioFormat(format)
                    .setBufferSizeInBytes(maxOf(minimum, PACKET_BYTES * 8))
                    .setAudioPlaybackCaptureConfig(config)
                    .build()
            }.getOrNull()

            if (record != null && record.state == AudioRecord.STATE_INITIALIZED) {
                running = true
                usesPlaybackCapture = true
                thread = Thread({ runRecord(record) }, "sound-capture").apply {
                    priority = Thread.MAX_PRIORITY
                    start()
                }
                return
            }
            record?.runCatching { release() }
        }

        if (Elevated.running) {
            running = true
            thread = Thread({ runElevated() }, "sound-capture-elevated").apply {
                priority = Thread.MAX_PRIORITY
                start()
            }
            return
        }

        Log.w(TAG, "no sound capture available")
        stream.end()
        onEnded()
    }

    private fun runRecord(record: AudioRecord) {
        try {
            record.startRecording()
            sentSound = true
            if (stream.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(RATE.toUInt(), CHANNELS.toUByte())) == MirrorSendResult.CLOSED) return
            val started = SystemClock.elapsedRealtimeNanos()
            val packet = ByteArray(PACKET_BYTES)
            while (running) {
                var filled = 0
                while (filled < packet.size && running) {
                    val read = record.read(packet, filled, packet.size - filled)
                    if (read < 0) return
                    filled += read
                }
                if (!running) return
                val time = ((SystemClock.elapsedRealtimeNanos() - started) / 1000).toULong()
                if (stream.send(VideoPacketKind.FRAME, time, packet.copyOf()) == MirrorSendResult.CLOSED) return
            }
        } catch (e: Exception) {
            Log.w(TAG, "sound capture stopped", e)
        } finally {
            running = false
            runCatching { record.stop() }
            record.release()
            stream.end()
            onEnded()
        }
    }

    private fun runElevated() {
        val sock = Elevated.openAudioStream()
        if (sock == null) {
            running = false
            stream.end()
            onEnded()
            return
        }
        elevatedSocket = sock
        try {
            val input = sock.getInputStream()
            val packet = ByteArray(PACKET_BYTES)
            var configSent = false
            var started = 0L
            while (running) {
                var filled = 0
                while (filled < packet.size && running) {
                    val read = input.read(packet, filled, packet.size - filled)
                    if (read <= 0) return
                    filled += read
                }
                if (!running) return
                if (!configSent) {
                    configSent = true
                    sentSound = true
                    started = SystemClock.elapsedRealtimeNanos()
                    if (stream.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(RATE.toUInt(), CHANNELS.toUByte())) == MirrorSendResult.CLOSED) return
                }
                val time = ((SystemClock.elapsedRealtimeNanos() - started) / 1000).toULong()
                if (stream.send(VideoPacketKind.FRAME, time, packet.copyOf()) == MirrorSendResult.CLOSED) return
            }
        } catch (e: Exception) {
            if (running) Log.w(TAG, "elevated sound capture stopped", e)
        } finally {
            running = false
            elevatedSocket = null
            runCatching { sock.close() }
            stream.end()
            onEnded()
        }
    }

    fun stop() {
        running = false
        runCatching { elevatedSocket?.close() }
        elevatedSocket = null
        thread = null
    }

    companion object {
        private const val TAG = "SoundCapture"
        private const val RATE = 48_000
        private const val CHANNELS = 2
        /** 10 ms of 16-bit stereo. */
        private const val PACKET_BYTES = RATE / 100 * CHANNELS * 2

        /** Whether this phone can share its sound (Elevated helper or Android 10+ with RECORD_AUDIO). */
        fun canCapture(context: Context): Boolean =
            Elevated.running ||
                (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                    ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED)
    }
}
