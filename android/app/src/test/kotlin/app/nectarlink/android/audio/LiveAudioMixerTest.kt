// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import app.nectarlink.android.mic.MicPcm
import app.nectarlink.android.mic.MicStallWatch
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.roundToInt

class LiveAudioMixerTest {
    private val rate = 48_000
    private val block = 240 // 5 ms at 48 kHz

    @After
    fun resetRouting() = AudioRouting.reset()

    // --- Drift ---------------------------------------------------------------------------

    @Test
    fun twoIndependentClocksConvergeWithoutUnderruns() {
        val mixer = LiveAudioMixer()
        val fast = 1.0 + 200e-6
        val slow = 1.0 - 200e-6
        // Each sender's 5 ms packet arrives a little early (fast) or late (slow) on the phone clock.
        val senders = mutableListOf(Sender("fast", 5.0 / fast), Sender("slow", 5.0 / slow))
        var renderAt = 0.0
        val out = ShortArray(block * 2)
        val corrections = mapOf("fast" to mutableListOf<Double>(), "slow" to mutableListOf())
        val buffers = mapOf("fast" to mutableListOf<Float>(), "slow" to mutableListOf())

        val endMs = 120_000.0
        while (renderAt < endMs) {
            val next = senders.minBy { it.nextMs }
            if (next.nextMs <= renderAt) {
                mixer.push(next.id, rate, 2, tone(block, 0.1f), next.seq++, next.nextMs.toLong())
                next.nextMs += next.intervalMs
                continue
            }
            mixer.render(out, rate, renderAt.toLong())
            if (renderAt > 60_000) {
                val stats = mixer.laneStats()
                for (id in corrections.keys) {
                    corrections.getValue(id) += stats.getValue(id).rateCorrection
                    buffers.getValue(id) += stats.getValue(id).bufferMs
                }
            }
            renderAt += 5.0
        }

        val stats = mixer.laneStats()
        for (id in listOf("fast", "slow")) {
            assertEquals("$id underruns", 0, stats.getValue(id).underruns)
            assertEquals("$id skipped", 0L, stats.getValue(id).skippedFrames)
            assertTrue("$id live", stats.getValue(id).live)
            val bufferRange = buffers.getValue(id)
            assertTrue("$id buffer ${bufferRange.min()}..${bufferRange.max()}",
                bufferRange.min() > 5f && bufferRange.max() < 30f)
        }
        // The fast sender is read faster (+200 ppm), the slow one slower (-200 ppm).
        assertEquals(200e-6, corrections.getValue("fast").average(), 80e-6)
        assertEquals(-200e-6, corrections.getValue("slow").average(), 80e-6)
    }

    // --- Sequence handling ---------------------------------------------------------------

