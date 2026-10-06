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
import androidx.annotation.RequiresApi
import androidx.core.content.ContextCompat
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig

/**
 * The phone's sound while its screen is shared (Android 10+): what apps
 * play (media, games, and apps that don't say), captured with the screen
 * share's consent, as 48 kHz stereo PCM in 10 ms packets. Apps that opt out
 * of playback capture, and calls, aren't heard.
 */
class SoundCapture(
    private val projection: MediaProjection,
    private val stream: MirrorStream,
) {
    @Volatile private var running = false
    private var thread: Thread? = null

    @SuppressLint("MissingPermission") // Checked by canCapture, before this is made.
    @RequiresApi(Build.VERSION_CODES.Q)
    fun start() {
        val config = AudioPlaybackCaptureConfiguration.Builder(projection)
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
        val record = runCatching {
            AudioRecord.Builder()
                .setAudioFormat(format)
                .setBufferSizeInBytes(maxOf(minimum, PACKET_BYTES * 8))
                .setAudioPlaybackCaptureConfig(config)
                .build()
        }.getOrElse {
            Log.w(TAG, "no sound capture", it)
            stream.end()
            return
        }
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            Log.w(TAG, "sound capture didn't start")
            record.release()
            stream.end()
            return
        }
        running = true
        thread = Thread({ run(record) }, "sound-capture").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
    }

    private fun run(record: AudioRecord) {
        try {
            record.startRecording()
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
        }
    }

    fun stop() {
        running = false
        thread = null
    }

    companion object {
        private const val TAG = "SoundCapture"
        private const val RATE = 48_000
        private const val CHANNELS = 2
        /** 10 ms of 16-bit stereo. */
        private const val PACKET_BYTES = RATE / 100 * CHANNELS * 2

        /** Whether this phone can share its sound (Android 10+, with the permission). */
        fun canCapture(context: Context): Boolean =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
    }
}
