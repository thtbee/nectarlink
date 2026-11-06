// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FrameQuadTest {
    @Test
    fun `rotations preserve the existing UV mapping and letterbox upright portrait frames`() {
        val expected = listOf(
            floatArrayOf(0f, 0f, 1f, 0f, 0f, 1f, 1f, 1f),
            floatArrayOf(0f, 1f, 0f, 0f, 1f, 1f, 1f, 0f),
            floatArrayOf(1f, 1f, 0f, 1f, 1f, 0f, 0f, 0f),
            floatArrayOf(1f, 0f, 1f, 1f, 0f, 0f, 0f, 1f),
        )
        for (rotation in 0..3) for (swapsAxes in listOf(false, true)) for (mirror in listOf(false, true)) {
            val quad = FrameQuad(1920, 1080, 1920, 1080)
            assertTrue(quad.update(rotation * 90, swapsAxes, mirror))
            val portrait = swapsAxes xor (rotation % 2 != 0)
            val width = (if (portrait) 81f / 256f else 1f) * (if (mirror) -1f else 1f)
            val values = FloatArray(16)
            for (i in 0..3) {
                values[i * 4] = if (i % 2 == 0) -width else width
                values[i * 4 + 1] = if (i < 2) -1f else 1f
                values[i * 4 + 2] = expected[rotation][i * 2]
                values[i * 4 + 3] = expected[rotation][i * 2 + 1]
            }
            val actual = FloatArray(16)
            quad.buffer.duplicate().get(actual)
            assertArrayEquals(values, actual, 0.00001f)
        }
    }

    @Test
    fun `caches unchanged transforms but refreshes mirror rotation and axis changes`() {
        val quad = FrameQuad(1920, 1080, 1920, 1080)
        assertTrue(quad.update(0, false, false))
        assertFalse(quad.update(360, false, false))
        assertTrue(quad.update(0, false, true))
        assertFalse(quad.update(0, false, true))
        assertTrue(quad.update(-90, false, true))
        assertFalse(quad.update(270, false, true))
        assertTrue(quad.update(270, true, true))
    }

    @Test
    fun `wide source letterboxes vertically without stretching`() {
        val quad = FrameQuad(3840, 1080, 1920, 1080)
        quad.update(0, false, false)
        assertArrayEquals(floatArrayOf(-1f, -0.5f, 0f, 0f, 1f, -0.5f, 1f, 0f,
            -1f, 0.5f, 0f, 1f, 1f, 0.5f, 1f, 1f), FloatArray(16).also { quad.buffer.duplicate().get(it) }, 0.00001f)
    }
}
