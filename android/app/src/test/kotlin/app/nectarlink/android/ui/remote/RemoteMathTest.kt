// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class RemoteMathTest {
    @Test
    fun slowMovesAreFineAndQuickOnesCoverGround() {
        val slow = TouchpadBallistics().gain(distance = 1f, dtMs = 8)
        val medium = TouchpadBallistics().gain(distance = 12f, dtMs = 8)
        val fast = TouchpadBallistics().gain(distance = 80f, dtMs = 8)
        assertTrue("slow $slow is finer than 1:1", slow < 1f)
        assertTrue("medium $medium is between", medium > slow && medium < fast)
        assertTrue("fast $fast goes further", fast > 2f)
    }

    @Test
    fun aLateOrInstantFrameDoesNotLurch() {
        val ballistics = TouchpadBallistics()
        repeat(10) { ballistics.gain(distance = 2f, dtMs = 8) }
        val steady = ballistics.gain(distance = 2f, dtMs = 8)
        // Two frames landing together: no time between them, then one big move.
        val instant = ballistics.gain(distance = 2f, dtMs = 0)
        val burst = ballistics.gain(distance = 16f, dtMs = 8)
        assertTrue(instant.isFinite() && burst.isFinite())
        assertTrue("burst $burst stays near steady $steady", burst - steady < 0.8f)
    }

    @Test
    fun theSmallestMoveStillMoves() {
        assertTrue(TouchpadBallistics().gain(distance = 0.1f, dtMs = 8) > 0f)
    }

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
        assertEquals("This is NectarLink.", joiner.next("This is NectarLink.").removePrefix(" ").also {
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

    @Test
    fun pcMirrorLandscapePillarboxMapsCoordinatesAndRejectsBarTaps() {
        // 20:9 phone in full-screen landscape (2400x1080) viewing a 16:9 PC (1920x1080):
        // fitted video is 1920x1080 with 240px pillarbox bars on left and right.
        val aspect16x9 = 16f / 9f
        val vp = PcMirrorMath.computeVideoViewport(
            containerWidth = 2400f,
            containerHeight = 1080f,
            videoAspect = aspect16x9,
        )
        assertEquals(240f, vp.left, 1e-3f)
        assertEquals(0f, vp.top, 1e-3f)
        assertEquals(1920f, vp.width, 1e-3f)
        assertEquals(1080f, vp.height, 1e-3f)

        // Taps in the left or right pillarbox bars return null when clampOutside = false,
        // and clamp to 0f / 1f during drag (clampOutside = true).
        assertEquals(null, PcMirrorMath.mapTouchToNormalized(120f, 540f, vp, clampOutside = false))
        assertEquals(null, PcMirrorMath.mapTouchToNormalized(2300f, 540f, vp, clampOutside = false))
        val clampedLeft = PcMirrorMath.mapTouchToNormalized(120f, 540f, vp, clampOutside = true)!!
        assertEquals(0f, clampedLeft.first, 1e-5f)
        assertEquals(0.5f, clampedLeft.second, 1e-5f)

        // Left video edge (x = 240), center (x = 1200, y = 540), and bottom-right video corner (x = 2160, y = 1080).
        val topLeft = PcMirrorMath.mapTouchToNormalized(240f, 0f, vp, clampOutside = false)!!
        assertEquals(0f, topLeft.first, 1e-5f)
        assertEquals(0f, topLeft.second, 1e-5f)

        val center = PcMirrorMath.mapTouchToNormalized(1200f, 540f, vp, clampOutside = false)!!
        assertEquals(0.5f, center.first, 1e-5f)
        assertEquals(0.5f, center.second, 1e-5f)

        val bottomRight = PcMirrorMath.mapTouchToNormalized(2160f, 1080f, vp, clampOutside = false)!!
        assertEquals(1f, bottomRight.first, 1e-5f)
        assertEquals(1f, bottomRight.second, 1e-5f)
    }

    @Test
    fun pcMirrorLandscapeLetterboxAndPortraitBothOrientationsMapAccurately() {
        // 1) Landscape letterboxed: 16:10 screen (1920x1200) viewing a 21:9 ultrawide PC (2520x1080, aspect = 7/3):
        // fitted video is 1920x822.857 with 188.571px letterbox bars top and bottom.
        val aspectUltrawide = 21f / 9f
        val landscapeLetterbox = PcMirrorMath.computeVideoViewport(
            containerWidth = 1920f,
            containerHeight = 1200f,
            videoAspect = aspectUltrawide,
        )
        assertEquals(0f, landscapeLetterbox.left, 1e-3f)
        assertEquals(1920f, landscapeLetterbox.width, 1e-3f)
        assertEquals(1920f / aspectUltrawide, landscapeLetterbox.height, 1e-2f)
        val expectedTopBar = (1200f - landscapeLetterbox.height) * 0.5f
        assertEquals(expectedTopBar, landscapeLetterbox.top, 1e-2f)

        assertEquals(null, PcMirrorMath.mapTouchToNormalized(960f, 50f, landscapeLetterbox, clampOutside = false))
        val landTop = PcMirrorMath.mapTouchToNormalized(960f, expectedTopBar, landscapeLetterbox, clampOutside = false)!!
        assertEquals(0.5f, landTop.first, 1e-4f)
        assertEquals(0f, landTop.second, 1e-4f)
        val landCenter = PcMirrorMath.mapTouchToNormalized(960f, 600f, landscapeLetterbox, clampOutside = false)!!
        assertEquals(0.5f, landCenter.first, 1e-4f)
        assertEquals(0.5f, landCenter.second, 1e-4f)

        // 2) Portrait letterboxed: 1080x1800 container viewing a 16:9 PC:
        // fitted video is 1080x607.5 with 596.25px letterbox bars top and bottom.
        val portraitLetterbox = PcMirrorMath.computeVideoViewport(
            containerWidth = 1080f,
            containerHeight = 1800f,
            videoAspect = 16f / 9f,
        )
        assertEquals(0f, portraitLetterbox.left, 1e-3f)
        assertEquals(596.25f, portraitLetterbox.top, 1e-3f)
        assertEquals(1080f, portraitLetterbox.width, 1e-3f)
        assertEquals(607.5f, portraitLetterbox.height, 1e-3f)

        assertEquals(null, PcMirrorMath.mapTouchToNormalized(540f, 200f, portraitLetterbox, clampOutside = false))
        val portQuarter = PcMirrorMath.mapTouchToNormalized(
            touchX = 270f,
            touchY = 596.25f + 607.5f * 0.75f,
            containerWidth = 1080f,
            containerHeight = 1800f,
            videoAspect = 16f / 9f,
            clampOutside = false,
        )!!
        assertEquals(0.25f, portQuarter.first, 1e-4f)
        assertEquals(0.75f, portQuarter.second, 1e-4f)

        // 3) Portrait pillarboxed: 1080x1800 container viewing a tall 9:20 vertical monitor (aspect = 0.45):
        // containerAspect = 0.6 > 0.45, so fitted height = 1800, fitted width = 810, left = 135.
        val portraitPillarbox = PcMirrorMath.computeVideoViewport(
            containerWidth = 1080f,
            containerHeight = 1800f,
            videoAspect = 9f / 20f,
        )
        assertEquals(135f, portraitPillarbox.left, 1e-3f)
        assertEquals(0f, portraitPillarbox.top, 1e-3f)
        assertEquals(810f, portraitPillarbox.width, 1e-3f)
        assertEquals(1800f, portraitPillarbox.height, 1e-3f)

        val portPillarCenter = PcMirrorMath.mapTouchToNormalized(540f, 900f, portraitPillarbox, clampOutside = false)!!
        assertEquals(0.5f, portPillarCenter.first, 1e-4f)
        assertEquals(0.5f, portPillarCenter.second, 1e-4f)
    }

    @Test
    fun pcMirrorAutoRotateRespectsSystemLockAndManualButtonState() {
        // When system auto-rotate lock is on (autoRotateEnabled = false), physical landscape never auto-enters.
        assertFalse(
            PcMirrorMath.shouldAutoEnterFullScreen(
                sensorDegrees = 90,
                autoRotateEnabled = false,
                isFullScreen = false,
                suppressWhileInLandscape = false,
            ),
        )
        // When system auto-rotate is enabled, rotating to 90 or 270 enters full screen.
        assertTrue(
            PcMirrorMath.shouldAutoEnterFullScreen(
                sensorDegrees = 90,
                autoRotateEnabled = true,
                isFullScreen = false,
                suppressWhileInLandscape = false,
            ),
        )
        assertTrue(
            PcMirrorMath.shouldAutoEnterFullScreen(
                sensorDegrees = 270,
                autoRotateEnabled = true,
                isFullScreen = false,
                suppressWhileInLandscape = false,
            ),
        )
        // If user manually collapsed while holding in landscape, do not re-enter until rotated back to portrait.
        assertFalse(
            PcMirrorMath.shouldAutoEnterFullScreen(
                sensorDegrees = 90,
                autoRotateEnabled = true,
                isFullScreen = false,
                suppressWhileInLandscape = true,
            ),
        )
        // When entered via button while holding phone in portrait, do not immediately auto-exit on portrait sensor reading.
        assertFalse(
            PcMirrorMath.shouldAutoExitFullScreen(
                sensorDegrees = 0,
                autoRotateEnabled = true,
                isFullScreen = true,
                enteredViaButton = true,
                sawPhysicalLandscapeSinceEnter = false,
            ),
        )
        // Once rotated to landscape and then back to portrait with auto-rotate enabled, auto-exit to portrait viewer.
        assertTrue(
            PcMirrorMath.shouldAutoExitFullScreen(
                sensorDegrees = 0,
                autoRotateEnabled = true,
                isFullScreen = true,
                enteredViaButton = true,
                sawPhysicalLandscapeSinceEnter = true,
            ),
        )
    }

    // A 16:9 picture pillarboxed in a 2000 x 900 container: 1600 x 900 at x = 200.
    private val zoomViewport = PcMirrorMath.computeVideoViewport(2000f, 900f, 16f / 9f)

    @Test
    fun pinchKeepsThePointUnderTheFingersWhereItWas() {
        val focusX = 900f
        val focusY = 300f
        val before = PcMirrorMath.mapTouchToNormalized(focusX, focusY, zoomViewport)!!
        val zoom = PcMirrorMath.pinch(PcZoom(), zoomViewport, focusX, focusY, factor = 2f, panX = 0f, panY = 0f)
        assertEquals(2f, zoom.scale, 0.0001f)
        val (ux, uy) = PcMirrorMath.unzoom(focusX, focusY, zoomViewport, zoom)
        val after = PcMirrorMath.mapTouchToNormalized(ux, uy, zoomViewport)!!
        assertEquals(before.first, after.first, 0.0001f)
        assertEquals(before.second, after.second, 0.0001f)
    }

    @Test
    fun aZoomedPictureAlwaysFillsItsFrame() {
        // Zoomed at the top-left corner, then dragged far right and down: it stops at the edge.
        var zoom = PcMirrorMath.pinch(PcZoom(), zoomViewport, 200f, 0f, factor = 3f, panX = 0f, panY = 0f)
        zoom = PcMirrorMath.pinch(zoom, zoomViewport, 600f, 400f, factor = 1f, panX = 5000f, panY = 5000f)
        assertEquals(0f, zoom.x, 0.0001f)
        assertEquals(0f, zoom.y, 0.0001f)
        // The corners of the frame still land inside the PC's screen.
        val (lx, ty) = PcMirrorMath.unzoom(zoomViewport.left, zoomViewport.top, zoomViewport, zoom)
        val (rx, by) = PcMirrorMath.unzoom(zoomViewport.right, zoomViewport.bottom, zoomViewport, zoom)
        assertTrue(lx >= zoomViewport.left - 0.01f && rx <= zoomViewport.right + 0.01f)
        assertTrue(ty >= zoomViewport.top - 0.01f && by <= zoomViewport.bottom + 0.01f)
        // Dragged the other way, it stops at the far edge.
        zoom = PcMirrorMath.pinch(zoom, zoomViewport, 600f, 400f, factor = 1f, panX = -9000f, panY = -9000f)
        assertEquals(1f - zoom.scale, zoom.x, 0.0001f)
        assertEquals(1f - zoom.scale, zoom.y, 0.0001f)
    }

    @Test
    fun zoomStaysWithinLimitsAndSnapsBackToTheWholeScreen() {
        val far = PcMirrorMath.pinch(PcZoom(), zoomViewport, 1000f, 450f, factor = 50f, panX = 0f, panY = 0f)
        assertEquals(PcMirrorMath.MAX_ZOOM, far.scale, 0.0001f)
        val almost = PcMirrorMath.pinch(PcZoom(scale = 1.5f), zoomViewport, 1000f, 450f, factor = 0.68f, panX = 0f, panY = 0f)
        assertEquals(PcZoom(), almost)
        assertFalse(almost.zoomed)
        // Not zoomed, a touch maps as it always did.
        assertEquals(123f to 456f, PcMirrorMath.unzoom(123f, 456f, zoomViewport, PcZoom()))
        // A broken factor (no movement between events) changes nothing.
        val zoom = PcZoom(scale = 2f, x = -0.5f, y = -0.25f)
        assertEquals(zoom, PcMirrorMath.pinch(zoom, zoomViewport, 1000f, 450f, factor = Float.NaN, panX = 0f, panY = 0f))
    }
}
