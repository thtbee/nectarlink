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
 * displays), with an H.264 encoder drawing from it, and the app started on
 * it. The encoded video goes to [out] as packets:
 * `u32 length | u8 kind | u64 time µs | data` (kind 0: config, as
 * `u32 width | u32 height | u32 display id`; 1: frame; 2: keyframe).
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
class AppDisplay(
    private val width: Int,
    private val height: Int,
    private val dpi: Int,
    private val bitrate: Int,
    private val fps: Int,
) {
    @Volatile private var running = true
    @Volatile private var nextSize: Triple<Int, Int, Int>? = null
    private var display: VirtualDisplay? = null

    /** The display's ID, once made. */
    var displayId: Int = -1
        private set

    /** Streams until [stop] or until [out] fails; starts [pkg] on the display first. */
    fun run(pkg: String, out: OutputStream) {
        var curW = width
        var curH = height
        var curDpi = dpi
        var codec: MediaCodec? = null
        var surface: Surface? = null
        val data = DataOutputStream(out.buffered(1 shl 16))
        try {
            val (firstCodec, firstSurface) = createEncoder(curW, curH)
            codec = firstCodec
            surface = firstSurface
            val made = createDisplay(curW, curH, curDpi, firstSurface)
            display = made
            displayId = made.display.displayId
            hideIme(displayId)
            firstCodec.start()
            codecForKeyframes = firstCodec
            write(data, KIND_CONFIG, 0, ByteArray(12).also {
                java.nio.ByteBuffer.wrap(it).putInt(curW).putInt(curH).putInt(displayId)
            })
            data.flush()
            launch(pkg, displayId)
            watchActivities(displayId)
            while (running) {
                drain(codec!!, data)
                if (!running) break
                val (newW, newH, newDpi) = nextSize ?: continue
                nextSize = null
                if (newW == curW && newH == curH && newDpi == curDpi) continue
                codecForKeyframes = null
                runCatching { codec.stop() }
                runCatching { codec.release() }
                codec = null
                runCatching { surface?.release() }
                surface = null
                val (nextCodec, nextSurface) = createEncoder(newW, newH)
                codec = nextCodec
                surface = nextSurface
                nextCodec.start()
                codecForKeyframes = nextCodec
                made.resize(newW, newH, newDpi)
                made.surface = nextSurface
                curW = newW
                curH = newH
                curDpi = newDpi
                write(data, KIND_CONFIG, 0, ByteArray(12).also {
                    java.nio.ByteBuffer.wrap(it).putInt(curW).putInt(curH).putInt(displayId)
                })
                data.flush()
            }
        } finally {
            running = false
            codecForKeyframes = null
            runCatching { codec?.stop() }
            runCatching { codec?.release() }
            runCatching { surface?.release() }
            display?.release()
            display = null
        }
    }

    fun stop() {
        running = false
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

    private fun createEncoder(w: Int, h: Int): Pair<MediaCodec, Surface> {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
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
            setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                setInteger(MediaFormat.KEY_LATENCY, 1)
            }
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        return codec to codec.createInputSurface()
    }

    private fun drain(codec: MediaCodec, out: DataOutputStream) {
        val info = MediaCodec.BufferInfo()
        var parameters = ByteArray(0)
        while (running && nextSize == null) {
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

    private fun createDisplay(w: Int, h: Int, d: Int, surface: Surface): VirtualDisplay {
        var flags = PUBLIC or OWN_CONTENT_ONLY or SUPPORTS_TOUCH or ROTATES_WITH_CONTENT or DESTROY_CONTENT_ON_REMOVAL
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            flags = flags or TRUSTED or OWN_DISPLAY_GROUP or ALWAYS_UNLOCKED or TOUCH_FEEDBACK_DISABLED
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            flags = flags or OWN_FOCUS or DEVICE_DISPLAY_GROUP
        }
        val constructor = DisplayManager::class.java.getDeclaredConstructor(Context::class.java)
        constructor.isAccessible = true
        val manager = constructor.newInstance(ShellContext.get())
        // Some of the flags are hidden ones (TRUSTED, OWN_FOCUS...), which
        // the shell may use.
        @SuppressLint("WrongConstant")
        return manager.createVirtualDisplay("Nectarlink", w, h, d, surface, flags)
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
        val command = arrayOf(
            "am", "start", "--display", display.toString(), "--windowingMode", "1",
            "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-p", pkg,
            "-f", (FLAG_NEW_TASK or FLAG_MULTIPLE_TASK).toString(),
        )
        Runtime.getRuntime().exec(command).waitFor()
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
        val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)!!
        val tasks = service.javaClass
            .getMethod("getAllRootTaskInfosOnDisplay", Int::class.javaPrimitiveType)
            .invoke(service, display) as List<*>
        tasks.any { task ->
            val visible = runCatching { task!!.javaClass.getField("visible").getBoolean(task) }.getOrDefault(true)
            val children = runCatching { (task!!.javaClass.getField("childTaskIds").get(task) as IntArray).size }.getOrDefault(1)
            visible && children > 0
        }
    }.getOrDefault(true)

    companion object {
        const val KIND_CONFIG = 0
        const val KIND_FRAME = 1
        const val KIND_KEYFRAME = 2

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
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "SoonBlockedPrivateApi")
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
            return type.getDeclaredMethod("getSystemContext").invoke(thread) as Context
        }
    }
}