    @Test
    fun sequenceGapAcrossWrapIsFilledWithSilence() {
        val mixer = LiveAudioMixer()
        val max = UInt.MAX_VALUE
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), max - 1u, 0))
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), max, 5))
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 0u, 10))
        // 1 was lost: 2 still arrives on time, and the mixer fills one packet of silence.
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 2u, 15))
        assertEquals(1L, mixer.laneStats().getValue("pc").lostPackets)

        // Duplicates and late arrivals are dropped.
        assertFalse(mixer.push("pc", rate, 2, tone(block, 0.9f), 2u, 16))
        assertFalse(mixer.push("pc", rate, 2, tone(block, 0.9f), 1u, 16))
        assertFalse(mixer.push("pc", rate, 2, tone(block, 0.9f), max, 16))

        val ramp = ShortArray(16 * 2)
        assertTrue(mixer.render(ramp, rate, 16))
        val out = ShortArray(1_000 * 2)
        assertTrue(mixer.render(out, rate, 16))
        // Ring frames 16.. follow the ramp block; silence sits at ring frames 720..959.
        val half = (0.5f * 32768).roundToInt().toShort()
        for (frame in 0 until 1_000) {
            val ringFrame = frame + 16
            val expected = if (ringFrame in 720 until 960) 0 else half
            assertEquals("frame $ringFrame", expected, out[frame * 2])
            assertEquals("frame $ringFrame", expected, out[frame * 2 + 1])
        }
    }

    @Test
    fun senderRestartIsAcceptedImmediately() {
        val mixer = LiveAudioMixer()
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 1_000u, 0))
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 1_001u, 5))
        // A new capture numbers from 0 again; it is not treated as 1000 late packets.
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 0u, 10))
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 1u, 15))
        assertEquals(0L, mixer.laneStats().getValue("pc").lostPackets)
    }

    @Test
    fun longGapsAreNotFilled() {
        val mixer = LiveAudioMixer()
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 1u, 0))
        // Too many missing packets: no silence fill.
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 20u, 5))
        // Too late since the last packet: no silence fill either.
        assertTrue(mixer.push("pc", rate, 2, tone(block, 0.5f), 22u, 200))
        assertEquals(0L, mixer.laneStats().getValue("pc").lostPackets)
    }

    // --- Forward skip --------------------------------------------------------------------

    @Test
    fun burstLongerThanThirtyMillisecondsIsSkipped() {
        val mixer = LiveAudioMixer()
        val sim = Steady(mixer, "pc")
        sim.run(2_000)
        assertEquals(0L, mixer.laneStats().getValue("pc").skippedFrames)

        // A Wi-Fi stall delivers 100 ms late, all at once.
        repeat(20) { sim.pushNow() }
        sim.run(3_000)

        val stats = mixer.laneStats().getValue("pc")
        assertTrue("skipped ${stats.skippedFrames}", stats.skippedFrames >= rate * 30 / 1_000)
        assertEquals(0, stats.underruns)
        assertTrue("buffer ${stats.bufferMs}", stats.bufferMs < 30f)
    }

    @Test
    fun jitterBelowThirtyMillisecondsIsNotSkipped() {
        val mixer = LiveAudioMixer()
        val sim = Steady(mixer, "pc")
        sim.run(2_000)
        repeat(3) { sim.pushNow() } // 15 ms early
        sim.run(3_000)
        assertEquals(0L, mixer.laneStats().getValue("pc").skippedFrames)
    }

    // --- Underrun backoff ----------------------------------------------------------------

    @Test
    fun midStreamUnderrunsRaiseMarginToCapThenDecay() {
        val mixer = LiveAudioMixer()
        val sim = Steady(mixer, "pc")
        sim.run(2_000)
        assertEquals(0f, mixer.laneStats().getValue("pc").extraMarginMs, 0f)

        for (n in 1..18) {
            sim.starve()
            sim.run(1_000)
            val extra = mixer.laneStats().getValue("pc").extraMarginMs
            val expected = minOf(10f * n, 150f)
            assertTrue("after $n: $extra", extra <= expected && extra > expected - 2f)
        }
        val stats = mixer.laneStats().getValue("pc")
        assertEquals(18, stats.underruns)
        assertTrue(stats.extraMarginMs <= 150f)
        assertTrue("buffer follows margin: ${stats.bufferMs}", stats.bufferMs > 100f)

        // 1 ms less per 15 s rendered without underruns.
        val before = stats.extraMarginMs
        sim.run(30_000)
        val after = mixer.laneStats().getValue("pc").extraMarginMs
        assertEquals(before - 2f, after, 0.1f)
    }

    @Test
    fun starvationDuringStartupDoesNotRaiseMargin() {
        val mixer = LiveAudioMixer()
        val sim = Steady(mixer, "pc")
        sim.run(500)
        sim.starve()
        sim.run(500)
        assertEquals(0f, mixer.laneStats().getValue("pc").extraMarginMs, 0f)
        assertEquals(0, mixer.laneStats().getValue("pc").underruns)
    }

    // --- Gain ----------------------------------------------------------------------------

    @Test
    fun sliderGainIsSquaredAndClamped() {
        assertEquals(0.25f, LiveAudioMixer.sliderGain(0.5f), 1e-6f)
        assertEquals(1f, LiveAudioMixer.sliderGain(1f), 0f)
        assertEquals(1f, LiveAudioMixer.sliderGain(2f), 0f)
        assertEquals(0f, LiveAudioMixer.sliderGain(-1f), 0f)
        assertEquals(0f, LiveAudioMixer.sliderGain(Float.NaN), 0f)
        assertEquals(0.25f, AudioRouting.Lane(slider = 0.5f).gain, 1e-6f)
    }

    @Test
    fun laneGainRampsInFromSilence() {
        val mixer = LiveAudioMixer()
        mixer.setLaneGain("pc", 0.5f)
        repeat(6) { mixer.push("pc", rate, 2, tone(block, 0.5f), it.toUInt(), it * 5L) }

        val first = ShortArray(block * 2)
        assertTrue(mixer.render(first, rate, 30))
        val target = 0.5f * 0.25f
        assertTrue("starts near zero: ${first[0]}", abs(first[0] / 32768f) < 0.01f)
        assertEquals(target, first[first.size - 2] / 32768f, 0.002f)
        for (frame in 1 until block) assertTrue(first[frame * 2] >= first[(frame - 1) * 2])

        val second = ShortArray(block * 2)
        mixer.render(second, rate, 35)
        for (sample in second) assertEquals(target, sample / 32768f, 0.001f)
    }

    @Test
    fun mutedLaneIsSilentAndMasterRamps() {
        val mixer = LiveAudioMixer()
        repeat(6) { mixer.push("pc", rate, 2, tone(block, 0.5f), it.toUInt(), it * 5L) }
        mixer.render(ShortArray(block * 2), rate, 30)

        mixer.masterMuted = true
        val fading = ShortArray(block * 2)
        mixer.render(fading, rate, 30)
        assertTrue(fading[0] > 15_000)
        assertEquals(0, fading[fading.size - 1].toInt())

        val silent = ShortArray(block * 2)
        mixer.render(silent, rate, 30)
        assertTrue(silent.all { it.toInt() == 0 })

        mixer.masterMuted = false
        mixer.setLaneGain("pc", 1f, muted = true)
        val muted = ShortArray(block * 2)
        mixer.render(muted, rate, 30)
        // The lane ramps out, then stays silent.
        assertEquals(0, muted[muted.size - 1].toInt())
    }

    @Test
    fun lanesMixInFloatAndClipOnce() {
        val mixer = LiveAudioMixer()
        val values = mapOf("a" to 0.9f, "b" to 0.9f, "c" to -0.9f)
        for ((id, value) in values) {
            repeat(6) { mixer.push(id, rate, 2, tone(block, value), it.toUInt(), it * 5L) }
        }
        mixer.render(ShortArray(block * 2), rate, 30)
        val out = ShortArray(block * 2)
        mixer.render(out, rate, 30)
        // Clipping each lane, or each partial sum, would give 0.1 or 1.0 here.
        for (sample in out) assertEquals(0.9f, sample / 32768f, 0.001f)

        mixer.removeLane("c")
        val clipped = ShortArray(block * 2)
        mixer.render(clipped, rate, 30)
        assertTrue(clipped.all { it == Short.MAX_VALUE })
    }

    // --- Lanes ---------------------------------------------------------------------------

    @Test
    fun removeLaneLeavesOtherLanesAndKeepsSettings() {
        val mixer = LiveAudioMixer()
        mixer.setLaneGain("a", 0.5f)
        repeat(6) {
            mixer.push("a", rate, 2, tone(block, 0.4f), it.toUInt(), it * 5L)
            mixer.push("b", rate, 2, tone(block, 0.2f), it.toUInt(), it * 5L)
        }
        mixer.render(ShortArray(block * 2), rate, 30)
        val both = ShortArray(block * 2)
        mixer.render(both, rate, 30)
        assertEquals(0.4f * 0.25f + 0.2f, both[0] / 32768f, 0.001f)

        val bBefore = mixer.laneStats().getValue("b")
        mixer.removeLane("a")
        assertEquals(setOf("b"), mixer.laneStats().keys)
        val bAfter = mixer.laneStats().getValue("b")
        assertEquals(bBefore, bAfter)

        val onlyB = ShortArray(block * 2)
        mixer.render(onlyB, rate, 30)
        for (sample in onlyB) assertEquals(0.2f, sample / 32768f, 0.001f)

        // "a" reconnects: its 0.5 slider still applies.
        mixer.removeLane("b")
        repeat(6) { mixer.push("a", rate, 2, tone(block, 0.4f), it.toUInt(), 2_000 + it * 5L) }
        mixer.render(ShortArray(block * 2), rate, 2_030)
        val again = ShortArray(block * 2)
        mixer.render(again, rate, 2_030)
        assertEquals(0.4f * 0.25f, again[0] / 32768f, 0.001f)

        // forgetLane drops the settings too.
        mixer.forgetLane("a")
        repeat(6) { mixer.push("a", rate, 2, tone(block, 0.4f), it.toUInt(), 4_000 + it * 5L) }
        mixer.render(ShortArray(block * 2), rate, 4_030)
        val fresh = ShortArray(block * 2)
        mixer.render(fresh, rate, 4_030)
        assertEquals(0.4f, fresh[0] / 32768f, 0.001f)
    }

    @Test
    fun invalidPacketsAreRejected() {
        val mixer = LiveAudioMixer()
        assertFalse(mixer.push("pc", 4_000, 2, ByteArray(8)))
        assertFalse(mixer.push("pc", rate, 3, ByteArray(12)))
        assertFalse(mixer.push("pc", rate, 2, ByteArray(0)))
        assertFalse(mixer.push("pc", rate, 2, ByteArray(6)))
        assertTrue(mixer.laneStats().isEmpty())
    }

    @Test
    fun latencyPresetsMatchProtocol() {
        assertEquals(4, LiveAudioMixer.LatencyPreset.LOWEST.marginMs)
        assertEquals(12, LiveAudioMixer.LatencyPreset.BALANCED.marginMs)
        assertEquals(35, LiveAudioMixer.LatencyPreset.STABLE.marginMs)

        for (preset in LiveAudioMixer.LatencyPreset.entries) {
            val mixer = LiveAudioMixer().apply { latencyPreset = preset }
            val sim = Steady(mixer, "pc")
            sim.run(5_000)
            val stats = mixer.laneStats().getValue("pc")
            // LOWEST sits at the floor of one 5 ms block plus 1 ms.
            val expected = maxOf(preset.marginMs.toFloat(), 6f)
            assertTrue("$preset: ${stats.bufferMs}", stats.bufferMs > expected - 1f && stats.bufferMs < expected + 4f)
            assertEquals("$preset underruns", 0, stats.underruns)
        }
    }

    // --- Playback policy -----------------------------------------------------------------

    @Test
    fun playbackDebounceActivatesAfter150AndReleasesAfter1500() {
        val debounce = PlaybackDebounce()
        assertEquals(150L, debounce.update(true, 0))
        assertEquals(50L, debounce.update(true, 100))
        assertFalse(debounce.active)
        assertNull(debounce.update(true, 150))
        assertTrue(debounce.active)

        assertEquals(1_500L, debounce.update(false, 200))
        // Playback came back before the release: stays active.
        assertNull(debounce.update(true, 1_000))
        assertTrue(debounce.active)
        assertNull(debounce.update(true, 5_000))
        assertTrue(debounce.active)

        assertEquals(1_500L, debounce.update(false, 6_000))
        assertEquals(1L, debounce.update(false, 7_499))
        assertTrue(debounce.active)
        assertNull(debounce.update(false, 7_500))
        assertFalse(debounce.active)

        // A blip shorter than 150 ms never activates.
        assertEquals(150L, debounce.update(true, 8_000))
        assertNull(debounce.update(false, 8_100))
        assertNull(debounce.update(false, 9_000))
        assertFalse(debounce.active)
    }

    @Test
    fun otherPlaybackIgnoresOwnAndAmbiguousSpeakerTrack() {
        val own = 10_001
        fun player(uid: Int?, id: Int?, relevant: Boolean = true, speaker: Boolean = true) =
            PlaybackObservation(relevant, speaker, uid, id)

        assertFalse(hasOtherPlayback(emptyList(), own, false, emptySet()))
        assertFalse(hasOtherPlayback(listOf(player(own, 1)), own, true, emptySet()))
        assertFalse(hasOtherPlayback(listOf(player(10_002, 1, relevant = false)), own, false, emptySet()))
        assertTrue(hasOtherPlayback(listOf(player(10_002, 1)), own, true, emptySet()))
        // Without a uid, a single media player might be our own speaker track.
        assertFalse(hasOtherPlayback(listOf(player(null, 7)), own, true, emptySet()))
        assertTrue(hasOtherPlayback(listOf(player(null, 7)), own, false, emptySet()))
        assertTrue(hasOtherPlayback(listOf(player(null, 7)), own, true, setOf(7)))
        assertTrue(hasOtherPlayback(listOf(player(null, 7, speaker = false)), own, true, emptySet()))
        assertTrue(hasOtherPlayback(listOf(player(null, 7), player(null, 8)), own, true, emptySet()))
    }

    @Test
    fun policyGains() {
        assertEquals(0f, PlaybackPolicy.PAUSE.gain(true), 0f)
        assertEquals(0f, PlaybackPolicy.MUTE.gain(true), 0f)
        assertEquals(0.15f, PlaybackPolicy.LOWER.gain(true), 0f)
        assertEquals(1f, PlaybackPolicy.OFF.gain(true), 0f)
        for (policy in PlaybackPolicy.entries) assertEquals(1f, policy.gain(false), 0f)
    }

    // --- Routing -------------------------------------------------------------------------

    @Test
    fun routingBumpsGenerationAndFallsBackToPhone() {
        val routes = mutableListOf<AudioRouting.State>()
        AudioRouting.hooks = AudioRouting.Hooks(onRouteChanged = { routes += it })
        val pc = AudioRouting.Peer("pc-a", "Desk", canSink = true)
        AudioRouting.updatePeers(listOf(pc))
        assertTrue(routes.isEmpty()) // disabled: nothing announced

        AudioRouting.setEnabled(true)
        val start = AudioRouting.state.value.generation
        AudioRouting.selectOutput(AudioRouting.Output.Pc("pc-a"))
        assertEquals(AudioRouting.Output.Pc("pc-a"), AudioRouting.state.value.output)
        assertEquals(start + 1, AudioRouting.state.value.generation)

        // Gain changes don't bump the generation.
        AudioRouting.setLaneGain("pc-a", 0.5f)
        AudioRouting.setMasterGain(0.5f)
        assertEquals(start + 1, AudioRouting.state.value.generation)
        assertEquals(0.25f, AudioRouting.state.value.masterLinearGain, 1e-6f)

        // The output PC goes offline: back to the phone, with a new generation.
        AudioRouting.updatePeers(listOf(pc.copy(online = false)))
        val state = AudioRouting.state.value
        assertEquals(AudioRouting.Output.Phone, state.output)
        assertEquals(start + 2, state.generation)
        assertEquals(state, routes.last())
        assertTrue(state.enrolledSources().isEmpty())
        assertEquals(0.5f, state.lanes.getValue("pc-a").slider, 0f)
    }

    @Test
    fun routingRejectsOfflineOrNonSinkOutputs() {
        AudioRouting.setEnabled(true)
        AudioRouting.updatePeers(listOf(AudioRouting.Peer("pc-a", "Desk", canSink = false)))
        AudioRouting.selectOutput(AudioRouting.Output.Pc("pc-a"))
        assertEquals(AudioRouting.Output.Phone, AudioRouting.state.value.output)
        AudioRouting.selectOutput(AudioRouting.Output.Pc("missing"))
        assertEquals(AudioRouting.Output.Phone, AudioRouting.state.value.output)
    }

    @Test
    fun micMeterFlagsTalkingWhileMuted() {
        AudioRouting.setMicMuted(true)
        AudioRouting.updateMicPeak(0.2f)
        assertTrue(AudioRouting.state.value.mic.talkingWhileMuted)
        AudioRouting.updateMicPeak(0.01f)
        assertFalse(AudioRouting.state.value.mic.talkingWhileMuted)
        AudioRouting.updateMicPeak(0.2f)
        AudioRouting.setMicMuted(false)
        assertFalse(AudioRouting.state.value.mic.talkingWhileMuted)
        AudioRouting.setMicGain(5f)
        assertEquals(2f, AudioRouting.state.value.mic.gain, 0f)
    }

    // --- Mic -----------------------------------------------------------------------------

    @Test
    fun micBlocksAreFiveMilliseconds() {
        assertEquals(480, MicPcm.blockBytes(48_000))
        assertEquals(160, MicPcm.blockBytes(16_000))
        assertArrayEquals(intArrayOf(48_000, 16_000), MicPcm.RATES)
    }

    @Test
    fun micGainAppliesInPlaceAndMuteKeepsPeak() {
        val pcm = pcm16(0.25f, -0.5f)
        val peak = MicPcm.applyGain(pcm, pcm.size, 1.5f, muted = false)
        assertEquals(0.75f, peak, 0.001f)
        assertEquals(0.375f, sampleAt(pcm, 0), 0.001f)
        assertEquals(-0.75f, sampleAt(pcm, 2), 0.001f)

        val loud = pcm16(0.8f)
        assertEquals(1f, MicPcm.applyGain(loud, loud.size, 2f, muted = false), 1e-4f)
        assertEquals(32767, (sampleAt(loud, 0) * 32768).roundToInt())

        val muted = pcm16(0.3f, -0.3f)
        assertEquals(0.3f, MicPcm.applyGain(muted, muted.size, 1f, muted = true), 0.001f)
        assertTrue(muted.all { it.toInt() == 0 })
        assertEquals(0.3f, MicPcm.peak(pcm16(0.1f, -0.3f), 4), 0.001f)
    }

    @Test
    fun micStallRetriesOnceThenFallsBack() {
        val watch = MicStallWatch()
        assertEquals(MicStallWatch.Action.NONE, watch.check(4_000, 1, 0, builtin = false))
        assertEquals(MicStallWatch.Action.NONE, watch.check(6_000, 1, 5_000, builtin = false))
        assertEquals(MicStallWatch.Action.RETRY, watch.check(9_000, 1, 5_000, builtin = false))
        assertEquals(MicStallWatch.Action.FALL_BACK, watch.check(9_500, 1, 5_000, builtin = false))

        // The built-in mic only retries.
        val builtin = MicStallWatch()
        repeat(3) { assertEquals(MicStallWatch.Action.RETRY, builtin.check(10_000, 1, 0, builtin = true)) }

        // Data in between clears the count.
        val recovering = MicStallWatch()
        assertEquals(MicStallWatch.Action.RETRY, recovering.check(9_000, 1, 0, builtin = false))
        assertEquals(MicStallWatch.Action.NONE, recovering.check(10_000, 1, 9_500, builtin = false))
        assertEquals(MicStallWatch.Action.RETRY, recovering.check(13_000, 1, 9_500, builtin = false))
    }

    // --- Helpers -------------------------------------------------------------------------

    private class Sender(val id: String, val intervalMs: Double) {
        var nextMs = 0.0
        var seq = 0u
    }

    /** One sender on the phone's clock, with 5 ms packets and 5 ms render blocks. */
    private inner class Steady(private val mixer: LiveAudioMixer, private val id: String) {
        var nowMs = 0L
        private var seq = 0u
        private val out = ShortArray(block * 2)

        fun pushNow() {
            mixer.push(id, rate, 2, tone(block, 0.1f), seq++, nowMs)
        }

        fun run(ms: Long) {
            val end = nowMs + ms
            while (nowMs < end) {
                pushNow()
                mixer.render(out, rate, nowMs)
                nowMs += 5
            }
        }

        /** Stops sending until the lane runs dry, then resumes 10 ms later. */
        fun starve() {
            var guard = 0
            while (mixer.laneStats().getValue(id).live) {
                mixer.render(out, rate, nowMs)
                nowMs += 5
                check(++guard < 1_000)
            }
            nowMs += 10
        }
    }

    private fun tone(frames: Int, value: Float): ByteArray {
        val sample = (value * 32768f).roundToInt().coerceIn(-32768, 32767)
        return ByteArray(frames * 4).also { bytes ->
            for (i in bytes.indices step 2) {
                bytes[i] = sample.toByte()
                bytes[i + 1] = (sample shr 8).toByte()
            }
        }
    }

    private fun pcm16(vararg values: Float): ByteArray = ByteArray(values.size * 2).also { bytes ->
        values.forEachIndexed { i, value ->
            val sample = (value * 32768f).roundToInt().coerceIn(-32768, 32767)
            bytes[i * 2] = sample.toByte()
            bytes[i * 2 + 1] = (sample shr 8).toByte()
        }
    }

    private fun sampleAt(bytes: ByteArray, offset: Int): Float =
        ((bytes[offset].toInt() and 0xff) or (bytes[offset + 1].toInt() shl 8)).toShort() / 32768f
}
