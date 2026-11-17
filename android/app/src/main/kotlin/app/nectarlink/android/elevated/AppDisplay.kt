// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.annotation.SuppressLint
import android.content.Context
import android.content.ContextWrapper
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Looper
import android.view.Surface
import java.io.DataOutputStream
import java.io.OutputStream

/**
 * An app in a window of its own (Elevated): a virtual display of its own,
 * made by the shell-side helper (apps may not put other apps on their
 * displays). A surface encoder captures that display, and the app starts on it.
 * Packets written to [out] use `u32 length | u8 kind | u64 time_us | data`.
 * Kind 0 carries four u32 fields: width, height, display ID, and codec ID
 * (1: H.264, 2: H.265, 3: AV1). Kinds 1 and 2 carry frames and keyframes.
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
class AppDisplay(
    private val width: Int,
    private val height: Int,
    private val dpi: Int,
    private val bitrate: Int,
    private val fps: Int,
    private val codecName: String = "h264",
    /** Wakes the phone; a window on a plain display needs it awake (#152). */
    private val wakePhone: () -> Unit = {},
) {
    private data class EncodedSurface(
        val codecName: String,
        val codec: MediaCodec,
        val surface: Surface,
    )

    @Volatile private var running = true
    @Volatile private var nextSize: Triple<Int, Int, Int>? = null
    /** The display's size and density now, to turn it with its app. */
    @Volatile private var size = Triple(width, height, dpi)
    private var lastRotationCheck = 0L
    private var display: VirtualDisplay? = null
    /** Owns [display] when it has one; closed after it. */
    private var device: VirtualDevice? = null

    /** The display's ID, once made. */
    var displayId: Int = -1
        private set

    /** Streams until [stop] or until [out] fails; starts [pkg] on the display first (or mirrors the main screen when `pkg == "-"`). */
    fun run(pkg: String, out: OutputStream) {
        val fullScreen = pkg == "-"
        var curW = width
        var curH = height
        var curDpi = dpi
        var codec: MediaCodec? = null
        var surface: Surface? = null
        val data = DataOutputStream(out.buffered(1 shl 16))
        try {
            val first = createEncoder(curW, curH)
            codec = first.codec
            surface = first.surface
            val made = if (fullScreen) {
                createMirrorDisplay(curW, curH, curDpi, first.surface)
            } else {
                createDisplay(curW, curH, curDpi, first.surface)
            }
            display = made
            displayId = if (fullScreen) 0 else made.display.displayId
            if (!fullScreen) {
                hideIme(displayId)
            }
            codecForKeyframes = first.codec
            writeConfig(data, curW, curH, displayId, first.codecName)
            if (!fullScreen) {
                launch(pkg, displayId)
                watchActivities(displayId)
                keepActive(displayId)
            }
            while (running) {
                val activeCodec = codec ?: break
                drain(activeCodec, data)
                if (!running) break
                val (newW, newH, newDpi) = nextSize ?: continue
                nextSize = null
                if (newW == curW && newH == curH && newDpi == curDpi) continue
                if (newW == curW && newH == curH) {
                    // Only the density (the PC's zoom): the picture's size
                    // stays, so the encoder does too, and nothing restarts.
                    made.resize(curW, curH, newDpi)
                    forceDensity(displayId, newDpi)
                    curDpi = newDpi
                    size = Triple(curW, curH, curDpi)
                    continue
                }
                codecForKeyframes = null
                runCatching { activeCodec.stop() }
                runCatching { activeCodec.release() }
                codec = null
                runCatching { surface?.release() }
                surface = null
                val next = createEncoder(newW, newH)
                codec = next.codec
                surface = next.surface
                codecForKeyframes = next.codec
                made.resize(newW, newH, newDpi)
                made.surface = next.surface
                curW = newW
                curH = newH
                curDpi = newDpi
                size = Triple(curW, curH, curDpi)
                writeConfig(data, curW, curH, displayId, next.codecName)
            }
        } finally {
            if (!fullScreen && displayId > 0) forceDensity(displayId, null)
            running = false
            codecForKeyframes = null
            runCatching { codec?.stop() }
            runCatching { codec?.release() }
            runCatching { surface?.release() }
            display?.release()
            display = null
            device?.close()
            device = null
        }
    }

    fun stop() {
        running = false
    }

    /**
     * An app that only runs one way up (a landscape-only game in a portrait
     * window, #126) turns the display (it rotates with its content). The
     * picture then came out on its side and taps landed elsewhere, so the
     * display takes the app's shape instead: the same size the other way
     * round, which the PC's window follows.
     */
    private fun followAppOrientation() {
        // Not the phone's own screen (mirroring it uses this loop too).
        if (displayId <= 0) return
        val now = android.os.SystemClock.uptimeMillis()
        if (now - lastRotationCheck < ROTATION_CHECK_MS) return
        lastRotationCheck = now
        val rotation = runCatching { display?.display?.rotation }.getOrNull() ?: return
        if (rotation == Surface.ROTATION_90 || rotation == Surface.ROTATION_270) {
            val (w, h, d) = size
            nextSize = Triple(h, w, d)
        }
    }

    fun resize(w: Int, h: Int, newDpi: Int) {
        nextSize = Triple(w, h, newDpi)
    }

    @Volatile private var codecForKeyframes: MediaCodec? = null

    /** The PC's decoder lost its place: a keyframe next. */
    fun requestKeyframe() {
        val codec = codecForKeyframes ?: return
        runCatching {
            codec.setParameters(android.os.Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
        }
    }

    private fun mimeForCodec(name: String): String = when (name.lowercase()) {
        "h265", "hevc" -> MediaFormat.MIMETYPE_VIDEO_HEVC
        "av1" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            "video/av01"
        } else {
            throw UnsupportedOperationException("AV1 requires Android 10+")
        }
        else -> MediaFormat.MIMETYPE_VIDEO_AVC
    }

    private fun createEncoderForCodec(name: String, w: Int, h: Int): EncodedSurface {
        val mime = mimeForCodec(name)
        val format = MediaFormat.createVideoFormat(mime, w, h).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
            // Frames only when the picture changes; repeat now and then so a
            // still app still reaches a PC that just joined.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                // KEY_FRAME_RATE describes the stream but does not cap a
                // 90/120 Hz display feeding a surface-input encoder.
                setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, fps.toFloat())
                setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                setInteger(MediaFormat.KEY_LATENCY, 1)
            }
        }
        val codec = MediaCodec.createEncoderByType(mime)
        var surface: Surface? = null
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface()
            codec.start()
            return EncodedSurface(name, codec, surface)
        } catch (error: Exception) {
            runCatching { surface?.release() }
            runCatching { codec.release() }
            throw error
        }
    }

    private fun createEncoder(w: Int, h: Int): EncodedSurface {
        if (codecName != "h264") {
            try {
                return createEncoderForCodec(codecName, w, h)
            } catch (error: Exception) {
                android.util.Log.i("AppDisplay", "$codecName screen encoder unavailable; using H.264", error)
            }
        }
        return createEncoderForCodec("h264", w, h)
    }

    private fun writeConfig(
        out: DataOutputStream,
        width: Int,
        height: Int,
        displayId: Int,
        codec: String,
    ) {
        val codecId = when (codec) {
            "h264" -> CODEC_ID_H264
            "h265" -> CODEC_ID_H265
            "av1" -> CODEC_ID_AV1
            else -> error("Unsupported screen codec: $codec")
        }
        write(out, KIND_CONFIG, 0, ByteArray(16).also {
            java.nio.ByteBuffer.wrap(it).putInt(width).putInt(height).putInt(displayId).putInt(codecId)
        })
        out.flush()
    }

    private fun drain(codec: MediaCodec, out: DataOutputStream) {
        val info = MediaCodec.BufferInfo()
        var parameters = ByteArray(0)
        while (running && nextSize == null) {
            followAppOrientation()
            val index = codec.dequeueOutputBuffer(info, 50_000)
            if (index < 0) continue
            val buffer = codec.getOutputBuffer(index)
            if (buffer != null && info.size > 0) {
                val bytes = ByteArray(info.size)
                buffer.position(info.offset)
                buffer.get(bytes)
                when {
                    info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0 -> parameters = bytes
                    info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0 -> {
                        write(out, KIND_KEYFRAME, info.presentationTimeUs, parameters + bytes)
                        out.flush()
                    }
                    else -> {
                        write(out, KIND_FRAME, info.presentationTimeUs, bytes)
                        out.flush()
                    }
                }
            }
            codec.releaseOutputBuffer(index, false)
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
        }
    }

    private fun write(out: DataOutputStream, kind: Int, timeUs: Long, bytes: ByteArray) {
        out.writeInt(1 + 8 + bytes.size)
        out.writeByte(kind)
        out.writeLong(timeUs)
        out.write(bytes)
    }

    private fun createMirrorDisplay(w: Int, h: Int, d: Int, surface: Surface): VirtualDisplay {
        // 1. Android 11+ hidden static helper DisplayManager.createVirtualDisplay(name, width, height, displayIdToMirror, surface)
        runCatching {
            val m = DisplayManager::class.java.getMethod(
                "createVirtualDisplay",
                String::class.java,
                Int::class.javaPrimitiveType,
                Int::class.javaPrimitiveType,
                Int::class.javaPrimitiveType,
                Surface::class.java,
            )
            val vd = m.invoke(null, "NectarlinkScreen", w, h, 0, surface) as? VirtualDisplay
            if (vd != null) return vd
        }
        // 2. Shell DisplayManager instance with VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR
        val constructor = DisplayManager::class.java.getDeclaredConstructor(Context::class.java)
        constructor.isAccessible = true
        val manager = constructor.newInstance(ShellContext.get())
        @SuppressLint("WrongConstant")
        return runCatching {
            manager.createVirtualDisplay(
                "NectarlinkScreen",
                w,
                h,
                d,
                surface,
                PUBLIC or DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR or DisplayManager.VIRTUAL_DISPLAY_FLAG_PRESENTATION,
            )
        }.getOrElse {
            manager.createVirtualDisplay(
                "NectarlinkScreen",
                w,
                h,
                d,
                surface,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            )
        }
    }

    private fun createDisplay(w: Int, h: Int, d: Int, surface: Surface): VirtualDisplay =
        try {
            createDisplay(w, h, d, surface, public = true)
        } catch (e: SecurityException) {
            // A public display mirrors unless it shows only its own content, and stock
            // Android lets the shell make one that doesn't. Some ROMs mirror it anyway
            // and then want CAPTURE_VIDEO_OUTPUT, which the shell only has from Android
            // 14 (#136). A private display needs no such permission, and the shell
            // owns it, so it can still start the app there.
            if (e.message?.contains("CAPTURE_VIDEO_OUTPUT") != true) throw e
            System.err.println("a public display was refused (${e.message}); making a private one")
            createDisplay(w, h, d, surface, public = false)
        }

    private fun createDisplay(w: Int, h: Int, d: Int, surface: Surface, public: Boolean): VirtualDisplay {
        var flags = OWN_CONTENT_ONLY or SUPPORTS_TOUCH or ROTATES_WITH_CONTENT or DESTROY_CONTENT_ON_REMOVAL
        if (public) flags = flags or PUBLIC
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            flags = flags or TRUSTED or OWN_DISPLAY_GROUP or ALWAYS_UNLOCKED or TOUCH_FEEDBACK_DISABLED
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            flags = flags or OWN_FOCUS or DEVICE_DISPLAY_GROUP
            ownedDisplay(w, h, d, surface, flags)?.let { return it }
        }
        wakePhone()
        val constructor = DisplayManager::class.java.getDeclaredConstructor(Context::class.java)
        constructor.isAccessible = true
        val manager = constructor.newInstance(ShellContext.get())
        // Some of the flags are hidden ones (TRUSTED, OWN_FOCUS...), which
        // the shell may use.
        @SuppressLint("WrongConstant")
        return manager.createVirtualDisplay("Nectarlink", w, h, d, surface, flags)
    }

    /**
     * A display owned by a virtual device, which runs while the phone sleeps
     * (see [VirtualDevice]), or null to make a plain one.
     */
    @androidx.annotation.RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    private fun ownedDisplay(w: Int, h: Int, d: Int, surface: Surface, flags: Int): VirtualDisplay? {
        val owner = VirtualDevice.create() ?: return null
        return runCatching {
            owner.createVirtualDisplay("Nectarlink", w, h, d, surface, flags and OWN_DISPLAY_GROUP.inv())
        }.onSuccess { device = owner }.onFailure {
            System.err.println("no display on the virtual device: $it")
            owner.close()
        }.getOrNull()
    }

    /**
     * The window manager's own density for [display] ([dpi] null: none).
     * Resizing a display to the size it has with another density reached
     * the app only seconds later, or with the next touch; this is applied
     * at once (the app draws anew within half a second). Taken off when the
     * window closes: it's remembered by the display's name, which another
     * window may get.
     */
    private fun forceDensity(display: Int, dpi: Int?) {
        runCatching {
            Runtime.getRuntime()
                .exec(arrayOf("wm", "density", dpi?.toString() ?: "reset", "-d", display.toString()))
                .waitFor()
        }.onFailure { System.err.println("can't set the density of display $display: $it") }
    }

    /** Hides the soft keyboard on this virtual display so typing from the PC never pops it up on the phone. */
    private fun hideIme(display: Int) {
        val hidden = runCatching {
            val wmg = Class.forName("android.view.WindowManagerGlobal")
            val wm = wmg.getMethod("getWindowManagerService").invoke(null) ?: return@runCatching false
            wm.javaClass
                .getMethod("setDisplayImePolicy", Int::class.javaPrimitiveType, Int::class.javaPrimitiveType)
                .invoke(wm, display, DISPLAY_IME_POLICY_HIDE)
            true
        }.getOrDefault(false)
        if (!hidden) {
            runCatching {
                Runtime.getRuntime().exec(arrayOf("wm", "set-display-ime-policy", display.toString(), "hide")).waitFor()
            }
        }
    }

    private fun launch(pkg: String, display: Int) {
        // By its launcher activity's name: a package-only launcher intent isn't resolved on some
        // phones (Samsung's One UI answers "intent not resolved"), and nothing would open.
        val target = launcherActivity(pkg)?.let { arrayOf("-n", it) }
            ?: arrayOf("-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-p", pkg)
        val command = arrayOf("am", "start", "--display", display.toString(), "--windowingMode", "1") + target +
            arrayOf("-f", (FLAG_NEW_TASK or FLAG_MULTIPLE_TASK).toString())
        val process = Runtime.getRuntime().exec(command)
        val said = process.inputStream.bufferedReader().readText() + process.errorStream.bufferedReader().readText()
        process.waitFor()
        if (said.contains("Error", ignoreCase = true) || said.contains("Warning", ignoreCase = true)) {
            System.err.println("launching $pkg: ${said.trim().lines().joinToString(" | ")}")
        }
    }

    /** `pkg/.Activity` for the package's launcher activity, or null. */
    private fun launcherActivity(pkg: String): String? = runCatching {
        val process = Runtime.getRuntime().exec(
            arrayOf(
                "cmd", "package", "resolve-activity", "--brief", "--user", "0",
                "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", pkg,
            ),
        )
        val last = process.inputStream.bufferedReader().readLines().lastOrNull { it.isNotBlank() }?.trim()
        process.waitFor()
        last?.takeIf { it.startsWith("$pkg/") }
    }.getOrNull()

    /**
     * Tells Android there is activity on [display] every few seconds while
     * the window is open. Otherwise the phone sleeps from inactivity, which
     * stops the app and freezes its window on the PC. The phone's own screen
     * still dims as usual (scrcpy's `--keep-active` does the same).
     */
    private fun keepActive(display: Int) {
        val signal = UserActivity.forDisplay(display) ?: return
        Thread({
            try {
                while (running && signal()) {
                    Thread.sleep(KEEP_ACTIVE_MS)
                }
            } catch (_: InterruptedException) {
            }
        }, "app-display-active").apply {
            isDaemon = true
            start()
        }
    }

    /** Stops the stream when the app on [display] finishes its last activity. */
    private fun watchActivities(display: Int) {
        Thread({
            try {
                Thread.sleep(1_500)
                var seen = false
                var emptyCount = 0
                var ticks = 0
                while (running) {
                    if (hasActivityOnDisplay(display)) {
                        seen = true
                        emptyCount = 0
                    } else if (seen || ticks >= 15) {
                        emptyCount++
                        if (emptyCount >= 2) {
                            System.err.println("the app on display $display closed")
                            running = false
                            break
                        }
                    }
                    ticks++
                    Thread.sleep(1_000)
                }
            } catch (_: InterruptedException) {
            }
        }, "app-display-watch").apply {
            isDaemon = true
            start()
        }
    }

    /**
     * Whether the display still has a visible task, asked of the activity
     * manager (Android 12+). When it can't tell, the window stays open:
     * closing someone's window by mistake is worse than a stale picture.
     */
    @SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    private fun hasActivityOnDisplay(display: Int): Boolean = runCatching {
        val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
            ?: return@runCatching true
        val tasks = service.javaClass
            .getMethod("getAllRootTaskInfosOnDisplay", Int::class.javaPrimitiveType)
            .invoke(service, display) as List<*>
        tasks.any { task ->
            if (task == null) return@any false
            val visible = runCatching { task.javaClass.getField("visible").getBoolean(task) }.getOrDefault(true)
            val children = runCatching { (task.javaClass.getField("childTaskIds").get(task) as IntArray).size }.getOrDefault(1)
            visible && children > 0
        }
    }.getOrDefault(true)

    companion object {
        const val KIND_CONFIG = 0
        const val KIND_FRAME = 1
        const val KIND_KEYFRAME = 2
        const val CODEC_ID_H264 = 1
        const val CODEC_ID_H265 = 2
        const val CODEC_ID_AV1 = 3

        /** How often an open window counts as activity; well under any screen timeout. */
        private const val KEEP_ACTIVE_MS = 4_000L
        private const val ROTATION_CHECK_MS = 250L

        /** WindowManager.DISPLAY_IME_POLICY_HIDE. */
        private const val DISPLAY_IME_POLICY_HIDE = 2

        // DisplayManager.VIRTUAL_DISPLAY_FLAG_* (some hidden).
        private const val PUBLIC = 1
        private const val OWN_CONTENT_ONLY = 1 shl 3
        private const val SUPPORTS_TOUCH = 1 shl 6
        private const val ROTATES_WITH_CONTENT = 1 shl 7
        private const val DESTROY_CONTENT_ON_REMOVAL = 1 shl 8
        private const val TRUSTED = 1 shl 10
        private const val OWN_DISPLAY_GROUP = 1 shl 11
        private const val ALWAYS_UNLOCKED = 1 shl 12
        private const val TOUCH_FEEDBACK_DISABLED = 1 shl 13
        private const val OWN_FOCUS = 1 shl 14
        private const val DEVICE_DISPLAY_GROUP = 1 shl 15

        private const val FLAG_NEW_TASK = 0x10000000
        private const val FLAG_MULTIPLE_TASK = 0x08000000
    }
}

