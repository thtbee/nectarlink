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
 * Any of these after `@display ` goes to that display (an app window's).
 *
 * A connection whose first line (after the token) is
 *   V width height dpi bitrate fps package
 * is an app window instead: the helper makes it a display of its own, runs
 * the app there and streams the display's video back on that connection
 * (see [AppDisplay]); a `K` line asks for a keyframe; closing the
 * connection closes the window.
 */
object InputServer {
    private val windows = java.util.Collections.synchronizedSet(LinkedHashSet<AppDisplay>())
    @Volatile private var serverSocket: ServerSocket? = null

    @JvmStatic
    fun main(args: Array<String>) {
        val token = args.getOrNull(0) ?: return fail("no token")
        val injector = runCatching { Injector() }.getOrElse { return fail("can't inject input: $it") }
        val server = runCatching { ServerSocket(0, 1, InetAddress.getLoopbackAddress()) }.getOrElse { return fail("can't listen: $it") }
        serverSocket = server
        println("ready ${server.localPort}")
        System.out.flush()
        while (!server.isClosed) {
            val client = runCatching { server.accept() }.getOrNull() ?: continue
            Thread { serve(client, token, injector) }.start()
        }
    }

    private fun fail(why: String) {
        System.err.println(why)
    }

    private fun shutdown() {
        runCatching { serverSocket?.close() }
        val open = synchronized(windows) { windows.toList().also { windows.clear() } }
        open.forEach { runCatching { it.stop() } }
        kotlin.system.exitProcess(0)
    }

    private fun serve(client: Socket, token: String, injector: Injector) {
        client.use {
            client.tcpNoDelay = true
            client.soTimeout = 0
            val reader = client.inputStream.bufferedReader()
            // Compared in constant time: the token is the only key.
            val given = runCatching { reader.nextLine() }.getOrNull() ?: return
            if (!MessageDigest.isEqual(given.toByteArray(), token.toByteArray())) return
            val first = runCatching { reader.nextLine() }.getOrNull()
            if (first != null && first.startsWith("V ")) return appWindow(client, first, reader)
            try {
                var line: String? = first
                while (line != null) {
                    val input = line
                    runCatching { injector.handle(input) }.onFailure { System.err.println("input failed: $it") }
                    line = runCatching { reader.nextLine() }.getOrNull()
                }
            } finally {
                shutdown()
            }
        }
    }

    /**
     * Reads until `\n` (ignoring bare `\r` as a line break so embedded `\r`
     * in typed text can never start a second command).
     */
    private fun java.io.Reader.nextLine(): String? {
        val sb = StringBuilder()
        while (true) {
            val ch = read()
            if (ch < 0) return if (sb.isEmpty()) null else sb.toString()
            if (ch == '\n'.code) return sb.toString()
            if (ch != '\r'.code) sb.append(ch.toChar())
        }
    }

    /** An app window on this connection, until it closes. */
    private fun appWindow(client: Socket, request: String, reader: java.io.BufferedReader) {
        val parts = request.split(' ')
        if (parts.size < 7) return
        val numbers = parts.subList(1, 6).map { it.toIntOrNull() ?: return }
        val (width, height, dpi, bitrate, fps) = numbers
        val pkg = parts.getOrNull(6)?.takeIf { PACKAGE.matches(it) } ?: return
        if (width !in 16..4096 || height !in 16..4096 || dpi !in 72..1000) return
        val window = AppDisplay(width, height, dpi, bitrate.coerceIn(500_000, 40_000_000), fps.coerceIn(1, 120))
        windows.add(window)
        // Further lines: keyframe or resize requests; the end of them closes the window.
        Thread {
            try {
                while (true) {
                    val line = reader.nextLine() ?: break
                    when {
                        line == "K" -> window.requestKeyframe()
                        line.startsWith("R ") -> {
                            val r = line.split(' ')
                            val rw = r.getOrNull(1)?.toIntOrNull()
                            val rh = r.getOrNull(2)?.toIntOrNull()
                            val rdpi = r.getOrNull(3)?.toIntOrNull()
                            if (rw != null && rh != null && rdpi != null &&
                                rw in 16..4096 && rh in 16..4096 && rdpi in 72..1000
                            ) {
                                window.resize(rw, rh, rdpi)
                            }
                        }
                    }
                }
            } catch (_: java.io.IOException) {
            }
            window.stop()
        }.start()
        try {
            runCatching { window.run(pkg, client.getOutputStream()) }.onFailure { System.err.println("app window ended: $it") }
        } finally {
            windows.remove(window)
            window.stop()
        }
    }

    private val PACKAGE = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

    private class Injector {
        private val manager: Any
        private val inject: Method
        private var downTime = 0L
        /** The display the line being handled is for (-1: the screen's own). */
        private var display = -1
        private val setDisplayId: Method? = runCatching {
            InputEvent::class.java.getMethod("setDisplayId", Int::class.javaPrimitiveType)
        }.getOrNull()

        init {
            // Android 14 moved injection to InputManagerGlobal.
            val global = runCatching { Class.forName("android.hardware.input.InputManagerGlobal") }.getOrNull()
            val owner = global ?: Class.forName("android.hardware.input.InputManager")
            manager = owner.getDeclaredMethod("getInstance").invoke(null)!!
            inject = owner.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
        }

        fun handle(full: String) {
            var line = full
            display = -1
            if (line.startsWith("@")) {
                val space = line.indexOf(' ')
                display = line.substring(1, space).toInt()
                line = line.substring(space + 1)
            }
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
                "C" -> if (allowedCommand(rest) && display < 0) {
                    Runtime.getRuntime().exec(rest.split(' ').toTypedArray()).waitFor()
                }
            }
        }

        private fun send(event: InputEvent) {
            if (display >= 0) setDisplayId?.invoke(event, display)
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

        private fun allowedCommand(cmd: String): Boolean {
            if (cmd in allowed) return true
            if (cmd.startsWith(BRIGHTNESS_PREFIX)) {
                val v = cmd.removePrefix(BRIGHTNESS_PREFIX).toIntOrNull()
                return v != null && v in 0..255
            }
            return false
        }

        /** The only shell commands the app may run through this. */
        private val allowed = setOf(
            "cmd statusbar expand-notifications",
            "svc wifi enable",
            "svc wifi disable",
            "svc bluetooth enable",
            "svc bluetooth disable",
        )

        private companion object {
            const val BRIGHTNESS_PREFIX = "settings put system screen_brightness "
        }
    }
}
