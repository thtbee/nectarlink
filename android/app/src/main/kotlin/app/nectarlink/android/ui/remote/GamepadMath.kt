// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import kotlin.math.atan2
import kotlin.math.hypot
import kotlin.math.roundToInt

/**
 * The phone's game controller at one moment, in the Xbox layout the PC plugs
 * in (`docs/protocol/remote.md` §3.1). Sticks run -32768 to 32767 with `y`
 * up; triggers 0 to 255.
 */
data class PadSnapshot(
    val buttons: Int = 0,
    val lt: Int = 0,
    val rt: Int = 0,
    val lx: Int = 0,
    val ly: Int = 0,
    val rx: Int = 0,
    val ry: Int = 0,
) {
    companion object {
        const val UP = 0x0001
        const val DOWN = 0x0002
        const val LEFT = 0x0004
        const val RIGHT = 0x0008
        const val START = 0x0010
        const val BACK = 0x0020
        const val LTHUMB = 0x0040
        const val RTHUMB = 0x0080
        const val LB = 0x0100
        const val RB = 0x0200
        const val GUIDE = 0x0400
        const val A = 0x1000
        const val B = 0x2000
        const val X = 0x4000
        const val Y = 0x8000
        const val DPAD = UP or DOWN or LEFT or RIGHT
    }
}

internal object GamepadMath {
    /** Below this share of the stick's reach, it reads as centred. */
    const val STICK_DEAD_ZONE = 0.08f

    /**
     * A stick pushed `(dx, dy)` screen pixels from where the thumb landed, with
     * `radius` pixels for all the way: `x` right and `y` up, -32767 to 32767.
     * Past the rim it stays at the rim in the same direction; the dead zone is
     * taken off and the rest stretched, so a small push still moves a little.
     */
    fun stick(dx: Float, dy: Float, radius: Float): Pair<Int, Int> {
        if (radius <= 0f) return 0 to 0
        val distance = hypot(dx, dy)
        val reach = distance / radius
        if (reach <= STICK_DEAD_ZONE) return 0 to 0
        val scaled = ((minOf(reach, 1f) - STICK_DEAD_ZONE) / (1f - STICK_DEAD_ZONE))
        val ux = dx / distance
        val uy = -dy / distance
        return (ux * scaled * 32767f).roundToInt().coerceIn(-32767, 32767) to
            (uy * scaled * 32767f).roundToInt().coerceIn(-32767, 32767)
    }

    /**
     * The d-pad buttons for a thumb `(dx, dy)` pixels from its centre: each
     * direction takes 135 degrees around it, so the 45 between two of them
     * press both (a diagonal). Within `deadZone` pixels of the centre, none.
     */
    fun dpad(dx: Float, dy: Float, deadZone: Float): Int {
        if (hypot(dx, dy) < deadZone) return 0
        val angle = Math.toDegrees(atan2(-dy, dx).toDouble())
        var bits = 0
        if (angle > -67.5 && angle < 67.5) bits = bits or PadSnapshot.RIGHT
        if (angle > 22.5 && angle < 157.5) bits = bits or PadSnapshot.UP
        if (angle > 112.5 || angle < -112.5) bits = bits or PadSnapshot.LEFT
        if (angle > -157.5 && angle < -22.5) bits = bits or PadSnapshot.DOWN
        return bits
    }
}
