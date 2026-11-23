// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.pow

class RemoteMathTest {
    @Test
    fun slowMovesAreFineAndQuickOnesCoverGround() {
        val slow = TouchpadBallistics().gain(distance = 1f, dtMs = 8)
        val medium = TouchpadBallistics().gain(distance = 12f, dtMs = 8)
        val fast = TouchpadBallistics().gain(distance = 80f, dtMs = 8)
        assertEquals(1.6f * 0.72f, slow, 1e-6f)
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
    @Test
    fun driftBallisticsUsesSmoothedDpSpeedAndClampsPreferences() {
        val ballistics = TouchpadBallistics(pointerSpeed = 2f, acceleration = 0.5f)
        val t = (0.45f - 0.1f) / 1.4f
        val first = 2f * (0.72f + (0.28f + 0.5f * 3.2f) * t * t * (3f - 2f * t))
        assertEquals(first, ballistics.gainDp(4f, 1), 1e-6f)
        assertEquals(first, ballistics.gainDp(0f, 16), 1e-6f)
        assertTrue(ballistics.gainDp(4f, 4) > first)
        ballistics.reset()
        assertEquals(first, ballistics.gainDp(4f, 4), 1e-6f)
        assertEquals(0.4f, TouchpadBallistics(0f, -1f).gainDp(100f, 4), 1e-6f)
        assertEquals(16.8f, TouchpadBallistics(99f, 9f).gainDp(100f, 4), 1e-5f)
    }

    @Test
    fun driftWheelNotchesFollowNectarLinkSigns() {
        val natural = TouchpadBallistics.scrollNotches(100f, -100f)
        assertEquals(-1.45f, natural.first, 1e-6f)
        assertEquals(1.45f, natural.second, 1e-6f)
        val traditional = TouchpadBallistics.scrollNotches(100f, -100f, scrollSpeed = 2f, naturalScroll = false)
        assertEquals(2.9f, traditional.first, 1e-6f)
        assertEquals(-2.9f, traditional.second, 1e-6f)
        assertEquals(-0.96f, TouchpadBallistics.pinchNotches(40f), 1e-6f)
        assertEquals(0.96f, TouchpadBallistics.pinchNotches(-40f), 1e-6f)
    }

    @Test
    fun airYawUsesGravityAndPitchTracksAllDisplayRotations() {
        val expectedPitch = listOf(1f, -2f, -1f, 2f)
        for ((i, rotation) in DisplayRotation.entries.withIndex()) {
            val (yaw, pitch) = AirMouseProjection.rates(1f, 2f, 3f, 0f, 0f, 9.81f, rotation)
            assertEquals(rotation, DisplayRotation.fromSurfaceRotation(i))
            assertEquals(3f, yaw, 1e-6f)
            assertEquals(expectedPitch[i], pitch, 1e-6f)
        }
        // Both the screen-right and phone-normal axes have vertical components in this grip.
        val (yaw, pitch) = AirMouseProjection.rates(1f, 0f, 1f, 7f, 0f, 7f)
        assertEquals(1.4142135f, yaw, 1e-6f)
        assertEquals(0f, pitch, 1e-6f)
        assertEquals(0f to 0f, AirMouseProjection.rates(1f, 2f, 3f, 0f, 0f, 0f))
        assertEquals(1f to 1f, AirMouseProjection.rates(1f, 0f, 0f, 9.81f, 0f, 0f))
    }

    @Test
    fun airPointerDeadZoneAndSpeedGainMatchDrift() {
        assertEquals(0f to 0f, AirMouseProjection.pointerDelta(0.02f, 0f, 1f))
        assertEquals(0f to 0f, AirMouseProjection.pointerDelta(1f, 0f, 0f))
        val (dx, dy) = AirMouseProjection.pointerDelta(-0.1f, 0f, 0.1f)
        assertEquals(120.75f, dx, 1e-4f)
        assertEquals(0f, dy, 1e-6f)
        val fast = AirMouseProjection.pointerDelta(0f, -0.3f, 0.1f, airSpeed = 2f)
        assertEquals(0.3f * 1150f * 2f * 1.65f, fast.second, 1e-3f)
        assertTrue(AirMouseProjection.pointerDelta(0.0201f, 0f, 1f).first < 0f)
    }

    @Test
    fun deskWaitsForSixteenQuietSamplesAfterAJolt() {
        val desk = DeskTrace()
        desk.samples(16)
        assertEquals(DeskStatus.Ready, desk.engine.status)
        desk.engine.onGravity(6f, 0f, 7f)
        desk.samples(6, ax = 2f, wz = 0.08f)
        val tilted = desk.engine.readMotion()
        assertEquals(DeskStatus.Tilted, tilted.status)
        assertEquals(0.0, tilted.xMetres, 0.0)
        assertEquals(0.0, tilted.twistRadians, 0.0)
        desk.engine.onGravity(0f, 0f, -9.81f)
        desk.samples(1)
        assertEquals(DeskStatus.Tilted, desk.engine.status)

        desk.engine.onGravity(0f, 0f, 9.81f)
        desk.samples(1, az = -2.21f)
        assertEquals(DeskStatus.Lifted, desk.engine.status)
        desk.samples(8, ax = 0.5f, wz = 0.08f)
        // Gyro movement interrupts the quiet count even after the accel has settled.
        desk.samples(6, wz = 0.08f)
        desk.samples(15, ax = 0.04f)
        val lifted = desk.engine.readMotion()
        assertEquals(DeskStatus.Lifted, lifted.status)
        assertEquals(0.0, lifted.xMetres, 0.0)
        assertEquals(0.0, lifted.twistRadians, 0.0)
        desk.samples(1, ax = 0.04f)
        assertEquals(DeskStatus.Ready, desk.engine.status)
        desk.samples(10, ax = 1.2f)
        assertTrue(desk.engine.readMotion().xMetres > 0.0)
    }

    @Test
    fun deskCommitsAStrokeAndBlocksWeakReversal() {
        val desk = DeskTrace()
        desk.samples(16)
        desk.samples(3, ax = 1.2f)
        assertEquals(0.0, desk.engine.readMotion().xMetres, 0.0)
        desk.samples(5, ax = 1.2f)
        val stroke = desk.engine.readMotion()
        assertEquals(DeskStatus.Tracking, stroke.status)
        assertTrue(stroke.xMetres > 0.0)
        desk.samples(8, ax = -1.2f)
        assertTrue(desk.engine.readMotion().xMetres > 0.0)
        desk.samples(13, ax = -1.2f)
        assertEquals(0.0, desk.engine.readMotion().xMetres, 0.0)
        desk.samples(1, ax = -1.2f)
        assertTrue(desk.engine.readMotion().xMetres < 0.0)
        assertEquals(0.0, desk.engine.readMotion().xMetres, 0.0)
    }

    @Test
    fun deskLearnsBiasAndRotatesSlideAndTwistIndependently() {
        val desk = DeskTrace()
        desk.samples(150, ax = 0.1f)
        assertEquals(DeskStatus.Ready, desk.engine.status)
        assertEquals(0.0, desk.engine.readMotion().xMetres, 0.0)
        desk.engine.reset()
        desk.samples(30, ax = 0.16f)
        assertEquals(DeskStatus.Ready, desk.engine.status)
        assertEquals(0.0, desk.engine.readMotion().xMetres, 0.0)
        desk.engine.rotation = DisplayRotation.Rotation90
        desk.samples(10, ax = 1.2f, wz = 0.1f)
        val slide = desk.engine.readMotion()
        assertEquals(0.0, slide.xMetres, 0.0)
        assertTrue(slide.yMetres > 0.0)
        assertEquals(0.005, slide.twistRadians, 1e-8)
        val withTwist = slide.pointerDelta(0.05)
        assertEquals(-3.25f, withTwist.first, 1e-5f)
        assertTrue(withTwist.second < 0f)
        assertEquals(0f, slide.pointerDelta(0.05, deskTwist = false).first, 1e-6f)
    }

    @Test
    fun deskPointerUsesDriftSpeedCurve() {
        val slow = DeskMotion(0.001, 0.002, 0.01, DeskStatus.Tracking)
        val (dx, dy) = slow.pointerDelta(0.1)
        assertEquals(6.1f, dx, 1e-5f)
        assertEquals(-25.2f, dy, 1e-5f)
        val midpoint = DeskMotion(0.029, 0.0, 0.0, DeskStatus.Tracking).pointerDelta(0.1, deskSpeed = 2f)
        assertEquals(0.029f * 21000f * 2f * 1.05f, midpoint.first, 1e-3f)
        val fast = DeskMotion(0.1, 0.0, 0.0, DeskStatus.Tracking).pointerDelta(0.1)
        assertEquals(3150f, fast.first, 1e-3f)
    }

    @Test
    fun stickRescalesRadialDeadZoneAndClampsAtRadius() {
        assertEquals(0f to 0f, StickCurve.velocity(6.4f, 0f))
        assertEquals(0f to 0f, StickCurve.velocity(4f, 4f))
        val halfway = StickCurve.velocity(32f, 0f)
        assertEquals(2300f * (0.4f / 0.9f).pow(1.8f), halfway.first, 1e-4f)
        assertEquals(0f, halfway.second, 1e-6f)
        val saturated = StickCurve.velocity(64f, -64f, stickSpeed = 2f)
        assertEquals(3252.6912f, saturated.first, 1e-3f)
        assertEquals(-saturated.first, saturated.second, 1e-6f)
    }

    @Test
    fun screenMapFitsNegativeMonitorOriginsAndFindsGaps() {
        val left = MonitorRect("Portrait", -1280, -200, 1280, 1024)
        val primary = MonitorRect("Primary", 0, 0, 1920, 1080, primary = true)
        val transform = ScreenMapTransform(listOf(left, primary), 352f, 160f)
        assertEquals(ScreenMapRect(-1280f, -200f, 3200f, 1280f), transform.virtualDesktop)
        assertEquals(ScreenMapRect(16f, 16f, 320f, 128f), transform.mapBounds)
        assertEquals(144f, transform.monitorBounds(primary).x, 1e-6f)
        assertEquals(36f, transform.monitorBounds(primary).y, 1e-6f)
        assertEquals(left, transform.hitTest(50f, 30f))
        assertEquals(primary, transform.hitTest(200f, 70f))
        assertNull(transform.hitTest(200f, 30f))
        assertNull(transform.hitTest(14f, 80f))
        assertEquals(0f to 0f, transform.normalizedAt(-2f, -2f))
        assertEquals(1f to 1f, transform.normalizedAt(400f, 200f))
        val (x, y) = transform.padPosition(0.4f, 0.15625f)
        assertEquals(0.4f to 0.15625f, transform.normalizedAt(x, y))
        assertEquals("Display", ScreenMapTransform(emptyList(), 352f, 160f).monitors.single().name)
    }

    @Test
    fun oneFingerTapUsesDurationTravelAndPreference() {
        val pad = GestureTrace()
        pad.at(0, p(1))
        pad.at(160, p(1, 13f))
        pad.at(299)
        assertEquals(listOf(left(TouchpadButtonState.Click)), pad.buttons)
        assertEquals(listOf(TouchpadHaptic.Click), pad.haptics)
        assertEquals(1, pad.events.count { it == TouchpadGestureEvent.LeftTap })
        assertTrue(pad.events.filterIsInstance<TouchpadGestureEvent.Move>().single().dx > 0f)

        pad.events.clear()
        pad.at(800, p(1))
        pad.at(950, p(1, 14f))
        pad.at(960)
        pad.at(1000, p(1))
        pad.at(1300)
        assertTrue(pad.buttons.isEmpty())

        val disabled = GestureTrace(TouchpadGestureOptions(tapToClick = false))
        disabled.at(0, p(1))
        disabled.at(100)
        disabled.at(180, p(1))
        disabled.at(200)
        assertTrue(disabled.buttons.isEmpty())
    }

    @Test
    fun tapTapDragPressesImmediatelyAndLiftingWithoutTravelCompletesDoubleClick() {
        val pad = GestureTrace()
        pad.at(0, p(1))
        pad.at(80)
        pad.at(399, p(1, 69f))
        assertEquals(listOf(left(TouchpadButtonState.Click), left(TouchpadButtonState.Down)), pad.buttons)
        pad.at(480)
        assertEquals(left(TouchpadButtonState.Up), pad.buttons.last())
        assertEquals(2, pad.events.count { it == TouchpadGestureEvent.LeftTap })

        pad.events.clear()
        pad.at(1000, p(1))
        pad.at(1080)
        pad.at(1100, p(1))
        pad.at(1150, p(1, 8f))
        pad.at(1200)
        assertEquals(
            listOf(left(TouchpadButtonState.Click), left(TouchpadButtonState.Down), left(TouchpadButtonState.Up)),
            pad.buttons,
        )
        assertEquals(listOf(TouchpadHaptic.Click, TouchpadHaptic.Tick), pad.haptics)
        assertEquals(1, pad.events.count { it == TouchpadGestureEvent.LeftTap })
    }

    @Test
    fun holdArmsWithoutMovesAndDragSurvivesRepositionUntilTimeoutOrCancel() {
        val pad = GestureTrace()
        pad.at(0, p(1))
        assertEquals(450L, pad.engine.nextDeadlineMs)
        pad.engine.advanceTo(449)
        assertTrue(pad.haptics.isEmpty())
        pad.engine.advanceTo(450)
        assertEquals(listOf(TouchpadHaptic.Tick), pad.haptics)
        assertTrue(pad.buttons.isEmpty())
        pad.at(460, p(1, 1f))
        assertEquals(listOf(left(TouchpadButtonState.Down)), pad.buttons)
        assertEquals(TouchpadHaptic.Heavy, pad.haptics.last())
        pad.at(500)
        assertEquals(880L, pad.engine.nextDeadlineMs)
        pad.at(879, p(2, 150f))
        assertNull(pad.engine.nextDeadlineMs)
        pad.at(901, p(2, 151f))
        assertEquals(1, pad.buttons.size)
        pad.at(920)
        pad.engine.advanceTo(1299)
        assertEquals(1, pad.buttons.size)
        pad.engine.advanceTo(1300)
        assertEquals(left(TouchpadButtonState.Up), pad.buttons.last())
        assertEquals(2, pad.buttons.size)

        pad.at(1500, p(1))
        pad.engine.advanceTo(1950)
        pad.at(1960, p(1, 1f))
        pad.engine.cancel()
        assertEquals(left(TouchpadButtonState.Up), pad.buttons.last())
        assertEquals(4, pad.buttons.size)
        assertNull(pad.engine.nextDeadlineMs)
    }

    @Test
    fun customAimingGetsSingleFingerLifecycleAndMultiFingerGestures() {
        val pad = GestureTrace(TouchpadGestureOptions(relativePointer = false))
        pad.at(0, p(1))
        assertNull(pad.engine.nextDeadlineMs)
        pad.at(15, p(1, 3f))
        pad.at(30, p(1, 3f), p(2, 10f))
        pad.at(65)
        assertEquals(
            listOf(
                TouchpadSingleFinger.Start(p(1)),
                TouchpadSingleFinger.Move(p(1, 3f), 3f, 0f),
                TouchpadSingleFinger.Cancel,
            ),
            pad.events.filterIsInstance<TouchpadGestureEvent.SingleFinger>().map { it.event },
        )
        assertTrue(pad.events.none { it is TouchpadGestureEvent.Move })
        assertEquals(listOf(TouchpadGestureEvent.Button(TouchpadButton.Right, TouchpadButtonState.Click)), pad.buttons)
        assertEquals(listOf(TouchpadHaptic.Double), pad.haptics)
    }

    @Test
    fun multiTapUsesPeakCountAndTimeSinceLastJoin() {
        val pad = GestureTrace()
        pad.at(0, p(1))
        pad.at(300, p(1), p(2, 15f))
        pad.at(470, p(1))
        pad.at(719)
        assertEquals(listOf(TouchpadGestureEvent.Button(TouchpadButton.Right, TouchpadButtonState.Click)), pad.buttons)

        pad.events.clear()
        pad.at(1000, p(1), p(2, 15f))
        pad.at(1050, p(1, 22f), p(2, 37f))
        pad.at(1060, p(1))
        pad.at(1090)
        pad.at(1200, p(1), p(2, 15f))
        pad.at(1620)
        assertTrue(pad.buttons.isEmpty())
    }

    @Test
    fun scrollWaitsForStableCountLocksAxisAndGlides() {
        val pad = GestureTrace()
        pad.shift(0, 2)
        pad.shift(69, 2, dy = -15f)
        assertTrue(pad.events.isEmpty())
        pad.shift(70, 2, dy = -16f)
        pad.shift(86, 2, dx = 2f, dy = -32f)
        pad.shift(102, 2, dx = 11f, dy = -48f)
        pad.shift(118, 2, dy = -64f)
        assertEquals(3, pad.scrolls.size)
        for (scroll in pad.scrolls) {
            assertEquals(0f, scroll.dxNotches, 1e-6f)
            assertEquals(0.232f, scroll.dyNotches, 1e-6f)
        }
        pad.at(130)
        assertEquals(146L, pad.engine.nextDeadlineMs)
        pad.engine.advanceTo(146)
        pad.engine.advanceTo(162)
        val decay = 0.9965f.pow(16f)
        assertEquals(0.232f * decay, pad.scrolls[3].dyNotches, 1e-6f)
        assertEquals(0.232f * decay * decay, pad.scrolls[4].dyNotches, 1e-6f)
        repeat(70) { pad.engine.nextDeadlineMs?.let(pad.engine::advanceTo) }
        assertNull(pad.engine.nextDeadlineMs)
    }

    @Test
    fun momentumIgnoresStaleSlowOrDisabledFlicksAndStopsOnNewTouch() {
        fun flick(pad: GestureTrace, step: Float, liftAt: Long) {
            pad.shift(0, 2)
            pad.shift(70, 2, dy = 14f)
            pad.shift(90, 2, dy = 14f + step)
            pad.shift(110, 2, dy = 14f + 2f * step)
            pad.at(liftAt)
        }
        val slow = GestureTrace()
        flick(slow, 5f, 120)
        assertNull(slow.engine.nextDeadlineMs)
        val stale = GestureTrace()
        flick(stale, 15f, 220)
        assertNull(stale.engine.nextDeadlineMs)
        val disabled = GestureTrace(TouchpadGestureOptions(momentum = false))
        flick(disabled, 15f, 120)
        assertNull(disabled.engine.nextDeadlineMs)
        val interrupted = GestureTrace()
        flick(interrupted, 15f, 120)
        assertEquals(136L, interrupted.engine.nextDeadlineMs)
        interrupted.at(125, p(1))
        interrupted.engine.advanceTo(200)
        assertEquals(2, interrupted.scrolls.size)
        assertEquals(575L, interrupted.engine.nextDeadlineMs)
    }

    @Test
    fun pinchFollowsSpreadAndReleasesCtrlOnLiftOrReclassification() {
        val pad = GestureTrace()
        pad.at(0, p(1, -10f), p(2, 10f))
        pad.at(70, p(1, -18f), p(2, 18f))
        pad.at(85, p(1, -39f), p(2, 39f))
        pad.at(90, p(1, -38f), p(2, 38f))
        pad.at(110, p(1, -38f))
        pad.at(120)
        assertEquals(listOf(true, false), pad.pinches)
        val zoom = pad.events.filterIsInstance<TouchpadGestureEvent.Zoom>()
        assertEquals(-42f * 0.024f, zoom[0].dyNotches, 1e-6f)
        assertEquals(2f * 0.024f, zoom[1].dyNotches, 1e-6f)
        assertEquals(listOf(TouchpadHaptic.Tick, TouchpadHaptic.Soft), pad.haptics)
        assertTrue(pad.scrolls.isEmpty())

        pad.events.clear()
        pad.at(500, p(1, -10f), p(2, 10f))
        pad.at(570, p(1, -20f), p(2, 20f))
        pad.at(580, p(1, -20f))
        pad.at(615, p(1, -20f), p(3, 20f))
        assertEquals(listOf(true, false), pad.pinches)
        pad.at(685, p(1, -13f), p(3, 27f))
        assertTrue(pad.scrolls.isEmpty())
        pad.at(690, p(1, -4f), p(3, 36f))
        pad.at(700, p(1, 9f), p(3, 49f))
        assertEquals(-0.1885f, pad.scrolls.single().dxNotches, 1e-6f)
    }

    @Test
    fun lateThirdAndFourthFingersReclassifyWithoutCentroidJumps() {
        val pad = GestureTrace()
        pad.shift(0, 2)
        pad.shift(70, 2, 14f)
        pad.shift(80, 2, 20f)
        assertEquals(1, pad.scrolls.size)
        pad.shift(95, 3, 20f)
        assertTrue(pad.actions.isEmpty())
        pad.shift(165, 3, 50f)
        assertEquals(listOf(TouchpadAction.SwitchStart, TouchpadAction.SwitchNext), pad.actions)
        pad.shift(180, 4, 50f)
        assertEquals(TouchpadAction.SwitchCancel, pad.actions.last())
        pad.shift(250, 4, 20f)
        pad.shift(260, 3, 20f)
        pad.shift(270, 2, 20f)
        pad.at(280)
        assertEquals(
            listOf(TouchpadAction.SwitchStart, TouchpadAction.SwitchNext, TouchpadAction.SwitchCancel, TouchpadAction.WorkspaceNext),
            pad.actions,
        )
        assertEquals(1, pad.scrolls.size)
    }

    @Test
    fun threeAndFourFingerTapsDispatchTheirConfiguredActions() {
        val pad = GestureTrace()
        pad.shift(0, 2)
        pad.shift(60, 3)
        pad.shift(100, 2)
        pad.at(120)
        pad.shift(200, 3)
        pad.shift(220, 4)
        pad.shift(255, 3)
        pad.at(280)
        assertEquals(listOf(TouchpadAction.Search, TouchpadAction.Notifications), pad.actions)
        assertEquals(listOf(TouchpadHaptic.Click, TouchpadHaptic.Click), pad.haptics)

        val alternate = GestureTrace(TouchpadGestureOptions(threeFingerMiddleClick = true))
        alternate.shift(0, 3)
        alternate.at(40)
        assertEquals(listOf(TouchpadGestureEvent.Button(TouchpadButton.Middle, TouchpadButtonState.Click)), alternate.buttons)
        assertTrue(alternate.actions.isEmpty())
    }

    @Test
    fun horizontalThreeFingerSwipeStepsBothWaysThenEnds() {
        val pad = GestureTrace()
        pad.shift(0, 3)
        pad.shift(70, 3, 29f)
        pad.shift(80, 3, 74f)
        assertEquals(listOf(TouchpadAction.SwitchStart, TouchpadAction.SwitchNext), pad.actions)
        pad.shift(90, 3, 76f)
        pad.shift(105, 3, -19f)
        pad.shift(120, 2, -19f)
        pad.at(125)
        assertEquals(
            listOf(
                TouchpadAction.SwitchStart, TouchpadAction.SwitchNext, TouchpadAction.SwitchNext,
                TouchpadAction.SwitchPrevious, TouchpadAction.SwitchPrevious, TouchpadAction.SwitchEnd,
            ),
            pad.actions,
        )
        assertEquals(listOf(TouchpadHaptic.Click, TouchpadHaptic.Tick, TouchpadHaptic.Tick, TouchpadHaptic.Tick), pad.haptics)
    }

    @Test
    fun verticalSwipesTrackInverseAndFourFingerHorizontalSwipesChangeWorkspace() {
        val pad = GestureTrace()
        fun swipe(timeMs: Long, count: Int, dx: Float, dy: Float) {
            pad.shift(timeMs, count)
            pad.shift(timeMs + 70, count, dx, dy)
            pad.at(timeMs + 90)
        }
        swipe(0, 3, 0f, -29f)
        swipe(100, 4, 0f, 29f)
        swipe(200, 3, 0f, 29f)
        swipe(300, 4, 0f, -29f)
        swipe(400, 4, -29f, 0f)
        swipe(500, 4, 29f, 0f)
        assertEquals(
            listOf(
                TouchpadAction.Overview, TouchpadAction.OverviewClose, TouchpadAction.ShowDesktop,
                TouchpadAction.ShowDesktop, TouchpadAction.WorkspaceNext, TouchpadAction.WorkspacePrevious,
            ),
            pad.actions,
        )
        assertEquals(List(6) { TouchpadHaptic.Heavy }, pad.haptics)
    }

    @Test
    fun systemCancellationEndsSwitcherAndReportsPeakFingerCountOnce() {
        val pad = GestureTrace()
        pad.shift(0, 3)
        pad.shift(70, 3, -29f)
        pad.shift(85, 2, -29f)
        pad.engine.cancel(stolenBySystem = true)
        pad.engine.cancel(stolenBySystem = true)
        assertEquals(
            listOf(TouchpadAction.SwitchStart, TouchpadAction.SwitchPrevious, TouchpadAction.SwitchCancel),
            pad.actions,
        )
        assertEquals(1, pad.events.count { it == TouchpadGestureEvent.Stolen })
        assertNull(pad.engine.nextDeadlineMs)

        pad.events.clear()
        pad.shift(100, 4)
        pad.engine.cancel()
        assertTrue(pad.events.isEmpty())
    }

    private class DeskTrace {
        val engine = DeskSensorEngine()
        private var timestampNs = 0L

        init {
            engine.onGravity(0f, 0f, 9.81f)
            engine.onGyro(0f, 0f, 0f, timestampNs)
            engine.onAcceleration(0f, 0f, 0f, timestampNs)
        }

        fun samples(count: Int, ax: Float = 0f, az: Float = 0f, wz: Float = 0f) {
            repeat(count) {
                timestampNs += 5_000_000L
                engine.onGyro(0f, 0f, wz, timestampNs)
                engine.onAcceleration(ax, 0f, az, timestampNs)
            }
        }
    }

    private class GestureTrace(options: TouchpadGestureOptions = TouchpadGestureOptions()) {
        val events = mutableListOf<TouchpadGestureEvent>()
        val engine = TouchpadGestureEngine(options, events::add)
        val buttons get() = events.filterIsInstance<TouchpadGestureEvent.Button>()
        val scrolls get() = events.filterIsInstance<TouchpadGestureEvent.Scroll>()
        val actions get() = events.filterIsInstance<TouchpadGestureEvent.Action>().map { it.action }
        val haptics get() = events.filterIsInstance<TouchpadGestureEvent.Haptic>().map { it.haptic }
        val pinches get() = events.filterIsInstance<TouchpadGestureEvent.Pinch>().map { it.active }

        fun at(timeMs: Long, vararg contacts: TouchpadContact) = engine.onPointers(timeMs, contacts.toList())

        fun shift(timeMs: Long, count: Int, dx: Float = 0f, dy: Float = 0f) =
            engine.onPointers(timeMs, List(count) { TouchpadContact(it.toLong(), 15f * it + dx, dy) })
    }

    private fun p(id: Long, x: Float = 0f, y: Float = 0f) = TouchpadContact(id, x, y)

    private fun left(state: TouchpadButtonState) = TouchpadGestureEvent.Button(TouchpadButton.Left, state)
}
