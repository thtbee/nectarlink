// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.os.SystemClock
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import java.lang.reflect.Method
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest

/**
 * Elevated input: runs as the shell user (started through the phone's own
 * wireless debugging with `app_process`, like scrcpy's server) and injects
 * real touch, scroll and key events for the app. It listens on loopback
 * (apps may not connect to the shell's Unix sockets) and serves only a
 * client that first sends the random token it was started with, which only
 * the app knows. It ends when the shell that started it goes (the app went
 * away).
 *
 * Lines from the app, positions in pixels:
 *   D x y / M x y / U x y   finger down, moved, up
 *   S x y dx dy             wheel, in notches
 *   K keycode               a key, pressed and released
 *   T text                  typed text (what the keyboard map can type)
 *   C command...            a shell command of a fixed set (see `allowed`)
 */
object InputServer {
    @JvmStatic
    fun main(args: Array<String>) {
        val token = args.getOrNull(0) ?: return fail("no token")
        val injector = runCatching { Injector() }.getOrElse { return fail("can't inject input: $it") }
        val server = runCatching { ServerSocket(0, 1, InetAddress.getLoopbackAddress()) }.getOrElse { return fail("can't listen: $it") }
        println("ready ${server.localPort}")
        System.out.flush()
        while (true) {
            val client = runCatching { server.accept() }.getOrNull() ?: continue
            Thread { serve(client, token, injector) }.start()
        }
    }

    private fun fail(why: String) {
        System.err.println(why)
    }

    private fun serve(client: Socket, token: String, injector: Injector) {
        client.use {
            client.tcpNoDelay = true
            client.soTimeout = 0
            val reader = client.inputStream.bufferedReader()
            // Compared in constant time: the token is the only key.
            val given = runCatching { reader.readLine() }.getOrNull() ?: return
            if (!MessageDigest.isEqual(given.toByteArray(), token.toByteArray())) return
            while (true) {
                val line = reader.readLine() ?: return
                runCatching { injector.handle(line) }.onFailure { System.err.println("input failed: $it") }
            }
        }
    }

    private class Injector {
        private val manager: Any
        private val inject: Method
        private var downTime = 0L

        init {
            // Android 14 moved injection to InputManagerGlobal.
            val global = runCatching { Class.forName("android.hardware.input.InputManagerGlobal") }.getOrNull()
            val owner = global ?: Class.forName("android.hardware.input.InputManager")
            manager = owner.getDeclaredMethod("getInstance").invoke(null)!!
            inject = owner.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
        }

        fun handle(line: String) {
            val parts = line.split(' ', limit = 2)
            val rest = parts.getOrElse(1) { "" }
            when (parts[0]) {
                "D", "M", "U" -> {
                    val (x, y) = rest.split(' ').map { it.toFloat() }
                    touch(parts[0], x, y)
                }
                "S" -> {
                    val (x, y, dx, dy) = rest.split(' ').map { it.toFloat() }
                    scroll(x, y, dx, dy)
                }
                "K" -> key(rest.toInt())
                "T" -> type(rest)
                "C" -> if (rest in allowed) Runtime.getRuntime().exec(rest.split(' ').toTypedArray()).waitFor()
            }
        }

        private fun send(event: InputEvent) {
            // 0: asynchronous; the app doesn't wait for the target app.
            inject.invoke(manager, event, 0)
        }

        private fun touch(action: String, x: Float, y: Float) {
            val now = SystemClock.uptimeMillis()
            val kind = when (action) {
                "D" -> { downTime = now; MotionEvent.ACTION_DOWN }
                "M" -> MotionEvent.ACTION_MOVE
                else -> MotionEvent.ACTION_UP
            }
            val event = MotionEvent.obtain(downTime, now, kind, x, y, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            send(event)
            event.recycle()
        }

        private fun scroll(x: Float, y: Float, dx: Float, dy: Float) {
            val now = SystemClock.uptimeMillis()
            val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE })
            val coords = arrayOf(
                MotionEvent.PointerCoords().apply {
                    this.x = x
                    this.y = y
                    // Wheel down is a negative vertical scroll on Android.
                    setAxisValue(MotionEvent.AXIS_VSCROLL, -dy)
                    setAxisValue(MotionEvent.AXIS_HSCROLL, dx)
                },
            )
            val event = MotionEvent.obtain(
                now, now, MotionEvent.ACTION_SCROLL, 1, properties, coords, 0, 0, 1f, 1f, 0, 0, InputDevice.SOURCE_MOUSE, 0,
            )
            send(event)
            event.recycle()
        }

        private fun key(code: Int) {
            val now = SystemClock.uptimeMillis()
            for (action in intArrayOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
                send(KeyEvent(now, now, action, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD))
            }
        }

        private fun type(text: String) {
            val map = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD)
            // What the keyboard map can't type (most non-Latin letters, emoji) is skipped.
            for (c in text) {
                map.getEvents(charArrayOf(c))?.forEach { send(it) }
            }
        }

        /** The only shell commands the app may run through this. */
        private val allowed = setOf("cmd statusbar expand-notifications")
    }
}
