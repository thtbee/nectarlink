// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import kotlin.math.abs
import kotlin.math.roundToInt

/** PCM16LE inputs, one stereo output clock. Calls may come from different threads. */
class LiveAudioMixer {
    enum class LatencyPreset(val marginMs: Int) { LOWEST(4), BALANCED(12), STABLE(35) }

    data class LaneStats(
        val peak: Float,
        val bufferMs: Float,
        val extraMarginMs: Float,
        val rateCorrection: Double,
        val skippedFrames: Long,
        val underruns: Int,
        val lostPackets: Long,
        val live: Boolean,
    )

    private val lanes = mutableMapOf<String, Lane>()
    private val settings = mutableMapOf<String, LaneSettings>()
    private var mix = FloatArray(0)
    private var outputGain = 1f

    @Volatile var latencyPreset = LatencyPreset.BALANCED
    @Volatile var masterGain = 1f
    @Volatile var masterMuted = false
    @Volatile var policyGain = 1f

    @Synchronized
    fun setLaneGain(sourceId: String, slider: Float, muted: Boolean = false) {
        settings[sourceId] = LaneSettings(sliderGain(slider), muted)
    }

    /** Returns true when a valid, newer packet was appended. */
    @Synchronized
    fun push(
        sourceId: String,
        sampleRate: Int,
        channels: Int,
        pcm: ByteArray,
        sequence: UInt? = null,
        nowMs: Long = monotonicMs(),
        backlogMs: Int? = null,
    ): Boolean {
        if (sampleRate !in 8_000..96_000 || channels !in 1..2 || pcm.isEmpty() ||
            pcm.size % (channels * 2) != 0
        ) return false

        val lane = lanes.getOrPut(sourceId) { Lane(sampleRate) }
        return lane.push(sampleRate, channels, pcm, sequence, nowMs, backlogMs)
    }

    /** Drops one source's buffered audio. Its gain and mute stay, since routing owns them. */
    @Synchronized
    fun removeLane(sourceId: String) {
        lanes.remove(sourceId)
    }

    /** Drops a source's gain and mute as well, for sources that left the route for good. */
    @Synchronized
    fun forgetLane(sourceId: String) {
        lanes.remove(sourceId)
        settings.remove(sourceId)
    }

    @Synchronized
    fun clear() {
        lanes.clear()
        outputGain = 1f
    }

    @Synchronized
    fun hasPending(nowMs: Long = monotonicMs()): Boolean =
        lanes.values.any { nowMs - it.lastAudioMs < 200 && it.writePos > it.readPos }

    @Synchronized
    fun laneStats(): Map<String, LaneStats> = lanes.mapValues { (_, lane) ->
        LaneStats(lane.peak, lane.bufferMs, lane.extraMarginMs, lane.correction, lane.skippedFrames,
            lane.underruns, lane.lostPackets, lane.playing)
    }

    /** Writes interleaved stereo PCM. Returns true if at least one lane advanced. */
    @Synchronized
    fun render(pcm: ShortArray, outputRate: Int, nowMs: Long = monotonicMs()): Boolean {
        require(pcm.size % 2 == 0 && outputRate > 0)
        if (pcm.isEmpty()) return false
        if (mix.size != pcm.size) mix = FloatArray(pcm.size) else mix.fill(0f)
        val frames = pcm.size / 2
        val targetMs = latencyPreset.marginMs
        var any = false
        for ((id, lane) in lanes) {
            val laneSettings = settings[id] ?: LaneSettings.Default
            val gain = if (laneSettings.muted) 0f else laneSettings.gain
            if (lane.render(mix, frames, outputRate, targetMs, gain, nowMs)) any = true
        }

        val targetGain = if (masterMuted) 0f else masterGain.unitGain() * policyGain.unitGain()
        val delta = (targetGain - outputGain) / frames
        for (frame in 0 until frames) {
            outputGain += delta
            val sample = frame * 2
            pcm[sample] = toPcm(mix[sample] * outputGain)
            pcm[sample + 1] = toPcm(mix[sample + 1] * outputGain)
        }
        outputGain = targetGain
        return any
    }

    private data class LaneSettings(val gain: Float, val muted: Boolean) {
        companion object { val Default = LaneSettings(1f, false) }
    }

    private class Lane(private var rate: Int) {
        private val ring = FloatArray(RING_FRAMES * 2)
        var writePos = 0L
            private set
        var readPos = 0.0
            private set
        var lastAudioMs = Long.MIN_VALUE / 2
            private set
        private var lastSequence: UInt? = null
        private var streamStartMs = 0L
        private var starvedAtMs: Long? = null

        var playing = false
            private set
        private var gain = 0f
        private var averageFill = 0.0
        private var windowMin = Double.POSITIVE_INFINITY
        private var previousMin: Double? = null
        private var correctionMin: Double? = null
        private var windowFrames = 0

        var extraMarginMs = 0f
            private set
        var correction = 0.0
            private set
        var peak = 0f
            private set
        var bufferMs = 0f
            private set
        var skippedFrames = 0L
            private set
        var underruns = 0
            private set
        var lostPackets = 0L
            private set