/**
 * A context for system services in the shell-side helper, which has no app
 * of its own: the system context, speaking as the shell package (scrcpy
 * does the same).
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "SoonBlockedPrivateApi", "BlockedPrivateApi")
internal class ShellContext private constructor(base: Context) : ContextWrapper(base) {
    override fun getPackageName(): String = SHELL
    override fun getOpPackageName(): String = SHELL
    override fun getApplicationContext(): Context = this

    @Suppress("NewApi")
    override fun getAttributionSource(): android.content.AttributionSource =
        android.content.AttributionSource.Builder(android.os.Process.SHELL_UID).setPackageName(SHELL).build()

    companion object {
        private const val SHELL = "com.android.shell"
        @Volatile private var instance: ShellContext? = null

        @Synchronized
        fun get(): ShellContext = instance ?: ShellContext(systemContext()).also { instance = it }

        private fun systemContext(): Context {
            if (Looper.getMainLooper() == null) {
                @Suppress("DEPRECATION")
                Looper.prepareMainLooper()
            }
            val type = Class.forName("android.app.ActivityThread")
            val constructor = type.getDeclaredConstructor().apply { isAccessible = true }
            val thread = constructor.newInstance()
            type.getDeclaredField("sCurrentActivityThread").apply { isAccessible = true }.set(null, thread)
            // Hidden-API rules are for apps; this runs in the shell's own process.
            runCatching { type.getDeclaredField("mSystemThread").apply { isAccessible = true }.setBoolean(thread, true) }
            fillConfigurationController(type, thread)
            return type.getDeclaredMethod("getSystemContext").invoke(thread) as Context
        }

        /**
         * A bare ActivityThread has no configuration controller (Android 13+). Samsung's
         * display sandboxing asks the thread for its configuration while the system context is
         * made, which then fails. Gives it a controller holding the default configuration.
         */
        private fun fillConfigurationController(type: Class<*>, thread: Any) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
            runCatching {
                val controllerType = Class.forName("android.app.ConfigurationController")
                val internal = Class.forName("android.app.ActivityThreadInternal")
                val controller = controllerType.getDeclaredConstructor(internal)
                    .apply { isAccessible = true }
                    .newInstance(thread)
                runCatching {
                    controllerType.getDeclaredField("mConfiguration").apply { isAccessible = true }
                        .set(controller, android.content.res.Configuration().apply { setToDefaults() })
                }
                type.getDeclaredField("mConfigurationController").apply { isAccessible = true }.set(thread, controller)
            }.onFailure { System.err.println("no configuration controller: $it") }
        }
    }
}

