// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTimestamp
import android.media.AudioTrack
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.Process
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Plays every PC lane through one low-latency stereo output clock ([LiveAudioMixer]).
 *
 * Legacy `pc.audio.relay` frames (no sequence) share one lane capped at [bufferMs];
 * managed route frames get one lane per PC. Wi-Fi stays in low-latency mode while PC
 * packets arrived in the last [WIFI_WINDOW_MS], and the hardware buffer grows by a
 * block per underrun, up to [MAX_BLOCKS].
 */
class PcSpeakerPlayer(
    private val activity: ActivityWindow = ActivityWindow(ACTIVE_WINDOW_MS),
    /** Monotonic milliseconds. Not SystemClock, so plain JVM tests can play frames. */
    private val clock: () -> Long = { System.nanoTime() / 1_000_000 },
) {
    val mixer = LiveAudioMixer()

    /** Maximum legacy arrival-buffer backlog from the existing PC speaker preference. */
    @Volatile var bufferMs = 40

    private val lock = java.lang.Object()
    /** Null off a device (unit tests), where activity then expires only on [stop]. */
    private val main: Handler? by lazy { runCatching { Handler(Looper.getMainLooper()) }.getOrNull() }
    private var audio: AudioManager? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var policy: PhonePlaybackPolicy? = null
    private var worker: Thread? = null
    private var epoch = 0L
    private var packetSerial = 0L
    private var lastActivityMs = 0L
    /** After an output that couldn't open, packets wait this long before trying again. */
    private var retryAtMs = 0L
    private val recent = linkedMapOf<String, Long>()

    private val _playing = MutableStateFlow(false)
    val playing: StateFlow<Boolean> = _playing.asStateFlow()

    private val _outputLatencyMs = MutableStateFlow<Float?>(null)
    val outputLatencyMs: StateFlow<Float?> = _outputLatencyMs.asStateFlow()

    private val _laneStats = MutableStateFlow<Map<String, LiveAudioMixer.LaneStats>>(emptyMap())
    val laneStats: StateFlow<Map<String, LiveAudioMixer.LaneStats>> = _laneStats.asStateFlow()

    @Volatile
    var activePcId: String? = null
        private set

    private val expireActivity = Runnable {
        synchronized(lock) {
            _playing.value = activity.isActive()
            updateWifiLock(clock())
        }
    }

    /** Attach process services to the existing player before packets arrive. */
    fun attach(context: Context, playbackPolicy: PhonePlaybackPolicy? = PhonePlaybackPolicy.forContext(context)) {
        synchronized(lock) {
            audio = context.getSystemService(AudioManager::class.java)
            policy = playbackPolicy
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && wifiLock == null) {
                wifiLock = runCatching {
                    context.applicationContext.getSystemService(WifiManager::class.java)
                        ?.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "NectarLink:pc-audio")
                        ?.apply { setReferenceCounted(false) }
                }.getOrNull()
                updateWifiLock(clock())
            }
        }
    }

    /** Legacy relay frame; [pcId] is the PC it came from, when known. */
    fun playFrame(sampleRate: Int, channels: Int, pcm: ByteArray, pcId: String? = null) {
        if (sampleRate <= 0 || channels <= 0) return
        enqueue(sampleRate.coerceIn(8_000, 96_000), channels.coerceAtMost(2), pcm, pcId, null)
    }

    /** Managed route frame for [pcId]'s own lane. */
    fun playFrame(sampleRate: Int, channels: Int, pcm: ByteArray, pcId: String, sequence: UInt) {
        enqueue(sampleRate, channels, pcm, pcId, sequence)
    }

    private fun enqueue(sampleRate: Int, channels: Int, pcm: ByteArray, pcId: String?, sequence: UInt?) {
        synchronized(lock) {
            val now = clock()
            val id = pcId ?: LEGACY_LANE
            val cap = if (sequence == null) bufferMs else null
            if (!mixer.push(id, sampleRate, channels, pcm, sequence, now, cap)) return
            recent.remove(id)
            recent[id] = now
            activePcId = recent.keys.lastOrNull { it != LEGACY_LANE }
            lastActivityMs = now
            packetSerial++
            activity.markActive()
            _playing.value = true
            updateWifiLock(now)
            main?.run {
                removeCallbacks(expireActivity)
                postDelayed(expireActivity, ACTIVE_WINDOW_MS)
                postDelayed(expireActivity, WIFI_WINDOW_MS)
            }
            if (now >= retryAtMs) ensureWorker()
            lock.notifyAll()
        }
    }

    /** Stops [pcId]'s lane only; other PCs keep playing. */
    fun stopForPc(pcId: String) {
        synchronized(lock) {
            mixer.removeLane(pcId)
            _laneStats.value = _laneStats.value - pcId
            if (recent.remove(pcId) == null) return
            if (recent.isEmpty()) {
                stopLocked()
            } else {
                activePcId = recent.keys.lastOrNull { it != LEGACY_LANE }
                updateWifiLock(clock())
            }
        }
    }

    /** Stops every lane and releases the output. */
    fun stop() = synchronized(lock) { stopLocked() }

    private fun stopLocked() {
        epoch++
        lastActivityMs = 0
        recent.clear()
        activePcId = null
        retryAtMs = 0L
        activity.reset()
        main?.removeCallbacks(expireActivity)
        _playing.value = false
        _outputLatencyMs.value = null
        _laneStats.value = emptyMap()
        mixer.clear()
        updateWifiLock(clock())
        lock.notifyAll()
    }

    private fun updateWifiLock(now: Long) {
        val wifi = wifiLock ?: return
        val shouldHold = recent.values.any { now - it < WIFI_WINDOW_MS }
        runCatching {
            if (shouldHold && !wifi.isHeld) wifi.acquire()
            if (!shouldHold && wifi.isHeld) wifi.release()
        }
    }

    private fun ensureWorker() {
        if (worker != null) return
        val workerEpoch = epoch
        worker = Thread({ runOutput(workerEpoch) }, "nectarlink-pc-speaker").apply {
            isDaemon = true
            start()
        }
    }

    private fun runOutput(workerEpoch: Long) {
        runCatching { Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO) }
        val (manager, trackPolicy, startedSerial) = synchronized(lock) { Triple(audio, policy, packetSerial) }
        var attemptedSerial = startedSerial
        val rate = manager?.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)
            ?.toIntOrNull()?.takeIf { it in 8_000..96_000 } ?: 48_000
        var block = manager?.getProperty(AudioManager.PROPERTY_OUTPUT_FRAMES_PER_BUFFER)
            ?.toIntOrNull()?.takeIf { it > 0 } ?: 240
        while (block < 96) block *= 2
        while (block > 480) block /= 2

        var track: AudioTrack? = null
        var announced = false
        var opened = false
        try {
            synchronized(lock) {
                while (epoch == workerEpoch && !mixer.hasPending(clock())) {
                    if (clock() - lastActivityMs >= START_IDLE_MS) {
                        opened = true // Idle, not a failure.
                        return
                    }
                    lock.wait(5)
                }
                if (epoch != workerEpoch) return
            }
            track = createTrack(rate, block) ?: return
            opened = true
            var blocks = 2
            track.setBufferSizeInFrames(block * blocks)
            trackPolicy?.beforeTrackStart()
            announced = true
            track.play()
            trackPolicy?.afterTrackStart()

            val pcm = ShortArray(block * 2)
            val timestamp = AudioTimestamp()
            var writtenSamples = 0L
            val startedAt = clock()
            var lastAudio = startedAt
            var lastStats = 0L
            var lastUnderruns = track.underrunCount
            while (true) {
                val now = clock()
                val done = synchronized(lock) {
                    if (epoch != workerEpoch) true else {
                        attemptedSerial = packetSerial
                        if (mixer.render(pcm, rate, now)) lastAudio = now
                        if (now - lastStats >= STATS_INTERVAL_MS) _laneStats.value = mixer.laneStats()
                        now - lastAudio >= INACTIVITY_TIMEOUT_MS && !mixer.hasPending(now)
                    }
                }
                if (done) break

                var offset = 0
                while (offset < pcm.size) {
                    val count = track.write(pcm, offset, pcm.size - offset, AudioTrack.WRITE_BLOCKING)
                    if (count <= 0) return
                    offset += count
                    writtenSamples += count
                }
                val underruns = track.underrunCount
                if (underruns > lastUnderruns && now - startedAt >= 1_000 && blocks < MAX_BLOCKS) {
                    track.setBufferSizeInFrames(block * ++blocks)
                }
                lastUnderruns = underruns
                if (now - lastStats >= STATS_INTERVAL_MS) {
                    lastStats = now
                    val latency = if (track.getTimestamp(timestamp)) {
                        val elapsedFrames = (System.nanoTime() - timestamp.nanoTime) * (rate / 1_000_000_000.0)
                        val aheadFrames = writtenSamples / 2 - timestamp.framePosition - elapsedFrames
                        (aheadFrames.coerceAtLeast(0.0) * 1_000 / rate).toFloat()
                    } else null
                    synchronized(lock) {
                        if (epoch == workerEpoch) _outputLatencyMs.value = latency
                    }
                }
            }
        } catch (_: RuntimeException) {
            // Output routing can disappear between initialization, play and write.
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            track?.let {
                runCatching { it.pause() }
                runCatching { it.flush() }
                runCatching { it.release() }
            }
            if (announced) trackPolicy?.afterTrackRelease()
            synchronized(lock) {
                worker = null
                _outputLatencyMs.value = null
                val now = clock()
                if (!opened && workerEpoch == epoch) {
                    // No output device right now: retry on a later packet, not on every one.
                    retryAtMs = now + RETRY_MS
                } else if ((workerEpoch != epoch || attemptedSerial != packetSerial) && mixer.hasPending(now)) {
                    ensureWorker()
                }
                updateWifiLock(now)
            }
        }
    }

    private fun createTrack(rate: Int, block: Int): AudioTrack? {
        val channels = AudioFormat.CHANNEL_OUT_STEREO
        val encoding = AudioFormat.ENCODING_PCM_16BIT
        val minimum = AudioTrack.getMinBufferSize(rate, channels, encoding)
        if (minimum <= 0) return null
        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    setAllowedCapturePolicy(AudioAttributes.ALLOW_CAPTURE_BY_SYSTEM)
                }
            }
            .build()
        val format = AudioFormat.Builder()
            .setSampleRate(rate)
            .setChannelMask(channels)
            .setEncoding(encoding)
            .build()
        return AudioTrack.Builder()
            .setAudioAttributes(attributes)
            .setAudioFormat(format)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .setBufferSizeInBytes(maxOf(minimum, block * 4 * MAX_BLOCKS))
            .build()
            .let { track ->
                if (track.state == AudioTrack.STATE_INITIALIZED) track else {
                    track.release()
                    null
                }
            }
    }

    companion object {
        const val ACTIVE_WINDOW_MS = 3_000L
        private const val WIFI_WINDOW_MS = 5_000L
        private const val INACTIVITY_TIMEOUT_MS = 120_000L
        private const val START_IDLE_MS = 200L
        private const val STATS_INTERVAL_MS = 200L
        private const val RETRY_MS = 500L
        private const val MAX_BLOCKS = 8
        private const val LEGACY_LANE = ""
    }
}
