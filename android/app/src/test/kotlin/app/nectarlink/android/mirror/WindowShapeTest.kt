// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class WindowShapeTest {
    // A Galaxy A35's screen.
    private val phoneW = 1080
    private val phoneH = 2340

    private fun fit(w: Int, h: Int, max: Int = 1920) = WindowShape.fit(w, h, max, phoneW, phoneH)

    private fun assertAligned(size: Pair<Int, Int>) {
        assertEquals("width $size", 0, size.first % 16)
        assertEquals("height $size", 0, size.second % 16)
    }

    @Test
    fun aWindowLikeThePhoneGetsThePhonesExactShape() {
        // Rounding 1080x2340 to multiples of 16 gave 880x1920 (2.18), which
        // Samsung's Camera refused to start on.
        val size = fit(1080, 2340)
        assertEquals(864 to 1872, size)
        assertEquals(2340.0 / 1080, size.second.toDouble() / size.first, 1e-9)
    }

    @Test
    fun theCrashingSizesBecomeExactShapes() {
        for ((w, h) in listOf(1920 to 880, 1472 to 672, 672 to 1472)) {
            val (fw, fh) = fit(w, h)
            assertEquals("$w x $h keeps its orientation", w > h, fw > fh)
            assertEquals(13.0 / 6, maxOf(fw, fh).toDouble() / minOf(fw, fh), 1e-9)
            assertAligned(fw to fh)
            assertTrue("fits $w x $h", fw <= w && fh <= h)
        }
    }

    @Test
    fun commonShapesStayExact() {
        assertEquals(1792 to 1008, fit(1920, 1080)) // 16:9
        assertEquals(1024 to 768, fit(1024, 768)) // 4:3
        assertEquals(800 to 800, fit(810, 800)) // nearly square
    }

    @Test
    fun aLargeWindowIsScaledToTheLimitAndASmallOneStaysUsable() {
        val big = fit(3840, 2160, max = 1280)
        assertTrue(big.first <= 1280 && big.second <= 1280)
        assertEquals(16.0 / 9, big.first.toDouble() / big.second, 1e-9)
        val small = fit(100, 100)
        assertTrue(small.first >= 240 && small.second >= 240)
        assertAligned(small)
    }

    @Test
    fun inputCoordinatesAndScrollDeltasClampOvershootWithinScreenBounds() {
        val (x0, y0) = clampScreenPoint(-0.4f, -0.5f, 1080f, 2340f)
        assertEquals(0f, x0, 0f)
        assertEquals(0f, y0, 0f)

        val (x1, y1) = clampScreenPoint(1.4f, 1.5f, 1080f, 2340f)
        assertEquals(1079f, x1, 0f)
        assertEquals(2339f, y1, 0f)

        val dxLeftEdge = clampScrollDelta(1f, -500f, 1080f)
        assertEquals(0f, dxLeftEdge, 0f)
        val dxRightEdge = clampScrollDelta(1079f, 500f, 1080f)
        assertEquals(0f, dxRightEdge, 0f)
    }
}