        fun push(
            sampleRate: Int,
            channels: Int,
            pcm: ByteArray,
            sequence: UInt?,
            nowMs: Long,
            backlogMs: Int?,
        ): Boolean {
            val gapMs = nowMs - lastAudioMs
            val frames = pcm.size / (channels * 2)
            if (sampleRate != rate || gapMs > 1_000) {
                rate = sampleRate
                restart(nowMs)
            }

            val previous = lastSequence
            if (sequence != null && previous != null) {
                // Subtraction modulo 2^32 preserves short distances across wrap.
                val distance = (sequence - previous).toInt()
                if (distance < -RESTART_DISTANCE) {
                    // The sender restarted its numbering (a new capture or route).
                    restart(nowMs)
                } else if (distance <= 0) {
                    return false
                } else if (distance in 2..MAX_FILL_PACKETS && gapMs < 100) {
                    val silenceFrames = (distance - 1L) * frames
                    val end = writePos + silenceFrames
                    for (position in maxOf(writePos, end - RING_FRAMES) until end) {
                        val i = index(position)
                        ring[i] = 0f
                        ring[i + 1] = 0f
                    }
                    writePos = end
                    lostPackets += distance - 1
                }
            }
            lastSequence = sequence

            // Packets larger than the ring retain their most recent frames.
            val first = (frames - RING_FRAMES).coerceAtLeast(0)
            var offset = first * channels * 2
            for (frame in first until frames) {
                val i = index(writePos + frame)
                val left = sample(pcm, offset)
                ring[i] = left
                ring[i + 1] = if (channels == 2) sample(pcm, offset + 2) else left
                offset += channels * 2
            }
            writePos += frames
            val cap = backlogMs?.let { (rate.toLong() * it.coerceIn(1, 1_000) / 1_000).toInt() } ?: RING_FRAMES
            readPos = maxOf(readPos, (writePos - cap.coerceIn(1, RING_FRAMES)).toDouble())

            val starved = starvedAtMs
            if (starved != null && nowMs - starved < 80 && nowMs - streamStartMs > 1_500) {
                extraMarginMs = (extraMarginMs + 10f).coerceAtMost(150f)
                underruns++
            }
            starvedAtMs = null
            lastAudioMs = nowMs
            return true
        }

        private fun restart(nowMs: Long) {
            readPos = writePos.toDouble()
            playing = false
            lastSequence = null
            streamStartMs = nowMs
            starvedAtMs = null
        }

        fun render(
            out: FloatArray,
            frames: Int,
            outRate: Int,
            targetMs: Int,
            targetGain: Float,
            nowMs: Long,
        ): Boolean {
            // A block is read whole, so the buffer can't run leaner than one block (plus 1 ms)
            // without starving. LOWEST asks for less than that with 5 ms blocks.
            val floor = frames.toDouble() * rate / outRate + 2 + rate / 1_000.0
            val target = maxOf((targetMs + extraMarginMs) * rate / 1_000.0, floor)
            if (!playing) {
                if (nowMs - lastAudioMs >= 100 || writePos - readPos < target) {
                    peak *= 0.86f
                    bufferMs = 0f
                    return false
                }
                val start = minOf(writePos - readPos, target + rate * 0.015)
                readPos = writePos - start
                averageFill = start
                playing = true
                gain = 0f
                correction = 0.0
                correctionMin = null
                previousMin = null
                windowMin = Double.POSITIVE_INFINITY
                windowFrames = 0
            }

            var fill = writePos - readPos
            windowMin = minOf(windowMin, fill)
            windowFrames += frames
            if (windowFrames >= outRate / 2) {
                correctionMin = previousMin?.let { minOf(it, windowMin) }
                previousMin = windowMin
                windowMin = Double.POSITIVE_INFINITY
                windowFrames -= outRate / 2
                val minimum = correctionMin
                if (minimum != null && minimum - target > rate * 0.03) {
                    val skip = minimum - target
                    readPos += skip
                    fill -= skip
                    skippedFrames += skip.toLong()
                    correctionMin = target
                    previousMin = target
                }
            }
            averageFill += (fill - averageFill) * 0.05
            correction = correctionMin?.let { (0.3 * (it - target) / rate).coerceIn(-0.004, 0.008) } ?: 0.0
            val step = rate.toDouble() / outRate * (1.0 + correction)
            val under = fill < frames * step + 2
            val count = if (under) ((fill - 2) / step).toInt().coerceIn(0, frames) else frames
            val end = if (under) 0f else targetGain
            val delta = (end - gain) / count.coerceAtLeast(1)
            var measured = 0f
            for (frame in 0 until count) {
                val whole = readPos.toLong()
                val fraction = (readPos - whole).toFloat()
                val a = index(whole)
                val b = index(whole + 1)
                gain += delta
                val left = (ring[a] + (ring[b] - ring[a]) * fraction) * gain
                val right = (ring[a + 1] + (ring[b + 1] - ring[a + 1]) * fraction) * gain
                out[frame * 2] += left
                out[frame * 2 + 1] += right
                measured = maxOf(measured, abs(left), abs(right))
                readPos += step
            }
            gain = end
            if (under) {
                playing = false
                starvedAtMs = nowMs
            } else {
                extraMarginMs = (extraMarginMs - frames * (1_000f / outRate) / 15_000f).coerceAtLeast(0f)
            }
            peak = maxOf(measured, peak * 0.86f).coerceAtMost(1f)
            bufferMs = (averageFill * 1_000 / rate).toFloat()
            return count > 0
        }
    }

    companion object {
        private const val RING_FRAMES = 65_536
        private const val MAX_FILL_PACKETS = 8
        private const val RESTART_DISTANCE = 256

        private fun monotonicMs() = System.nanoTime() / 1_000_000
        private fun index(frame: Long) = (frame.toInt() and (RING_FRAMES - 1)) * 2
        private fun sample(bytes: ByteArray, offset: Int): Float =
            ((bytes[offset].toInt() and 0xff) or (bytes[offset + 1].toInt() shl 8)).toShort() / 32768f

        private fun toPcm(sample: Float): Short =
            (sample * 32768f).roundToInt().coerceIn(-32768, 32767).toShort()

        private fun Float.unitGain() = if (isFinite()) coerceIn(0f, 1f) else 0f

        fun sliderGain(slider: Float): Float = slider.unitGain().let { it * it }
    }
}
