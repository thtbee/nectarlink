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
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import androidx.core.content.ContextCompat
import app.nectarlink.android.audio.PhonePlaybackPolicy
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.ElevatedAudioStream
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig
import app.nectarlink.services.client.NectarlinkServiceContract
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.abs

/**
 * The phone's sound: what apps play (media, games, and apps that don't say), through
 * Android 10+ [AudioPlaybackCaptureConfiguration] or the Elevated helper's internal
 * submix, as 48 kHz stereo PCM 16.
 *
 * Mirroring ([SoundCapture] with a [MirrorStream]) sends 10 ms packets on that stream.
 * A managed route ([managed]) hands 5 ms packets and a level to its callbacks. Either
 * way, [stop] unblocks the read and [onEnded] runs once, from the capture thread when
 * one ran.
 */
class SoundCapture private constructor(
    private val projection: MediaProjection?,
    private val stream: MirrorStream?,
    private val managed: Managed?,
    private val onEnded: () -> Unit,
) {
    constructor(projection: MediaProjection?, stream: MirrorStream, onEnded: () -> Unit = {}) :
        this(projection, stream, null, onEnded)

    private data class Managed(
        val context: Context,
        val quietPhone: Boolean,
        val phonePlaybackActive: () -> Boolean,
        val onPcm: (Int, Int, ByteArray) -> Boolean,
        val onPeak: (Float) -> Unit,
    )

    private val lock = Any()
    private val started = AtomicBoolean(false)
    private val ended = AtomicBoolean(false)
    @Volatile private var running = false
    /** A capture thread owns teardown, so [onEnded] comes after its last packet. */
    @Volatile private var threadOwnsEnd = false
    private var record: AudioRecord? = null
    private var elevatedSocket: ElevatedAudioStream? = null
    private var quiet: PhoneQuiet.Session? = null
    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() = stop()
    }

    @Volatile
    var usesPlaybackCapture = false
        private set

    @Volatile
    var sentSound = false
        private set

    fun start() {
        if (!started.compareAndSet(false, true)) return
        synchronized(lock) {
            if (ended.get()) return
            val proj = projection
            if (proj != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val recorder = runCatching {
                    proj.registerCallback(projectionCallback, Handler(Looper.getMainLooper()))
                    createRecord(proj)
                }.getOrNull()
                if (recorder != null && recorder.state == AudioRecord.STATE_INITIALIZED) {
                    record = recorder
                    running = true
                    usesPlaybackCapture = true
                    runCaptureThread { runRecord(recorder) }
                    return
                }
                recorder?.release()
                runCatching { proj.unregisterCallback(projectionCallback) }
            }
            if (Elevated.hasCapability(NectarlinkServiceContract.CAP_DEVICE_AUDIO)) {
                running = true
                runCaptureThread(::runElevated)
                return
            }
        }
        finishCapture()
    }

    @SuppressLint("MissingPermission")
    @androidx.annotation.RequiresApi(Build.VERSION_CODES.Q)
    private fun createRecord(projection: MediaProjection): AudioRecord {
        val config = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val channelMask = AudioFormat.CHANNEL_IN_STEREO
        val encoding = AudioFormat.ENCODING_PCM_16BIT
        val format = AudioFormat.Builder()
            .setEncoding(encoding)
            .setSampleRate(RATE)
            .setChannelMask(channelMask)
            .build()
        val minimum = AudioRecord.getMinBufferSize(RATE, channelMask, encoding)
        return AudioRecord.Builder()
            .setAudioFormat(format)
            .setBufferSizeInBytes(maxOf(minimum, MIRROR_PACKET_BYTES * 8))
            .setAudioPlaybackCaptureConfig(config)
            .build()
    }

    private fun runCaptureThread(capture: () -> Unit) {
        threadOwnsEnd = true
        Thread({
            runCatching { Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO) }
            capture()
        }, "nectarlink-sound-capture").start()
    }

    private fun runRecord(recorder: AudioRecord) {
        try {
            synchronized(lock) {
                if (!running) return
                recorder.startRecording()
                val capture = managed
                if (capture?.quietPhone == true) quiet = PhoneQuiet.begin(capture.context, capture.phonePlaybackActive)
            }
            // A blocking read returns 0 only when stopped; the loop then sees `running`.
            pump { packet, filled -> recorder.read(packet, filled, packet.size - filled) }
        } catch (_: Exception) {
            // Projection revocation or a lost recording route ends the session.
        } finally {
            synchronized(lock) {
                record = null
                runCatching { recorder.stop() }
                recorder.release()
            }
            finishCapture()
        }
    }

    private fun runElevated() {
        var opened: ElevatedAudioStream? = null
        try {
            val socket = Elevated.openAudioStream() ?: return
            opened = socket
            synchronized(lock) {
                if (!running) return
                elevatedSocket = socket
            }
            val input = socket.inputStream
            pump { packet, filled -> input.read(packet, filled, packet.size - filled).let { if (it == 0) -1 else it } }
        } catch (_: Exception) {
            // Closing the helper socket interrupts a blocked read.
        } finally {
            synchronized(lock) { elevatedSocket = null }
            runCatching { opened?.close() }
            finishCapture()
        }
    }

    private fun pump(read: (ByteArray, Int) -> Int) {
        val capture = managed
        val packet = ByteArray(if (capture == null) MIRROR_PACKET_BYTES else MANAGED_PACKET_BYTES)
        val mirror = stream
        if (mirror?.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(RATE.toUInt(), CHANNELS.toUByte())) ==
            MirrorSendResult.CLOSED
        ) return
        val started = SystemClock.elapsedRealtimeNanos()
        var lastMeter = 0L
        var peak = 0f
        while (running) {
            var filled = 0
            while (filled < packet.size && running) {
                val count = read(packet, filled)
                if (count < 0) return
                if (count == 0) Thread.sleep(2) // Never spin on an idle recorder.
                filled += count
            }
            if (!running) return
            sentSound = true
            if (usesPlaybackCapture) {
                if (capture == null) PhoneQuiet.observeLegacyPcm(packet)
                else synchronized(lock) { quiet }?.observePcm(packet)
            }
            if (capture != null) {
                val now = SystemClock.elapsedRealtime()
                peak *= 0.86f
                for (i in packet.indices step 2) {
                    val sample = ((packet[i].toInt() and 0xff) or (packet[i + 1].toInt() shl 8)).toShort()
                    peak = maxOf(peak, abs(sample.toInt()) / 32768f)
                }
                if (now - lastMeter >= 50) {
                    capture.onPeak(peak)
                    lastMeter = now
                }
                if (!capture.onPcm(RATE, CHANNELS, packet.copyOf())) return
            } else {
                val time = ((SystemClock.elapsedRealtimeNanos() - started) / 1_000).toULong()
                if (mirror?.send(VideoPacketKind.FRAME, time, packet.copyOf()) == MirrorSendResult.CLOSED) return
            }
        }
    }

    fun stop() {
        synchronized(lock) {
            running = false
            // Stopping the recorder or closing the socket unblocks the capture thread's read.
            runCatching { record?.stop() }
            runCatching { elevatedSocket?.close() }
        }
        if (!threadOwnsEnd || !started.get()) finishCapture()
    }

    private fun finishCapture() {
        if (!ended.compareAndSet(false, true)) return
        running = false
        synchronized(lock) {
            quiet?.close()
            quiet = null
        }
        runCatching { projection?.unregisterCallback(projectionCallback) }
        stream?.end()
        managed?.onPeak?.invoke(0f)
        onEnded()
    }

    companion object {
        private const val RATE = 48_000
        private const val CHANNELS = 2
        private const val MANAGED_PACKET_BYTES = RATE / 200 * CHANNELS * 2
        private const val MIRROR_PACKET_BYTES = MANAGED_PACKET_BYTES * 2

        /** Return false from [onPcm] when the route has ended. Each callback owns its PCM bytes. */
        fun managed(
            context: Context,
            projection: MediaProjection?,
            quietPhone: Boolean = false,
            phonePlaybackActive: () -> Boolean = {
                PhonePlaybackPolicy.forContext(context)?.phonePlaybackActive?.value == true
            },
            onPcm: (sampleRate: Int, channels: Int, pcm: ByteArray) -> Boolean,
            onPeak: (Float) -> Unit = {},
            onEnded: () -> Unit = {},
        ): SoundCapture = SoundCapture(
            projection, null,
            Managed(context.applicationContext, quietPhone, phonePlaybackActive, onPcm, onPeak),
            onEnded,
        )

        fun canCapture(context: Context): Boolean =
            Elevated.hasCapability(NectarlinkServiceContract.CAP_DEVICE_AUDIO) ||
                (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                    ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED)
    }
}
