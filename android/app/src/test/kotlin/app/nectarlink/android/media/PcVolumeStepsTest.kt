// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PcVolumeStepsTest {
    @Test
    fun pcVolumeRoundsToTheNearestStep() {
        assertEquals(0, PcMedia.toSteps(0))
        assertEquals(0, PcMedia.toSteps(1))
        assertEquals(1, PcMedia.toSteps(2))
        assertEquals(9, PcMedia.toSteps(37))
        assertEquals(PcMedia.VOLUME_STEPS, PcMedia.toSteps(100))
    }

    @Test
    fun outOfRangeVolumesAreClamped() {
        assertEquals(0, PcMedia.toSteps(-5))
        assertEquals(PcMedia.VOLUME_STEPS, PcMedia.toSteps(140))
    }

    @Test
    fun hugeArtworkIsDecodedSmall() {
        assertEquals(1, PcMedia.artSampleSize(300, 300))
        assertEquals(1, PcMedia.artSampleSize(1023, 600))
        assertEquals(2, PcMedia.artSampleSize(1024, 1024))
        // A tiny file can claim 50,000 x 50,000 pixels: decoded at 1/64, about 780 px.
        assertEquals(64, PcMedia.artSampleSize(50_000, 50_000))
    }

    @Test
    fun aSuddenRiseOutOfNowhereIsLeftAlone() {
        // HyperOS's full volume when an app opens (#643).
        assertTrue(PcMedia.isStrayJump(from = 10, to = PcMedia.VOLUME_STEPS, sinceTouchMs = 60_000))
        // Right after the user moved it, a big rise is theirs (a tap far along the panel).
        assertFalse(PcMedia.isStrayJump(from = 10, to = PcMedia.VOLUME_STEPS, sinceTouchMs = 300))
        // Small rises, like dragging the panel, always go through.
        assertFalse(PcMedia.isStrayJump(from = 10, to = 10 + PcMedia.MAX_JUMP, sinceTouchMs = 60_000))
        // Lowering always goes through.
        assertFalse(PcMedia.isStrayJump(from = PcMedia.VOLUME_STEPS, to = 0, sinceTouchMs = 60_000))
    }

    @Test
    fun everyStepMapsBackToItself() {
        for (step in 0..PcMedia.VOLUME_STEPS) {
            assertEquals(step, PcMedia.toSteps(step * 100 / PcMedia.VOLUME_STEPS))
        }
    }
}
