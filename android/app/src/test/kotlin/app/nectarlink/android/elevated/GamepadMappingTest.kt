// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import app.nectarlink.android.elevated.GamepadMapping.A
import app.nectarlink.android.elevated.GamepadMapping.B
import app.nectarlink.android.elevated.GamepadMapping.BACK
import app.nectarlink.android.elevated.GamepadMapping.DPAD_DOWN
import app.nectarlink.android.elevated.GamepadMapping.DPAD_LEFT
import app.nectarlink.android.elevated.GamepadMapping.DPAD_RIGHT
import app.nectarlink.android.elevated.GamepadMapping.DPAD_UP
import app.nectarlink.android.elevated.GamepadMapping.HAT_NONE
import app.nectarlink.android.elevated.GamepadMapping.LEFT_SHOULDER
import app.nectarlink.android.elevated.GamepadMapping.LEFT_THUMB
import app.nectarlink.android.elevated.GamepadMapping.RIGHT_SHOULDER
import app.nectarlink.android.elevated.GamepadMapping.RIGHT_THUMB
import app.nectarlink.android.elevated.GamepadMapping.START
import app.nectarlink.android.elevated.GamepadMapping.X
import app.nectarlink.android.elevated.GamepadMapping.Y
import org.junit.Assert.assertEquals
import org.junit.Test

class GamepadMappingTest {
    @Test
    fun `each XInput button lands on its Linux gamepad button`() {
        // HID button n is BTN_GAMEPAD + n - 1: A, B, C, X, Y, Z, TL, TR, TL2, TR2, Select, Start, Mode, ThumbL, ThumbR.
        val expected = mapOf(
            A to 0, B to 1, X to 3, Y to 4,
            LEFT_SHOULDER to 6, RIGHT_SHOULDER to 7,
            BACK to 10, START to 11,
            LEFT_THUMB to 13, RIGHT_THUMB to 14,
        )
        for ((xinput, bit) in expected) {
            assertEquals("button %#06x".format(xinput), 1 shl bit, GamepadMapping.hidButtons(xinput))
        }
    }

    @Test
    fun `the D-pad is not reported as buttons`() {
        assertEquals(0, GamepadMapping.hidButtons(DPAD_UP or DPAD_DOWN or DPAD_LEFT or DPAD_RIGHT))
    }

    @Test
    fun `buttons held together stay together`() {
        assertEquals((1 shl 0) or (1 shl 1) or (1 shl 7), GamepadMapping.hidButtons(A or B or RIGHT_SHOULDER))
    }

    @Test
    fun `the hat goes clockwise from up`() {
        assertEquals(0, GamepadMapping.hat(DPAD_UP))
        assertEquals(1, GamepadMapping.hat(DPAD_UP or DPAD_RIGHT))
        assertEquals(2, GamepadMapping.hat(DPAD_RIGHT))
        assertEquals(3, GamepadMapping.hat(DPAD_DOWN or DPAD_RIGHT))
        assertEquals(4, GamepadMapping.hat(DPAD_DOWN))
        assertEquals(5, GamepadMapping.hat(DPAD_DOWN or DPAD_LEFT))
        assertEquals(6, GamepadMapping.hat(DPAD_LEFT))
        assertEquals(7, GamepadMapping.hat(DPAD_UP or DPAD_LEFT))
    }

    @Test
    fun `no D-pad or opposite directions is no hat`() {
        assertEquals(HAT_NONE, GamepadMapping.hat(0))
        assertEquals(HAT_NONE, GamepadMapping.hat(A))
        assertEquals(HAT_NONE, GamepadMapping.hat(DPAD_UP or DPAD_DOWN))
        assertEquals(2, GamepadMapping.hat(DPAD_UP or DPAD_DOWN or DPAD_RIGHT))
    }

    @Test
    fun `hat axes point right and down`() {
        assertEquals(0 to -1, GamepadMapping.hatAxes(DPAD_UP))
        assertEquals(1 to 1, GamepadMapping.hatAxes(DPAD_DOWN or DPAD_RIGHT))
        assertEquals(0 to 0, GamepadMapping.hatAxes(DPAD_LEFT or DPAD_RIGHT))
    }

    @Test
    fun `pushing a stick forward is up on Android`() {
        // XInput: forward is positive. Android: up is negative.
        assertEquals(-32767, GamepadMapping.stickY(32767))
        assertEquals(32767, GamepadMapping.stickY(-32768))
        assertEquals(0, GamepadMapping.stickY(0))
    }
}
