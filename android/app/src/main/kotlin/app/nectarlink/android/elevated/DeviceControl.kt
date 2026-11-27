// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.os.Handler
import android.os.Looper
import android.view.KeyEvent
import app.nectarlink.core.DeviceInput
import app.nectarlink.core.MirrorInputEvent
import kotlin.math.roundToInt

/**
 * This tablet (or phone) as the target of another one's touchpad and
 * keyboard (#651), through the Elevated helper: a pointer of its own that
 * the touchpad moves (the one the PC's mouse uses beside a PC), clicks and
 * the wheel on it, named keys, and Unicode text (pasted when it isn't plain
 * ASCII). The pointer shows with the first input and goes after a minute
 * without any.
 */
object DeviceControl {
    private const val IDLE_MS = 60_000L

    private val main = Handler(Looper.getMainLooper())
    private val lock = Any()

    /** Logical pixels per pointer count, once the pointer shows. */
    private var scale = 0f
    private var carryX = 0f
    private var carryY = 0f
    private var carryWheel = 0f
    /** Bits: 1 left, 2 right, 4 middle. */
    private var buttons = 0

    private val idle = Runnable { stop() }

    /** Handles [input]; false when Elevated can't take it now. Not on the main thread. */
    fun handle(input: DeviceInput): Boolean {
        main.removeCallbacks(idle)
        main.postDelayed(idle, IDLE_MS)
        return when (input) {
            is DeviceInput.Move -> move(input.dx, input.dy)
            is DeviceInput.Button -> button(input.button, input.action)
            is DeviceInput.Scroll -> scroll(input.dy)
            is DeviceInput.Text -> Elevated.handle(MirrorInputEvent.Text(input.text))
            is DeviceInput.Key -> key(input.key, input.mods)
        }
    }

    /** The pointer's scale, showing it first when it isn't there yet. */
    private fun ensurePointer(): Float? = synchronized(lock) {
        if (scale > 0f) return scale
        val info = Elevated.pointerStart("left", 0.5f) ?: return null
        scale = info.scale
        carryX = 0f
        carryY = 0f
        carryWheel = 0f
        buttons = 0
        scale
    }

    private fun move(dx: Float, dy: Float): Boolean {
        val scale = ensurePointer() ?: return false
        val (cx, cy) = synchronized(lock) {
            val wantX = dx / scale + carryX
            val wantY = dy / scale + carryY
            val countX = wantX.toInt()
            val countY = wantY.toInt()
            carryX = wantX - countX
            carryY = wantY - countY
            countX to countY
        }
        if (cx == 0 && cy == 0) return true
        return Elevated.pointerMove(cx, cy, buttons, 0)
    }

    private fun button(name: String, action: String): Boolean {
        ensurePointer() ?: return false
        val bit = when (name) {
            "left" -> 1
            "right" -> 2
            "middle" -> 4
            else -> return true
        }
        return when (action) {
            "down" -> Elevated.pointerMove(0, 0, synchronized(lock) { buttons = buttons or bit; buttons }, 0)
            "up" -> Elevated.pointerMove(0, 0, synchronized(lock) { buttons = buttons and bit.inv(); buttons }, 0)
            "click" -> {
                val held = synchronized(lock) { buttons }
                Elevated.pointerMove(0, 0, held or bit, 0) && Elevated.pointerMove(0, 0, held, 0)
            }
            else -> true
        }
    }

    /** Wheel notches; positive [dy] scrolls down, and Android's wheel up is positive. */
    private fun scroll(dy: Float): Boolean {
        ensurePointer() ?: return false
        val notches = synchronized(lock) {
            val want = -dy + carryWheel
            val whole = want.roundToInt().coerceIn(-Byte.MAX_VALUE.toInt(), Byte.MAX_VALUE.toInt())
            carryWheel = want - whole
            whole
        }
        if (notches == 0) return true
        return Elevated.pointerMove(0, 0, buttons, notches)
    }

    private fun key(name: String, mods: List<String>): Boolean {
        if (name == "space" && mods.isEmpty()) return Elevated.handle(MirrorInputEvent.Text(" "))
        // Undo has no key of its own on Android: Ctrl+Z, as text fields take it.
        val (code, undo) = if (name == "undo") KeyEvent.KEYCODE_Z to true else (keyCode(name) ?: return true) to false
        var meta = if (undo) KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON else 0
        if ("ctrl" in mods) meta = meta or KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON
        if ("alt" in mods) meta = meta or KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON
        if ("shift" in mods) meta = meta or KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON
        if ("win" in mods) meta = meta or KeyEvent.META_META_ON or KeyEvent.META_META_LEFT_ON
        return Elevated.keyCode(code, meta)
    }

    /** The tablet's key code for a named key from the phone's keyboard (`remote_keys`). */
    internal fun keyCode(name: String): Int? = when (name) {
        "enter" -> KeyEvent.KEYCODE_ENTER
        "backspace" -> KeyEvent.KEYCODE_DEL
        "delete" -> KeyEvent.KEYCODE_FORWARD_DEL
        "tab" -> KeyEvent.KEYCODE_TAB
        "escape" -> KeyEvent.KEYCODE_ESCAPE
        "space" -> KeyEvent.KEYCODE_SPACE
        "left" -> KeyEvent.KEYCODE_DPAD_LEFT
        "right" -> KeyEvent.KEYCODE_DPAD_RIGHT
        "up" -> KeyEvent.KEYCODE_DPAD_UP
        "down" -> KeyEvent.KEYCODE_DPAD_DOWN
        "home" -> KeyEvent.KEYCODE_MOVE_HOME
        "end" -> KeyEvent.KEYCODE_MOVE_END
        "page_up" -> KeyEvent.KEYCODE_PAGE_UP
        "page_down" -> KeyEvent.KEYCODE_PAGE_DOWN
        "copy" -> KeyEvent.KEYCODE_COPY
        "paste" -> KeyEvent.KEYCODE_PASTE
        "cut" -> KeyEvent.KEYCODE_CUT
        // The Windows key: the tablet's own home screen.
        "win" -> KeyEvent.KEYCODE_HOME
        else -> name.removePrefix("f").toIntOrNull()?.takeIf { name.startsWith("f") && it in 1..12 }
            ?.let { KeyEvent.KEYCODE_F1 + it - 1 }
    }

    /** The pointer goes: the other device stopped using this one. */
    fun stop() {
        val shown = synchronized(lock) {
            val was = scale > 0f
            scale = 0f
            buttons = 0
            was
        }
        if (shown) Elevated.pointerStop()
    }
}
