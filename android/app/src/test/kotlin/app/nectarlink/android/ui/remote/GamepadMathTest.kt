// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class GamepadMathTest {
    @Test
    fun stickIsCentredInsideTheDeadZone() {
        assertEquals(0 to 0, GamepadMath.stick(3f, 3f, 100f))
        assertEquals(0 to 0, GamepadMath.stick(0f, 0f, 100f))
        assertEquals(0 to 0, GamepadMath.stick(50f, 0f, 0f))
    }

    @Test
    fun stickPointsUpWhenTheThumbGoesUpTheScreen() {
        assertEquals(0 to 32767, GamepadMath.stick(0f, -100f, 100f))
        assertEquals(0 to -32767, GamepadMath.stick(0f, 100f, 100f))
        assertEquals(32767 to 0, GamepadMath.stick(100f, 0f, 100f))
        assertEquals(-32767 to 0, GamepadMath.stick(-100f, 0f, 100f))
    }

    @Test
    fun stickStaysAtTheRimPastIt() {
        assertEquals(32767 to 0, GamepadMath.stick(400f, 0f, 100f))
        val (x, y) = GamepadMath.stick(300f, -300f, 100f)
        assertEquals(23170, x)
        assertEquals(23170, y)
    }

    @Test
    fun stickStretchesWhatsPastTheDeadZone() {
        val (half, _) = GamepadMath.stick(54f, 0f, 100f)
        assertEquals(16384, half, 200)
    }

    @Test
    fun dpadPressesOneOrTwoDirections() {
        assertEquals(0, GamepadMath.dpad(2f, 2f, 10f))
        assertEquals(PadSnapshot.RIGHT, GamepadMath.dpad(50f, 0f, 10f))
        assertEquals(PadSnapshot.UP, GamepadMath.dpad(0f, -50f, 10f))
        assertEquals(PadSnapshot.LEFT, GamepadMath.dpad(-50f, 5f, 10f))
        assertEquals(PadSnapshot.DOWN, GamepadMath.dpad(0f, 50f, 10f))
        assertEquals(PadSnapshot.UP or PadSnapshot.RIGHT, GamepadMath.dpad(40f, -40f, 10f))
        assertEquals(PadSnapshot.DOWN or PadSnapshot.LEFT, GamepadMath.dpad(-40f, 40f, 10f))
    }

    private fun assertEquals(expected: Int, actual: Int, tolerance: Int) {
        assertTrue("$actual is not within $tolerance of $expected", kotlin.math.abs(expected - actual) <= tolerance)
    }
}
