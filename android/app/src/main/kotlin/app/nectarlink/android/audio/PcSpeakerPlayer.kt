// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.media.audiofx.LoudnessEnhancer
import android.os.Build
import android.os.Process
import android.util.Log
import java.util.concurrent.LinkedBlockingDeque
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Plays live PCM-16LE audio frames streamed from a paired PC (`pc.audio.relay`)
 * on a low-latency output (`PERFORMANCE_MODE_LOW_LATENCY`), while a
 * dedicated `THREAD_PRIORITY_URGENT_AUDIO` worker maintains a tight ~35–45 ms
 * jitter cushion without ever calling `pause()`/`flush()` mid-stream.
 */
class PcSpeakerPlayer(
    private val activity: ActivityWindow = ActivityWindow(ACTIVE_WINDOW_MS),
) {
    private class AudioChunk(
        val sampleRate: Int,
        val channels: Int,
        val pcm: ByteArray,
    )

    private val queue = LinkedBlockingDeque<AudioChunk>(MAX_QUEUE_CHUNKS)
    private val workerRunning = AtomicBoolean(false)
    private val watchdogRunning = AtomicBoolean(false)
    @Volatile
    private var watchdogThread: Thread? = null

    private val _playing = MutableStateFlow(false)

    /** True while audio frames arrived within the trailing [ACTIVE_WINDOW_MS]. */
    val playing: StateFlow<Boolean> = _playing.asStateFlow()

    @Volatile
    private var queuedBytes: Int = 0

    @Volatile
    var activePcId: String? = null
        private set

    fun playFrame(sampleRate: Int, channels: Int, pcm: ByteArray, pcId: String? = null) {
        if (pcm.isEmpty() || sampleRate <= 0 || channels <= 0) return
        if (pcId != null) {
            activePcId = pcId
        }
        activity.markActive()
        if (!_playing.value) {
            _playing.value = true
        }
        ensureWatchdog()

        val safeRate = sampleRate.coerceIn(8_000, 96_000)
        val safeChannels = if (channels >= 2) 2 else 1
        val frameBytes = safeChannels * 2
        // Cap queued backlog at ~40 ms so Wi-Fi bursts never build up latency
        // and we never need to flush the hardware AudioTrack.
        val maxQueuedBytes = (safeRate * frameBytes * 40) / 1000

        synchronized(queue) {
            while (queuedBytes + pcm.size > maxQueuedBytes || queue.size >= MAX_QUEUE_CHUNKS - 1) {
                val dropped = queue.pollFirst() ?: break
                queuedBytes = (queuedBytes - dropped.pcm.size).coerceAtLeast(0)
            }
            if (queue.offerLast(AudioChunk(safeRate, safeChannels, pcm))) {
                queuedBytes += pcm.size
            }
        }

        if (workerRunning.compareAndSet(false, true)) {
            Thread(::runAudioLoop, "nectarlink-pc-speaker").apply {
                isDaemon = true
                start()
            }
        }
    }

    fun stopForPc(pcId: String) {
        if (activePcId == pcId) {
            stop()
        }
    }

    fun stop() {
        activePcId = null
        activity.reset()
        _playing.value = false
        watchdogThread?.interrupt()
        synchronized(queue) {
            queue.clear()
            queuedBytes = 0
            // Poison pill with rate = 0 signals immediate stop.
            queue.offerFirst(AudioChunk(0, 0, ByteArray(0)))
        }
    }

    private fun ensureWatchdog() {
        if (watchdogRunning.compareAndSet(false, true)) {
            Thread(::runWatchdogLoop, "nectarlink-pc-speaker-watchdog").apply {
                isDaemon = true
                start()
            }
        }
    }

    private fun runWatchdogLoop() {
        watchdogThread = Thread.currentThread()
        try {
            while (true) {
                Thread.sleep(WATCHDOG_CHECK_INTERVAL_MS)
                if (!activity.isActive()) {
                    _playing.value = false
                    break
                }
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            _playing.value = false
        } finally {
            watchdogThread = null
            watchdogRunning.set(false)
            if (activity.isActive() && watchdogRunning.compareAndSet(false, true)) {
                _playing.value = true
                Thread(::runWatchdogLoop, "nectarlink-pc-speaker-watchdog").apply {
                    isDaemon = true
                    start()
                }
            }
        }
    }

    private fun pollChunk(timeoutMs: Long): AudioChunk? {
        val chunk = if (timeoutMs <= 0L) {
            queue.pollFirst()
        } else {
            queue.pollFirst(timeoutMs, TimeUnit.MILLISECONDS)
        }
        if (chunk != null && chunk.pcm.isNotEmpty()) {
            synchronized(queue) {
                queuedBytes = (queuedBytes - chunk.pcm.size).coerceAtLeast(0)
            }
        }
        return chunk
    }

    private fun runAudioLoop() {
        runCatching {
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        }

        var track: AudioTrack? = null
        var enhancer: LoudnessEnhancer? = null
        var currentRate = 0
        var currentChannels = 0
        var needsPrebuffer = true

        try {
            while (true) {
                val first = pollChunk(INACTIVITY_TIMEOUT_MS) ?: break
                if (first.sampleRate == 0) break

                if (track == null ||
                    track.state != AudioTrack.STATE_INITIALIZED ||
                    currentRate != first.sampleRate ||
                    currentChannels != first.channels
                ) {
                    releaseTrack(track, enhancer)
                    track = createTrack(first.sampleRate, first.channels)
                    enhancer = track?.let { createEnhancer(it.audioSessionId) }
                    currentRate = first.sampleRate
                    currentChannels = first.channels
                    needsPrebuffer = true
                }

                val activeTrack = track ?: continue

                if (needsPrebuffer) {
                    // Wait up to 8 ms to accumulate a ~10 ms pre-buffer cushion on stream start
                    // so minor Wi-Fi jitter never causes an initial underrun while keeping latency minimal.
                    val frameBytes = first.channels * 2
                    val prebufferTarget = (first.sampleRate * frameBytes * 10) / 1000
                    var waitedMs = 0
                    while (queuedBytes + first.pcm.size < prebufferTarget && waitedMs < 8) {
                        Thread.sleep(2)
                        waitedMs += 2
                    }
                    needsPrebuffer = false
                }

                if (activeTrack.playState != AudioTrack.PLAYSTATE_PLAYING) {
                    runCatching { activeTrack.play() }
                }

                val startedWriteNs = System.nanoTime()
                runCatching {
                    activeTrack.write(first.pcm, 0, first.pcm.size, AudioTrack.WRITE_BLOCKING)
                }.onFailure {
                    Log.w(TAG, "AudioTrack write failed", it)
                }
                val writeMs = (System.nanoTime() - startedWriteNs) / 1_000_000L
                if (writeMs > 25L) {
                    Log.d(TAG, "AudioTrack write blocked ${writeMs}ms (queuedBytes=$queuedBytes)")
                }
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            workerRunning.set(false)
            releaseTrack(track, enhancer)
            // If new frames arrived just as the loop exited, restart the worker.
            val hasPending = synchronized(queue) {
                queue.peekFirst()?.let { it.sampleRate > 0 } == true
            }
            if (hasPending && workerRunning.compareAndSet(false, true)) {
                Thread(::runAudioLoop, "nectarlink-pc-speaker").apply {
                    isDaemon = true
                    start()
                }
            }
        }
    }

    private fun createTrack(sampleRate: Int, channels: Int): AudioTrack? {
        val channelMask = if (channels == 1) {
            AudioFormat.CHANNEL_OUT_MONO
        } else {
            AudioFormat.CHANNEL_OUT_STEREO
        }
        val minBuf = AudioTrack.getMinBufferSize(
            sampleRate,
            channelMask,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minBuf <= 0) {
            Log.w(TAG, "invalid minBufferSize=$minBuf for rate=$sampleRate channels=$channels")
            return null
        }
        val frameBytes = channels * 2
        // ~24 ms hardware buffer floor balances low latency with Wi-Fi jitter resilience.
        val targetBuf = minBuf.coerceAtLeast((sampleRate * frameBytes * 24) / 1000)

        return runCatching {
            val attrs = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .apply {
                    // The PC's own sound must not go back to the PC with the
                    // phone's sound or a mirror, or it plays twice and can loop.
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        setAllowedCapturePolicy(AudioAttributes.ALLOW_CAPTURE_BY_SYSTEM)
                    }
                }
                .build()
            val format = AudioFormat.Builder()
                .setSampleRate(sampleRate)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setChannelMask(channelMask)
                .build()
            AudioTrack.Builder()
                .setAudioAttributes(attrs)
                .setAudioFormat(format)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(targetBuf)
                .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                .build()
        }.getOrElse {
            Log.w(TAG, "failed to create AudioTrack", it)
            null
        }
    }

    private fun createEnhancer(audioSessionId: Int): LoudnessEnhancer? {
        if (audioSessionId == AudioTrack.ERROR_BAD_VALUE || audioSessionId == 0) return null
        return runCatching {
            LoudnessEnhancer(audioSessionId).apply {
                // Subtle +1.5 dB (150 mB) presence lift for phone speakers.
                setTargetGain(150)
                enabled = true
            }
        }.getOrNull()
    }

    private fun releaseTrack(track: AudioTrack?, enhancer: LoudnessEnhancer?) {
        runCatching { enhancer?.release() }
        if (track == null) return
        runCatching {
            if (track.playState == AudioTrack.PLAYSTATE_PLAYING) {
                track.stop()
            }
        }
        runCatching { track.release() }
    }

    companion object {
        private const val TAG = "PcSpeakerPlayer"
        private const val INACTIVITY_TIMEOUT_MS = 30_000L
        private const val MAX_QUEUE_CHUNKS = 96
        const val ACTIVE_WINDOW_MS = 3_000L
        private const val WATCHDOG_CHECK_INTERVAL_MS = 1_000L
    }
}
