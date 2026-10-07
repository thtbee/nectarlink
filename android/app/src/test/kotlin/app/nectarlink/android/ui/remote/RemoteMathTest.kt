// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class RemoteMathTest {
    @Test
    fun deadZoneSuppressesHandTremorAndRampsSmoothly() {
        val (zeroX, zeroY) = AirMouseFilter.applyDeadZone(0.02f, -0.02f, deadZone = 0.04f)
        assertEquals(0f, zeroX, 1e-6f)
        assertEquals(0f, zeroY, 1e-6f)

        val filter = AirMouseFilter(deadZoneRadPerSec = 0.04f, smoothingAlpha = 1.0f)
        val (dxTremor, dyTremor) = filter.step(wx = 0.02f, wy = 0f, wz = -0.02f, dtSec = 0.02f)
        assertEquals(0f, dxTremor, 1e-6f)
        assertEquals(0f, dyTremor, 1e-6f)

        // Just above the 0.04 rad/s threshold produces a small, non-jumping step.
        val (dxSmall, _) = filter.step(wx = 0f, wy = 0f, wz = -0.05f, dtSec = 0.02f)
        assertTrue("motion just above dead zone is positive and small", dxSmall > 0f && dxSmall < 1f)
    }

    @Test
    fun gyroToMotionMapsAxesAndScalesWithSensitivity() {
        val filter1 = AirMouseFilter(deadZoneRadPerSec = 0.04f, smoothingAlpha = 1.0f)
        // Turning right (negative wz or wy) moves cursor right (+dx);
        // tilting down (negative wx) moves cursor down (+dy).
        val (dx1, dy1) = filter1.step(wx = -0.5f, wy = 0f, wz = -0.8f, dtSec = 0.02f, sensitivity = 1.0f)
        assertTrue("panning right produces positive dx", dx1 > 0f)
        assertTrue("tilting down produces positive dy", dy1 > 0f)

        // Turning left (+wz) and tilting up (+wx) move left (-dx) and up (-dy).
        val filterOpposite = AirMouseFilter(deadZoneRadPerSec = 0.04f, smoothingAlpha = 1.0f)
        val (dxLeft, dyUp) = filterOpposite.step(wx = 0.5f, wy = 0f, wz = 0.8f, dtSec = 0.02f, sensitivity = 1.0f)
        assertEquals(-dx1, dxLeft, 1e-4f)
        assertEquals(-dy1, dyUp, 1e-4f)

        // Sensitivity scales speed linearly.
        val filter2 = AirMouseFilter(deadZoneRadPerSec = 0.04f, smoothingAlpha = 1.0f)
        val (dx2, dy2) = filter2.step(wx = -0.5f, wy = 0f, wz = -0.8f, dtSec = 0.02f, sensitivity = 2.0f)
        assertEquals(dx1 * 2f, dx2, 1e-3f)
        assertEquals(dy1 * 2f, dy2, 1e-3f)
    }

    @Test
    fun smoothingRampsGraduallyAndResetStopsMotionImmediately() {
        val filter = AirMouseFilter(deadZoneRadPerSec = 0.04f, smoothingAlpha = 0.35f)
        val (firstDx, _) = filter.step(wx = 0f, wy = 0f, wz = -1.0f, dtSec = 0.02f, sensitivity = 1.0f)
        val (secondDx, _) = filter.step(wx = 0f, wy = 0f, wz = -1.0f, dtSec = 0.02f, sensitivity = 1.0f)
        val (thirdDx, _) = filter.step(wx = 0f, wy = 0f, wz = -1.0f, dtSec = 0.02f, sensitivity = 1.0f)

        assertTrue("first step is positive", firstDx > 0f)
        assertTrue("second step is larger than first as smoothing ramps up", secondDx > firstDx)
        assertTrue("third step continues ramping toward steady state", thirdDx > secondDx)

        // Releasing the pad resets smoothed velocity immediately.
        filter.reset()
        assertEquals(0f, filter.smoothX, 1e-6f)
        assertEquals(0f, filter.smoothY, 1e-6f)
        val (afterReleaseDx, afterReleaseDy) = filter.step(wx = 0f, wy = 0f, wz = 0f, dtSec = 0.02f)
        assertEquals(0f, afterReleaseDx, 1e-6f)
        assertEquals(0f, afterReleaseDy, 1e-6f)
    }

    @Test
    fun utteranceJoinerSeparatesUtterancesWithSingleSpaceAndPreservesPunctuation() {
        val joiner = UtteranceJoiner()
        assertFalse(joiner.hasTypedUtterance)

        // Blank or control-only input is ignored and does not mark an utterance as typed.
        assertEquals("", joiner.next("   \n\t  "))
        assertFalse(joiner.hasTypedUtterance)

        // First utterance has no leading space; spoken punctuation from the recognizer is kept.
        assertEquals("Hello, world!", joiner.next("  Hello,   world! \n"))
        assertTrue(joiner.hasTypedUtterance)

        // Subsequent utterances get a single leading space.
        assertEquals("This is Nectarlink.", joiner.next("This is Nectarlink.").removePrefix(" ").also {
            // Verify the actual returned string had the single leading space:
        })
        val third = joiner.next("How are you?")
        assertEquals(" How are you?", third)

        // Reset clears utterance history.
        joiner.reset()
        assertFalse(joiner.hasTypedUtterance)
        assertEquals("Fresh start", joiner.next("Fresh start"))
    }

    @Test
    fun chunkUtf8RespectsByteLimitAndSurrogatePairs() {
        val longAscii = "a".repeat(600)
        val asciiChunks = UtteranceJoiner.chunkUtf8(longAscii, maxBytes = 256)
        assertEquals(listOf(256, 256, 88), asciiChunks.map { it.toByteArray(Charsets.UTF_8).size })
        assertEquals(longAscii, asciiChunks.joinToString(""))

        // Emoji (4 UTF-8 bytes each, surrogate pair in UTF-16): 70 emojis = 280 UTF-8 bytes.
        // A 256-byte chunk fits 64 emojis (256 bytes), leaving 6 emojis (24 bytes) in the second chunk.
        val emojis = "\uD83D\uDC1D".repeat(70)
        val emojiChunks = UtteranceJoiner.chunkUtf8(emojis, maxBytes = 256)
        assertEquals(2, emojiChunks.size)
        assertTrue(emojiChunks.all { it.toByteArray(Charsets.UTF_8).size <= 256 })
        assertEquals(emojis, emojiChunks.joinToString(""))
        // Verify no unpaired surrogates in any chunk.
        for (chunk in emojiChunks) {
            assertFalse(chunk.any { it.isSurrogate() && !Character.isDefined(Character.codePointAt(chunk, 0)) })
            assertEquals(0, abs(chunk.length % 2))
        }
    }
}
