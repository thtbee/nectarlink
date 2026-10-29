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
 *   P 0 / P 1               physical screen power off (0) or normal (1)
 * Any of these after `@display ` goes to that display (an app window's).
 *
 * A connection whose first line (after the token) is
 *   V width height dpi bitrate fps package
 * is an app window instead: the helper makes it a display of its own, runs
 * the app there and streams the display's video back on that connection
 * (see [AppDisplay]); a `K` line asks for a keyframe; closing the
 * connection closes the window.
 */
/**
 * Compiles NectarLink ahead of time from its startup profile, as Android does
 * on its own only during idle maintenance. Asked once per version by
 * [Elevated]; allowed by name in the helper.
 */
internal const val COMPILE_APP_COMMAND = "cmd package compile -m speed-profile app.nectarlink"

object InputServer {
    private const val POWER_MODE_OFF = 0
    private const val POWER_MODE_NORMAL = 2
    private const val PKG = "app.nectarlink"

    private val windows = java.util.Collections.synchronizedSet(LinkedHashSet<AppDisplay>())
    @Volatile private var serverSocket: ServerSocket? = null
    @Volatile private var controlOut: java.io.Writer? = null
    @Volatile private var displayTurnedOff = false
    @Volatile private var savedBrightness: Int? = null

    @JvmStatic
    fun main(args: Array<String>) {
        val rawArg = args.getOrNull(0)
        val attach = rawArg == null || rawArg == "--attach"
        val token = if (attach) {
            ByteArray(24).also(java.security.SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
        } else {
            rawArg
        }
        grantAppOps()
        startClipboardWatcher()
        val injector = runCatching { Injector() }.getOrElse { return fail("can't inject input: $it") }
        val server = runCatching { ServerSocket(0, 8, InetAddress.getLoopbackAddress()) }.getOrElse { return fail("can't listen: $it") }
        serverSocket = server
        println("ready ${server.localPort}")
        System.out.flush()
        if (attach) {
            runCatching {
                Runtime.getRuntime().exec(
                    arrayOf(
                        "am",
                        "broadcast",
                        "--include-stopped-packages",
                        "-n",
                        "$PKG/${Elevated.AttachReceiver::class.java.name}",
                        "--ei",
                        "port",
                        server.localPort.toString(),
                        "--es",
                        "token",
                        token,
                    ),
                ).waitFor()
            }
        }
        while (!server.isClosed) {
            val client = runCatching { server.accept() }.getOrNull() ?: continue
            Thread { serve(client, token, injector) }.start()
        }
    }

    private fun grantAppOps() {
        val commands = buildList {
            add(arrayOf("pm", "grant", PKG, "android.permission.WRITE_SECURE_SETTINGS"))
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) {
                add(arrayOf("appops", "set", PKG, "MANAGE_ONGOING_CALLS", "allow"))
            }
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                add(arrayOf("appops", "set", PKG, "RECEIVE_SENSITIVE_NOTIFICATIONS", "allow"))
            }
            add(arrayOf("cmd", "notification", "allow_dnd", PKG))
            add(arrayOf("appops", "set", PKG, "WRITE_SETTINGS", "allow"))
            add(arrayOf("appops", "set", PKG, "READ_CLIPBOARD_IN_BACKGROUND", "allow"))
            add(arrayOf("appops", "set", PKG, "PROJECT_MEDIA", "allow"))
        }
        for (cmd in commands) {
            runCatching { Runtime.getRuntime().exec(cmd).waitFor() }
        }
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            runCatching {
                val proc = Runtime.getRuntime().exec(arrayOf("settings", "get", "secure", "enabled_notification_listeners"))
                val enabled = proc.inputStream.bufferedReader().readText()
                proc.waitFor()
                if (enabled.contains("$PKG/")) {
                    Runtime.getRuntime().exec(
                        arrayOf("cmd", "notification", "allow_listener", "$PKG/app.nectarlink.android.notifications.NotificationListener"),
                    ).waitFor()
                }
            }
        }
    }

    @Volatile private var clipListenerProxy: Any? = null

    /** The clipboard service and its setPrimaryClip, once the watcher found them. */
    @Volatile private var clipService: Pair<Any, Method>? = null

    /**
     * Puts a PC's text on the clipboard as the shell, marked to suppress the
     * clipboard overlay: SystemUI (Samsung's copy toast too) honours that
     * only for clips the shell sets, so the copy doesn't pop up on the phone.
     */
    private fun setClipboard(rest: String) {
        val (service, set) = clipService ?: return System.err.println("no clipboard to set")
        runCatching {
            val parts = rest.split(' ')
            val decode = { b64: String -> String(android.util.Base64.decode(b64, android.util.Base64.NO_WRAP), Charsets.UTF_8) }
            val text = decode(parts[0])
            val html = parts.getOrNull(1)?.takeIf(String::isNotEmpty)?.let(decode)
            val clip = if (html != null) {
                android.content.ClipData.newHtmlText("Nectarlink", text, html)
            } else {
                android.content.ClipData.newPlainText("Nectarlink", text)
            }
            clip.description.extras = android.os.PersistableBundle().apply {
                putBoolean("com.android.systemui.SUPPRESS_CLIPBOARD_OVERLAY", true)
                putBoolean("android.content.extra.IS_REMOTE_DEVICE", true)
            }
            set.invoke(service, *buildClipboardArgs(set.parameterTypes, null, clip))
        }.onFailure { System.err.println("can't set the clipboard: $it") }
    }

    @android.annotation.SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    private fun startClipboardWatcher() {
        runCatching {
            val sm = Class.forName("android.os.ServiceManager")
            val binder = sm.getMethod("getService", String::class.java).invoke(null, "clipboard") as? android.os.IBinder ?: return
            val stub = Class.forName("android.content.IClipboard\$Stub")
            val clipboard = stub.getMethod("asInterface", android.os.IBinder::class.java).invoke(null, binder) ?: return
            val getMethod = clipboard.javaClass.methods.firstOrNull { it.name == "getPrimaryClip" } ?: return
            clipboard.javaClass.methods.firstOrNull { it.name == "setPrimaryClip" }?.let { clipService = clipboard to it }
            val addMethod = clipboard.javaClass.methods.firstOrNull { it.name == "addPrimaryClipChangedListener" } ?: return
            val listenerClass = Class.forName("android.content.IOnPrimaryClipChangedListener")
            val listenerBinder = object : android.os.Binder() {
                override fun onTransact(code: Int, data: android.os.Parcel, reply: android.os.Parcel?, flags: Int): Boolean {
                    if (code == FIRST_CALL_TRANSACTION) {
                        runCatching { data.enforceInterface("android.content.IOnPrimaryClipChangedListener") }
                        readAndSendPrimaryClip(clipboard, getMethod)
                        return true
                    }
                    return super.onTransact(code, data, reply, flags)
                }
            }
            var proxyRef: Any? = null
            val proxy = java.lang.reflect.Proxy.newProxyInstance(
                listenerClass.classLoader,
                arrayOf(listenerClass),
            ) { _, method, args ->
                when (method.name) {
                    "asBinder" -> listenerBinder
                    "dispatchPrimaryClipChanged" -> {
                        readAndSendPrimaryClip(clipboard, getMethod)
                        null
                    }
                    "hashCode" -> System.identityHashCode(listenerBinder)
                    "equals" -> args?.getOrNull(0) === proxyRef
                    "toString" -> "NectarlinkClipListener"
                    else -> null
                }
            }
            proxyRef = proxy
            clipListenerProxy = proxy
            addMethod.invoke(clipboard, *buildClipboardArgs(addMethod.parameterTypes, proxy, null))
        }.onFailure {
            System.err.println("clipboard watcher unavailable: $it")
        }
    }

    private fun buildClipboardArgs(
        paramTypes: Array<Class<*>>,
        listener: Any?,
        clipData: android.content.ClipData?,
    ): Array<Any?> {
        var seenPkg = false
        return Array(paramTypes.size) { i ->
            val t = paramTypes[i]
            when {
                t.name == "android.content.IOnPrimaryClipChangedListener" -> listener
                t == android.content.ClipData::class.java -> clipData
                t == String::class.java -> if (!seenPkg) {
                    seenPkg = true
                    "com.android.shell"
                } else {
                    null
                }
                t == Int::class.javaPrimitiveType || t == Int::class.javaObjectType -> 0
                t == Boolean::class.javaPrimitiveType || t == Boolean::class.javaObjectType -> false
                else -> null
            }
        }
    }

    @Volatile private var lastSentClipKey: String? = null
    private val clipExecutor = java.util.concurrent.Executors.newSingleThreadExecutor { r ->
        Thread(r, "nectarlink-clip-watcher").apply { isDaemon = true }
    }

    private fun readAndSendPrimaryClip(clipboard: Any, getMethod: Method) {
        val out = controlOut ?: return
        clipExecutor.execute {
            runCatching {
                val clip = getMethod.invoke(clipboard, *buildClipboardArgs(getMethod.parameterTypes, null, null))
                    as? android.content.ClipData ?: return@runCatching
                val desc = clip.description
                val label = desc?.label?.toString()
                val extras = desc?.extras
                val isHostPipe = label == "host clipboard" ||
                    extras?.getBoolean("com.android.systemui.SUPPRESS_CLIPBOARD_OVERLAY") == true ||
                    extras?.getBoolean("android.content.extra.IS_REMOTE_DEVICE") == true
                val isSensitive = extras?.getBoolean("android.content.extra.IS_SENSITIVE") == true
                if (isHostPipe || isSensitive || label == "Nectarlink") {
                    lastSentClipKey = null
                    return@runCatching
                }
                for (i in 0 until clip.itemCount) {
                    val item = clip.getItemAt(i)
                    val cs = item.text ?: continue
                    val text = cs.toString().takeIf(String::isNotBlank) ?: continue
                    val normalized = text.replace("\r\n", "\n").replace('\r', '\n')
                    val html = item.htmlText?.takeIf(String::isNotBlank) ?: run {
                        if (cs is android.text.Spanned &&
                            cs.getSpans(0, cs.length, android.text.style.CharacterStyle::class.java).isNotEmpty()
                        ) {
                            android.text.Html.toHtml(cs, android.text.Html.TO_HTML_PARAGRAPH_LINES_CONSECUTIVE)
                                .takeIf(String::isNotBlank)
                        } else {
                            null
                        }
                    }
                    val dedupKey = "$normalized\u0000${html.orEmpty()}"
                    if (dedupKey == lastSentClipKey) return@runCatching
                    lastSentClipKey = dedupKey
                    val b64Text = android.util.Base64.encodeToString(text.toByteArray(Charsets.UTF_8), android.util.Base64.NO_WRAP)
                    val b64Html = if (html != null) {
                        android.util.Base64.encodeToString(html.toByteArray(Charsets.UTF_8), android.util.Base64.NO_WRAP)
                    } else {
                        ""
                    }
                    synchronized(out) {
                        out.write("CLIP\t$b64Text\t$b64Html\n")
                        out.flush()
                    }
                    return@runCatching
                }
            }
        }
    }

    private fun fail(why: String) {
        System.err.println(why)
    }

    private fun shutdown() {
        runCatching { serverSocket?.close() }
        runCatching { UhidBridge.closeAll() }
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
            if (first == "A") return streamDeviceAudio(client.getOutputStream())
            val writer = java.io.OutputStreamWriter(client.getOutputStream(), Charsets.UTF_8)
            controlOut = writer
            try {
                var line: String? = if (first == "H") runCatching { reader.nextLine() }.getOrNull() else first
                while (line != null) {
                    val input = line
                    if (input != "H") {
                        runCatching { injector.handle(input) }.onFailure { System.err.println("input failed: $it") }
                    }
                    line = runCatching { reader.nextLine() }.getOrNull()
                }
            } finally {
                if (controlOut === writer) controlOut = null
                if (displayTurnedOff) runCatching { setDisplayPower(true) }
                shutdown()
            }
        }
    }

    /**
     * Captures internal device audio at 48 kHz 16-bit stereo PCM in the
     * `shell` process (`CAPTURE_AUDIO_OUTPUT` / `REMOTE_SUBMIX`) without a
     * MediaProjection prompt and streams 10 ms (1920-byte) frames until the
     * socket closes.
     */
    @android.annotation.SuppressLint("MissingPermission", "PrivateApi", "DiscouragedPrivateApi")
    private fun streamDeviceAudio(out: java.io.OutputStream) {
        val rate = 48_000
        val channelMask = android.media.AudioFormat.CHANNEL_IN_STEREO
        val encoding = android.media.AudioFormat.ENCODING_PCM_16BIT
        val frameBytes = (rate / 100) * 2 * 2 // 1920 bytes = 10 ms
        val minBuf = maxOf(
            android.media.AudioRecord.getMinBufferSize(rate, channelMask, encoding),
            frameBytes * 4,
        )
        val format = android.media.AudioFormat.Builder()
            .setSampleRate(rate)
            .setChannelMask(channelMask)
            .setEncoding(encoding)
            .build()

        fun openRecorder(): android.media.AudioRecord? {
            // 1. Try AudioAttributes.Builder.setInternalCapturePreset(REMOTE_SUBMIX = 8) via reflection.
            runCatching<Unit> {
                val attrBuilder = android.media.AudioAttributes.Builder()
                val setPreset = attrBuilder.javaClass.getMethod(
                    "setInternalCapturePreset",
                    Int::class.javaPrimitiveType,
                )
                setPreset.invoke(attrBuilder, android.media.MediaRecorder.AudioSource.REMOTE_SUBMIX)
                val recBuilder = android.media.AudioRecord.Builder()
                    .setAudioFormat(format)
                    .setBufferSizeInBytes(minBuf)
                val setAttrs = recBuilder.javaClass.getMethod(
                    "setAudioAttributes",
                    android.media.AudioAttributes::class.java,
                )
                setAttrs.invoke(recBuilder, attrBuilder.build())
                val rec = recBuilder.build()
                if (rec.state == android.media.AudioRecord.STATE_INITIALIZED) return rec
                rec.release()
            }
            // 2. Try standard AudioRecord with REMOTE_SUBMIX (8).
            runCatching {
                val rec = android.media.AudioRecord(
                    android.media.MediaRecorder.AudioSource.REMOTE_SUBMIX,
                    rate,
                    channelMask,
                    encoding,
                    minBuf,
                )
                if (rec.state == android.media.AudioRecord.STATE_INITIALIZED) return rec
                rec.release()
            }
            // Nothing else: any other source is the microphone, and the PC
            // asked for what the phone plays, not what it hears.
            return null
        }

        val recorder = openRecorder() ?: return
        try {
            recorder.startRecording()
            val buf = ByteArray(frameBytes)
            while (true) {
                val n = recorder.read(buf, 0, buf.size)
                if (n <= 0) break
                out.write(buf, 0, n)
                out.flush()
            }
        } catch (_: java.io.IOException) {
            // Client disconnected.
        } finally {
            runCatching { recorder.stop() }
            runCatching { recorder.release() }
        }
    }

    /** Turns the physical screen panel on or off while keeping the display pipeline alive for capture. */
    @android.annotation.SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
    private fun setDisplayPower(on: Boolean) {
        if (!on) {
            displayTurnedOff = true
            if (savedBrightness == null) {
                savedBrightness = runCatching {
                    val proc = Runtime.getRuntime().exec(arrayOf("settings", "get", "system", "screen_brightness"))
                    val out = proc.inputStream.bufferedReader().use { it.readText().trim().toIntOrNull() }
                    proc.waitFor()
                    out
                }.getOrNull()
            }
        } else {
            displayTurnedOff = false
        }
        runCatching {
            val sc = Class.forName("android.view.SurfaceControl")
            val token = runCatching {
                sc.getMethod("getInternalDisplayToken").invoke(null) as? android.os.IBinder
            }.getOrNull() ?: runCatching {
                val dc = runCatching {
                    dalvik.system.PathClassLoader("/system/framework/services.jar", null)
                        .loadClass("com.android.server.display.DisplayControl")
                }.getOrElse { Class.forName("com.android.server.display.DisplayControl") }
                val ids = dc.getMethod("getPhysicalDisplayIds").invoke(null) as? LongArray
                if (ids != null && ids.isNotEmpty()) {
                    dc.getMethod("getPhysicalDisplayToken", Long::class.javaPrimitiveType)
                        .invoke(null, ids[0]) as? android.os.IBinder
                } else {
                    null
                }
            }.getOrNull() ?: return@runCatching
            sc.getMethod("setDisplayPowerMode", android.os.IBinder::class.java, Int::class.javaPrimitiveType)
                .invoke(null, token, if (on) POWER_MODE_NORMAL else POWER_MODE_OFF)
        }
        if (!on) {
            runCatching {
                Runtime.getRuntime().exec(arrayOf("settings", "put", "system", "screen_brightness", "0")).waitFor()
            }
        } else if (on) {
            val saved = savedBrightness
            if (saved != null) {
                savedBrightness = null
                runCatching {
                    Runtime.getRuntime().exec(arrayOf("settings", "put", "system", "screen_brightness", saved.toString())).waitFor()
                }
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

    /** An app window (or full-screen mirror when `pkg == "-"`) on this connection, until it closes. */
    private fun appWindow(client: Socket, request: String, reader: java.io.BufferedReader) {
        val parts = request.split(' ')
        if (parts.size < 7) return
        val numbers = parts.subList(1, 6).map { it.toIntOrNull() ?: return }
        val (width, height, dpi, bitrate, fps) = numbers
        val pkg = parts.getOrNull(6)?.takeIf { it == "-" || PACKAGE.matches(it) } ?: return
        val codec = parts.getOrNull(7)?.takeIf { it in setOf("h264", "h265", "av1") } ?: "h264"
        if (width !in 16..4096 || height !in 16..4096 || dpi !in 72..1000) return
        val window = AppDisplay(width, height, dpi, bitrate.coerceIn(500_000, 40_000_000), fps.coerceIn(1, 120), codec)
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
            runCatching { window.run(pkg, client.getOutputStream()) }.onFailure { e ->
                // Reflection wraps the real failure; report the innermost one, with where it happened.
                var cause: Throwable = e
                while (cause.cause != null && cause.cause !== cause) cause = cause.cause!!
                System.err.println("app window ended: $cause")
                cause.stackTrace.take(8).forEach { System.err.println("  at $it") }
            }
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
        private var lastGamepadButtons = 0
        /**The display the line being handled is for (-1: the screen's own). */
        private var display = -1
        private val setDisplayId: Method? = runCatching {
            InputEvent::class.java.getMethod("setDisplayId", Int::class.javaPrimitiveType)
        }.getOrNull()

        init {
            // Android 14 moved injection to InputManagerGlobal.
            val global = runCatching { Class.forName("android.hardware.input.InputManagerGlobal") }.getOrNull()
            val owner = global ?: Class.forName("android.hardware.input.InputManager")
            manager = checkNotNull(owner.getDeclaredMethod("getInstance").invoke(null)) { "InputManager instance is null" }
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
                "G" -> gamepad(rest)
                "C" -> if (allowedCommand(rest) && display < 0) {
                    when (rest) {
                        "hotspot on" -> setHotspot(true)
                        "hotspot off" -> setHotspot(false)
                        // Takes up to a minute: off this thread, which also carries input.
                        COMPILE_APP_COMMAND -> Thread { runCatching { Runtime.getRuntime().exec(rest.split(' ').toTypedArray()).waitFor() } }.start()
                        else -> Runtime.getRuntime().exec(rest.split(' ').toTypedArray()).waitFor()
                    }
                }
                "P" -> if (display < 0) setDisplayPower(rest.trim() != "0")
                "B" -> if (display < 0) setClipboard(rest)
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
            val hidSent = if (display < 0) UhidBridge.sendKeyTap(code) else false
            if (!hidSent) {
                val now = SystemClock.uptimeMillis()
                for (action in intArrayOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
                    send(KeyEvent(now, now, action, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD))
                }
            }
        }

        private fun type(text: String) {
            val map = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD)
            // What the keyboard map can't type (most non-Latin letters, emoji) is skipped.
            for (c in text) {
                map.getEvents(charArrayOf(c))?.forEach { send(it) }
            }
        }

        /**
         * Forwards a PC gamepad state (`G <buttons> <lx> <ly> <rx> <ry> <lt> <rt>`)
         * through `/dev/uhid` when writable and injects Android gamepad/joystick
         * events (`SOURCE_GAMEPAD` + `SOURCE_JOYSTICK`).
         */
        private fun gamepad(rest: String) {
            val tokens = rest.trim().split(' ')
            if (tokens.size < 7) return
            val buttons = tokens[0].toIntOrNull() ?: return
            val lx = tokens[1].toIntOrNull() ?: return
            val ly = tokens[2].toIntOrNull() ?: return
            val rx = tokens[3].toIntOrNull() ?: return
            val ry = tokens[4].toIntOrNull() ?: return
            val lt = tokens[5].toIntOrNull() ?: return
            val rt = tokens[6].toIntOrNull() ?: return

            if (display < 0) {
                UhidBridge.sendGamepadReport(buttons, lx, ly, rx, ry, lt, rt)
            }

            val now = SystemClock.uptimeMillis()
            val changed = buttons xor lastGamepadButtons
            lastGamepadButtons = buttons

            for ((bit, keyCode) in GAMEPAD_BUTTON_KEYCODES) {
                val mask = 1 shl bit
                if ((changed and mask) != 0) {
                    val action = if ((buttons and mask) != 0) KeyEvent.ACTION_DOWN else KeyEvent.ACTION_UP
                    send(
                        KeyEvent(
                            now,
                            now,
                            action,
                            keyCode,
                            0,
                            0,
                            KeyCharacterMap.VIRTUAL_KEYBOARD,
                            0,
                            0,
                            InputDevice.SOURCE_GAMEPAD or InputDevice.SOURCE_DPAD,
                        ),
                    )
                }
            }

            fun normAxis(v: Int): Float = (v.coerceIn(-32767, 32767) / 32767f)
            fun normTrigger(v: Int): Float = (v.coerceIn(0, 255) / 255f)
            val hatX = when {
                (buttons and (1 shl 13)) != 0 -> 1f
                (buttons and (1 shl 12)) != 0 -> -1f
                else -> 0f
            }
            val hatY = when {
                (buttons and (1 shl 11)) != 0 -> 1f
                (buttons and (1 shl 10)) != 0 -> -1f
                else -> 0f
            }

            val properties = arrayOf(
                MotionEvent.PointerProperties().apply {
                    id = 0
                    toolType = MotionEvent.TOOL_TYPE_UNKNOWN
                },
            )
            val coords = arrayOf(
                MotionEvent.PointerCoords().apply {
                    setAxisValue(MotionEvent.AXIS_X, normAxis(lx))
                    setAxisValue(MotionEvent.AXIS_Y, normAxis(ly))
                    setAxisValue(MotionEvent.AXIS_Z, normAxis(rx))
                    setAxisValue(MotionEvent.AXIS_RZ, normAxis(ry))
                    setAxisValue(MotionEvent.AXIS_LTRIGGER, normTrigger(lt))
                    setAxisValue(MotionEvent.AXIS_RTRIGGER, normTrigger(rt))
                    setAxisValue(MotionEvent.AXIS_HAT_X, hatX)
                    setAxisValue(MotionEvent.AXIS_HAT_Y, hatY)
                },
            )
            val motion = MotionEvent.obtain(
                now,
                now,
                MotionEvent.ACTION_MOVE,
                1,
                properties,
                coords,
                0,
                0,
                1f,
                1f,
                0,
                0,
                InputDevice.SOURCE_JOYSTICK or InputDevice.SOURCE_GAMEPAD,
                0,
            )
            send(motion)
            motion.recycle()
        }

        private fun setHotspot(enable: Boolean) {
            val candidates = if (enable) {
                listOf(
                    arrayOf("cmd", "connectivity", "start-tethering", "wifi"),
                    arrayOf("cmd", "wifi", "start-softap"),
                    arrayOf("svc", "wifi", "tether", "enable"),
                )
            } else {
                listOf(
                    arrayOf("cmd", "connectivity", "stop-tethering", "wifi"),
                    arrayOf("cmd", "wifi", "stop-softap"),
                    arrayOf("svc", "wifi", "tether", "disable"),
                )
            }
            for (cmd in candidates) {
                val ok = runCatching {
                    val proc = Runtime.getRuntime().exec(cmd)
                    proc.waitFor() == 0
                }.getOrDefault(false)
                if (ok) break
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
            "hotspot on",
            "hotspot off",
            "cmd wifi start-softap",
            "cmd wifi stop-softap",
            // Pause for banking apps: seeing which app is in front.
            "appops set app.nectarlink GET_USAGE_STATS allow",
            // Fast starts right after an install or update.
            COMPILE_APP_COMMAND,
        )

        private companion object {
            const val BRIGHTNESS_PREFIX = "settings put system screen_brightness "

            val GAMEPAD_BUTTON_KEYCODES = listOf(
                0 to KeyEvent.KEYCODE_BUTTON_A,
                1 to KeyEvent.KEYCODE_BUTTON_B,
                2 to KeyEvent.KEYCODE_BUTTON_X,
                3 to KeyEvent.KEYCODE_BUTTON_Y,
                4 to KeyEvent.KEYCODE_BUTTON_L1,
                5 to KeyEvent.KEYCODE_BUTTON_R1,
                6 to KeyEvent.KEYCODE_BUTTON_SELECT,
                7 to KeyEvent.KEYCODE_BUTTON_START,
                8 to KeyEvent.KEYCODE_BUTTON_THUMBL,
                9 to KeyEvent.KEYCODE_BUTTON_THUMBR,
                10 to KeyEvent.KEYCODE_DPAD_UP,
                11 to KeyEvent.KEYCODE_DPAD_DOWN,
                12 to KeyEvent.KEYCODE_DPAD_LEFT,
                13 to KeyEvent.KEYCODE_DPAD_RIGHT,
                14 to KeyEvent.KEYCODE_BUTTON_MODE,
            )
        }
    }

    /**
     * Kernel `/dev/uhid` bridge for registering virtual USB HID keyboard and
     * gamepad peripherals in the `shell` user context (which belongs to GID
     * `uhid` / `input` on stock Android). Falls back gracefully to
     * `InputManager.injectInputEvent` if `/dev/uhid` is restricted by SELinux.
     */
    private object UhidBridge {
        private const val UHID_DESTROY = 1
        private const val UHID_CREATE2 = 11
        private const val UHID_INPUT2 = 12
        private const val UHID_EVENT_SIZE = 4380
        private const val BUS_USB = 0x03

        // While a hardware keyboard is attached Android hides the on-screen
        // one, so the virtual keyboard only stays while the PC is typing.
        private const val KEYBOARD_IDLE_MS = 10_000L
        private val idle = java.util.concurrent.Executors.newSingleThreadScheduledExecutor { r ->
            Thread(r, "nectarlink-uhid-idle").apply { isDaemon = true }
        }
        private var idleClose: java.util.concurrent.ScheduledFuture<*>? = null

        // Standard 8-byte boot HID keyboard report descriptor.
        private val KEYBOARD_RD = byteArrayOf(
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x06, // Usage (Keyboard)
            0xA1.toByte(), 0x01, // Collection (Application)
            0x05, 0x07, //   Usage Page (Key Codes)
            0x19, 0xE0.toByte(), //   Usage Minimum (224)
            0x29, 0xE7.toByte(), //   Usage Maximum (231)
            0x15, 0x00, //   Logical Minimum (0)
            0x25, 0x01, //   Logical Maximum (1)
            0x75, 0x01, //   Report Size (1)
            0x95.toByte(), 0x08, //   Report Count (8)
            0x81.toByte(), 0x02, //   Input (Data, Variable, Absolute)
            0x95.toByte(), 0x01, //   Report Count (1)
            0x75, 0x08, //   Report Size (8)
            0x81.toByte(), 0x01, //   Input (Constant)
            0x95.toByte(), 0x06, //   Report Count (6)
            0x75, 0x08, //   Report Size (8)
            0x15, 0x00, //   Logical Minimum (0)
            0x25, 0x65, //   Logical Maximum (101)
            0x05, 0x07, //   Usage Page (Key Codes)
            0x19, 0x00, //   Usage Minimum (0)
            0x29, 0x65, //   Usage Maximum (101)
            0x81.toByte(), 0x00, //   Input (Data, Array)
            0xC0.toByte(), // End Collection
        )

        // 12-byte HID gamepad report descriptor:
        // [buttonsLo, buttonsHi, lxLo, lxHi, lyLo, lyHi, rxLo, rxHi, ryLo, ryHi, lt, rt]
        private val GAMEPAD_RD = byteArrayOf(
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x05, // Usage (Game Pad)
            0xA1.toByte(), 0x01, // Collection (Application)
            0x05, 0x09, //   Usage Page (Button)
            0x19, 0x01, //   Usage Minimum (Button 1)
            0x29, 0x10, //   Usage Maximum (Button 16)
            0x15, 0x00, //   Logical Minimum (0)
            0x25, 0x01, //   Logical Maximum (1)
            0x75, 0x01, //   Report Size (1)
            0x95.toByte(), 0x10, //   Report Count (16)
            0x81.toByte(), 0x02, //   Input (Data, Variable, Absolute)
            0x05, 0x01, //   Usage Page (Generic Desktop)
            0x09, 0x30, //   Usage (X)
            0x09, 0x31, //   Usage (Y)
            0x09, 0x32, //   Usage (Z)
            0x09, 0x35, //   Usage (Rz)
            0x16, 0x01, 0x80.toByte(), // Logical Minimum (-32767)
            0x26, 0xFF.toByte(), 0x7F, // Logical Maximum (32767)
            0x75, 0x10, //   Report Size (16)
            0x95.toByte(), 0x04, //   Report Count (4)
            0x81.toByte(), 0x02, //   Input (Data, Variable, Absolute)
            0x05, 0x02, //   Usage Page (Simulation Controls)
            0x09, 0xC5.toByte(), // Usage (Brake / LT)
            0x09, 0xC4.toByte(), // Usage (Accelerator / RT)
            0x15, 0x00, //   Logical Minimum (0)
            0x26, 0xFF.toByte(), 0x00, // Logical Maximum (255)
            0x75, 0x08, //   Report Size (8)
            0x95.toByte(), 0x02, //   Report Count (2)
            0x81.toByte(), 0x02, //   Input (Data, Variable, Absolute)
            0xC0.toByte(), // End Collection
        )

        @Volatile private var keyboardFd: java.io.RandomAccessFile? = null
        @Volatile private var keyboardAttempted = false

        @Volatile private var gamepadFd: java.io.RandomAccessFile? = null
        @Volatile private var gamepadAttempted = false

        private fun openDevice(name: String, product: Int, rd: ByteArray): java.io.RandomAccessFile? = runCatching {
            val raf = java.io.RandomAccessFile("/dev/uhid", "rw")
            val buf = java.nio.ByteBuffer.allocate(UHID_EVENT_SIZE).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            buf.putInt(UHID_CREATE2)
            val nameBytes = name.toByteArray(Charsets.UTF_8)
            buf.position(4)
            buf.put(nameBytes, 0, minOf(nameBytes.size, 127))
            val physBytes = "nectarlink/uhid".toByteArray(Charsets.UTF_8)
            buf.position(4 + 128)
            buf.put(physBytes, 0, minOf(physBytes.size, 63))
            val uniqBytes = "nectarlink-$product".toByteArray(Charsets.UTF_8)
            buf.position(4 + 128 + 64)
            buf.put(uniqBytes, 0, minOf(uniqBytes.size, 63))
            buf.position(4 + 128 + 64 + 64)
            buf.putShort(rd.size.toShort())
            buf.putShort(BUS_USB.toShort())
            buf.putInt(0x18D1) // Google vendor ID
            buf.putInt(product)
            buf.putInt(1)
            buf.putInt(0)
            buf.put(rd)
            raf.write(buf.array())
            raf
        }.getOrNull()

        private fun writeInputReport(raf: java.io.RandomAccessFile, report: ByteArray): Boolean = runCatching {
            val buf = java.nio.ByteBuffer.allocate(4 + 2 + report.size).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            buf.putInt(UHID_INPUT2)
            buf.putShort(report.size.toShort())
            buf.put(report)
            raf.write(buf.array())
            true
        }.getOrDefault(false)

        @Synchronized
        fun sendKeyTap(androidKeyCode: Int): Boolean {
            val usage = androidKeyCodeToHidUsage(androidKeyCode) ?: return false
            val created = !keyboardAttempted
            if (created) {
                keyboardAttempted = true
                keyboardFd = openDevice("NectarLink Virtual Keyboard", 0x4EE7, KEYBOARD_RD)
            }
            val fd = keyboardFd ?: return false
            idleClose?.cancel(false)
            idleClose = idle.schedule({ closeKeyboard() }, KEYBOARD_IDLE_MS, java.util.concurrent.TimeUnit.MILLISECONDS)
            // A device that was just created may not be registered in time for
            // its first report: that key goes the injected way.
            if (created) return false
            val down = ByteArray(8).apply { this[2] = usage.toByte() }
            val up = ByteArray(8)
            return writeInputReport(fd, down) && writeInputReport(fd, up)
        }

        /** Removes the virtual keyboard; the next key from the PC adds it again. */
        @Synchronized
        private fun closeKeyboard() {
            keyboardFd?.let(::destroy)
            keyboardFd = null
            keyboardAttempted = false
            idleClose = null
        }

        private fun destroy(fd: java.io.RandomAccessFile) {
            runCatching {
                val buf = java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                buf.putInt(UHID_DESTROY)
                fd.write(buf.array())
                fd.close()
            }
        }

        @Synchronized
        fun sendGamepadReport(buttons: Int, lx: Int, ly: Int, rx: Int, ry: Int, lt: Int, rt: Int): Boolean {
            if (!gamepadAttempted) {
                gamepadAttempted = true
                gamepadFd = openDevice("NectarLink Virtual Gamepad", 0x4EE8, GAMEPAD_RD)
            }
            val fd = gamepadFd ?: return false
            val report = java.nio.ByteBuffer.allocate(12).order(java.nio.ByteOrder.LITTLE_ENDIAN).apply {
                putShort(buttons.toShort())
                putShort(lx.coerceIn(-32767, 32767).toShort())
                putShort(ly.coerceIn(-32767, 32767).toShort())
                putShort(rx.coerceIn(-32767, 32767).toShort())
                putShort(ry.coerceIn(-32767, 32767).toShort())
                put(lt.coerceIn(0, 255).toByte())
                put(rt.coerceIn(0, 255).toByte())
            }.array()
            return writeInputReport(fd, report)
        }

        @Synchronized
        fun closeAll() {
            idleClose?.cancel(false)
            idleClose = null
            listOfNotNull(keyboardFd, gamepadFd).forEach(::destroy)
            keyboardFd = null
            gamepadFd = null
        }

        private fun androidKeyCodeToHidUsage(keyCode: Int): Int? = when (keyCode) {
            in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> 0x04 + (keyCode - KeyEvent.KEYCODE_A)
            in KeyEvent.KEYCODE_1..KeyEvent.KEYCODE_9 -> 0x1E + (keyCode - KeyEvent.KEYCODE_1)
            KeyEvent.KEYCODE_0 -> 0x27
            KeyEvent.KEYCODE_ENTER -> 0x28
            KeyEvent.KEYCODE_ESCAPE -> 0x29
            KeyEvent.KEYCODE_DEL -> 0x2A
            KeyEvent.KEYCODE_TAB -> 0x2B
            KeyEvent.KEYCODE_SPACE -> 0x2C
            KeyEvent.KEYCODE_DPAD_RIGHT -> 0x4F
            KeyEvent.KEYCODE_DPAD_LEFT -> 0x50
            KeyEvent.KEYCODE_DPAD_DOWN -> 0x51
            KeyEvent.KEYCODE_DPAD_UP -> 0x52
            else -> null
        }
    }
}
