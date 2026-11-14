// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Build
import android.util.Log
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.nectarlink.android.R
import app.nectarlink.android.elevated.AppDisplay
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.MirrorOptions
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.PhoneApp
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorConfig
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.EOFException
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/**
 * Phone apps in windows of their own on a PC (Elevated): each runs on a
 * display the shell-side helper makes for it, whose video goes to the PC
 * as a mirroring session of its own. A notification says so while any are
 * open, with a button to close them.
 */
class AppWindows(
    context: Context,
    private val scope: CoroutineScope,
    /** Opens a video stream to a PC. */
    private val open: suspend (String) -> MirrorStream,
    /** A PC's name. */
    private val nameOf: (String) -> String,
) {
    private val context = context.applicationContext

    private class Window(
        val pc: String,
        val session: UInt,
        val socket: Socket,
        @Volatile var width: Int,
        @Volatile var height: Int,
        val dpi: Int,
        val maxSize: Int,
    ) {
        /** The PC's zoom, in percent of [dpi]. */
        @Volatile var zoom: Int = 100
        @Volatile var display: Int = -1
    }

    private val windows = ConcurrentHashMap<Pair<String, UInt>, Window>()

    /** What each PC asks of the phone while its windows are open: (stay awake, screen off). */
    private val power = ConcurrentHashMap<String, Pair<Boolean, Boolean>>()

    init {
        instance = this
    }

    /** Runs the PC's app on a display of its own and streams it; false if it can't. */
    fun open(pc: String, options: MirrorOptions): Boolean {
        val pkg = options.app ?: return false
        if (pkg == context.packageName) return false
        val maxSize = options.maxSize.toInt().coerceIn(320, 1920)
        val (width, height, dpi) = size(maxSize)
        // The phone stays as it is: the helper gives the window a display
        // that runs while the phone sleeps, or wakes the phone when it
        // can't (see AppDisplay.createDisplay).
        // Straight off when it's meant to stay off, so it barely lights up.
        if (options.screenOff) Elevated.setScreenOff(true)
        val socket = Elevated.openApp(pkg, width, height, dpi, options.bitrate.toInt(), options.fps.toInt()) ?: run {
            if (options.screenOff && windows.isEmpty()) Elevated.setScreenOff(false)
            return false
        }
        val window = Window(pc, options.session, socket, width, height, dpi, maxSize)
        power[pc] = options.stayAwake to options.screenOff
        windows.put(pc to options.session, window)?.let { runCatching { it.socket.close() } }
        changed()
        scope.launch(Dispatchers.IO) {
            val stream = runCatching { open(pc) }.getOrElse {
                Log.w(TAG, "can't stream an app window", it)
                close(window)
                return@launch
            }
            Thread({ forward(window, stream) }, "app-window").start()
        }
        return true
    }

    /** The helper's packets to the PC, until either side closes. */
    private fun forward(window: Window, stream: MirrorStream) {
        try {
            val input = DataInputStream(window.socket.getInputStream().buffered(1 shl 16))
            while (true) {
                val length = input.readInt()
                if (length < 9 || length > MAX_PACKET) return
                val kind = input.readUnsignedByte()
                val time = input.readLong().toULong()
                val data = ByteArray(length - 9).also { input.readFully(it) }
                val result = when (kind) {
                    AppDisplay.KIND_CONFIG -> {
                        val fields = java.nio.ByteBuffer.wrap(data)
                        val (w, h) = fields.int to fields.int
                        window.width = w
                        window.height = h
                        window.display = fields.int
                        stream.send(VideoPacketKind.CONFIG, 0u, mirrorConfig(w.toUInt(), h.toUInt(), window.session))
                    }
                    AppDisplay.KIND_KEYFRAME -> stream.send(VideoPacketKind.KEYFRAME, time, data)
                    else -> stream.send(VideoPacketKind.FRAME, time, data)
                }
                when (result) {
                    MirrorSendResult.CLOSED -> return
                    MirrorSendResult.NEED_KEYFRAME, MirrorSendResult.DROPPED -> keyframe(window)
                    MirrorSendResult.QUEUED -> {}
                }
            }
        } catch (_: EOFException) {
        } catch (e: java.io.IOException) {
            Log.d(TAG, "an app window ended", e)
        } finally {
            stream.end()
            close(window)
        }
    }

    /** The PC closed a window. */
    fun close(pc: String, session: UInt) {
        windows[pc to session]?.let(::close)
    }

    private fun close(window: Window) {
        runCatching { window.socket.close() }
        if (windows.remove(window.pc to window.session, window)) changed()
    }

    fun closeAll() {
        windows.values.toList().forEach(::close)
    }

    fun keyframe(pc: String, session: UInt) {
        windows[pc to session]?.let(::keyframe)
    }

    private fun keyframe(window: Window) {
        runCatching {
            synchronized(window) {
                window.socket.getOutputStream().apply {
                    write("K\n".toByteArray())
                    flush()
                }
            }
        }
    }

    /**
     * The PC resized an app window: its own size, fit to encoder limits, so
     * the picture fills a window of any shape, at the PC's [zoom] (percent
     * of the window's usual density); tell the helper.
     */
    fun resize(pc: String, session: UInt, width: Int, height: Int, zoom: Int) {
        val window = windows[pc to session] ?: return
        val (newW, newH) = WindowShape.exact(width, height, window.maxSize)
        if (newW == window.width && newH == window.height && zoom == window.zoom) return
        window.width = newW
        window.height = newH
        window.zoom = zoom
        val dpi = (window.dpi * zoom / 100).coerceIn(72, 1000)
        runCatching {
            synchronized(window) {
                window.socket.getOutputStream().apply {
                    write("R $newW $newH $dpi\n".toByteArray())
                    flush()
                }
            }
        }
    }

    /** The display a PC's window runs on, once it has one. */
    fun displayOf(pc: String, session: UInt): Int? = windows[pc to session]?.display?.takeIf { it >= 0 }

    /** The PC's mouse and keyboard on a window. */
    fun input(pc: String, session: UInt, event: MirrorInputEvent) {
        val window = windows[pc to session] ?: return
        if (window.display < 0) return
        Elevated.handleOn(window.display, window.width.toFloat(), window.height.toFloat(), event)
    }

    /** The phone's screen, in pixels. */
    private fun screen(): Pair<Int, Int> {
        val windows = context.getSystemService(WindowManager::class.java)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windows.maximumWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windows.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels to metrics.heightPixels
        }
    }

    /** The phone's screen shape scaled to fit [maxSize] (see [WindowShape]), upright, and its density likewise. */
    private fun size(maxSize: Int): Triple<Int, Int, Int> {
        val (w, h) = screen()
        val (shortSide, longSide) = minOf(w, h) to maxOf(w, h)
        val (fitW, fitH) = WindowShape.fit(shortSide, longSide, maxSize.coerceAtLeast(320), w, h)
        val scale = fitH.toFloat() / longSide
        val dpi = (context.resources.displayMetrics.densityDpi * scale).roundToInt().coerceIn(120, 640)
        return Triple(fitW, fitH, dpi)
    }

    /** The PC turned keeping the phone awake, or its screen off, on or off. */
    fun setPower(pc: String, stayAwake: Boolean, screenOff: Boolean) {
        if (windows.values.none { it.pc == pc }) return
        power[pc] = stayAwake to screenOff
        applyPower()
    }

    private fun applyPower() {
        val open = windows.values.map { it.pc }.toSet()
        power.keys.retainAll(open)
        val wants = power.values
        StreamPower.set(context, StreamPower.APPS, open.isNotEmpty(), wants.any { it.first }, wants.any { it.second })
    }

    private fun changed() {
        applyPower()
        val manager = NotificationManagerCompat.from(context)
        if (windows.isEmpty()) {
            manager.cancel(NOTIFICATION_ID)
            return
        }
        val allowed = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        if (!allowed) return
        MirrorService.createChannel(context)
        val pcs = windows.values.map { it.pc }.distinct().map(nameOf).filter { it.isNotEmpty() }
        val closeAll = PendingIntent.getBroadcast(
            context, 0, Intent(context, CloseReceiver::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, MirrorService.CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(
                context.resources.getQuantityString(R.plurals.app_windows_title, windows.size, windows.size),
            )
            .setContentText(
                context.getString(R.string.app_windows_text, pcs.joinToString().ifEmpty { context.getString(R.string.your_pc) }),
            )
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, context.getString(R.string.action_close_all), closeAll)
            .build()
        try {
            manager.notify(NOTIFICATION_ID, notification)
        } catch (e: SecurityException) {
            Log.i(TAG, "can't say the app windows are open", e)
        }
    }

    /** The notification's Close all. */
    class CloseReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            instance?.closeAll()
        }
    }

    companion object {
        private const val TAG = "AppWindows"
        private const val NOTIFICATION_ID = 10
        /** Packets from the helper, at most. */
        private const val MAX_PACKET = 8 shl 20

        @Volatile private var instance: AppWindows? = null

        fun closeAllCurrent() {
            instance?.closeAll()
        }

        /** The apps a PC may open in windows: launchable ones, by name, with small icons. */
        fun list(context: Context): List<PhoneApp> {
            val manager = context.packageManager
            val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
            val found = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                manager.queryIntentActivities(launcher, PackageManager.ResolveInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                manager.queryIntentActivities(launcher, 0)
            }
            val used = HashSet<String>()
            val apps = found
                .map { it.activityInfo }
                .filter { it.packageName != context.packageName }
                .distinctBy { it.packageName }
                .map { info ->
                    PhoneApp(
                        pkg = info.packageName,
                        label = info.loadLabel(manager).toString().trim().ifEmpty { info.packageName },
                        icon = cachedIcon(info, manager, used),
                    )
                }
                .sortedBy { it.label.lowercase() }
            // Forget uninstalled and updated apps' icons.
            icons.keys.retainAll(used)
            return apps
        }

        /**
         * Icons drawn for an earlier list, by package and its installed APK
         * (a new path after every update) and icon: drawing and compressing
         * every icon again made every list slow to arrive.
         */
        private val icons = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
        private val NO_ICON = ByteArray(0)

        private fun cachedIcon(
            info: android.content.pm.ActivityInfo,
            manager: PackageManager,
            used: MutableSet<String>,
        ): ByteArray? {
            val key = "${info.packageName}|${info.applicationInfo?.sourceDir}|${info.iconResource}"
            used += key
            val png = icons.getOrPut(key) { runCatching { icon(info.loadIcon(manager)) }.getOrNull() ?: NO_ICON }
            return png.takeIf { it.isNotEmpty() }
        }

        /** A PNG of the icon: 64 pixels, or 48 when that's too big. */
        private fun icon(drawable: android.graphics.drawable.Drawable): ByteArray? {
            for (size in intArrayOf(64, 48)) {
                val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
                drawable.setBounds(0, 0, size, size)
                drawable.draw(Canvas(bitmap))
                val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
                bitmap.recycle()
                if (png.size <= MAX_ICON_BYTES) return png
            }
            return null
        }

        private const val MAX_ICON_BYTES = 8 * 1024
    }
}