/** `IPowerManager.userActivity`, which the shell may call (DEVICE_POWER). */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi")
private object UserActivity {
    private val power: Any? by lazy {
        runCatching {
            val binder = Class.forName("android.os.ServiceManager")
                .getMethod("getService", String::class.java)
                .invoke(null, "power") as android.os.IBinder
            Class.forName("android.os.IPowerManager\$Stub")
                .getMethod("asInterface", android.os.IBinder::class.java)
                .invoke(null, binder)
        }.onFailure { System.err.println("no power manager: $it") }.getOrNull()
    }

    /**
     * A call that counts as activity on [display] (false once Android refuses
     * it), or null if there is none. Before Android 12 activity is for the
     * whole device.
     */
    fun forDisplay(display: Int): (() -> Boolean)? {
        val manager = power ?: return null
        val perDisplay = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
        val int = Int::class.javaPrimitiveType
        val long = Long::class.javaPrimitiveType
        val method = runCatching {
            if (perDisplay) {
                manager.javaClass.getMethod("userActivity", int, long, int, int)
            } else {
                manager.javaClass.getMethod("userActivity", long, int, int)
            }
        }.onFailure { System.err.println("no userActivity: $it") }.getOrNull() ?: return null
        return {
            val now = android.os.SystemClock.uptimeMillis()
            runCatching {
                if (perDisplay) {
                    method.invoke(manager, display, now, EVENT_OTHER, NO_CHANGE_LIGHTS)
                } else {
                    method.invoke(manager, now, EVENT_OTHER, NO_CHANGE_LIGHTS)
                }
            }.onFailure { System.err.println("user activity: $it") }.isSuccess
        }
    }

    /** PowerManager.USER_ACTIVITY_EVENT_OTHER. */
    private const val EVENT_OTHER = 0

    /** PowerManager.USER_ACTIVITY_FLAG_NO_CHANGE_LIGHTS: the screen may still dim. */
    private const val NO_CHANGE_LIGHTS = 1
}
