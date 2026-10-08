// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.app.WallpaperColors
import android.app.WallpaperManager
import android.content.BroadcastReceiver
import android.content.ComponentCallbacks
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.Configuration
import android.graphics.Path
import android.graphics.PathMeasure
import android.hardware.display.DisplayManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.DisplayMetrics
import android.view.Display
import android.view.RoundedCorner
import android.view.Surface
import android.view.WindowManager
import androidx.core.content.ContextCompat
import app.nectarlink.core.Battery
import app.nectarlink.core.DeviceInfo
import app.nectarlink.core.DeviceKind
import app.nectarlink.core.ScreenCorners
import app.nectarlink.core.ScreenRect
import app.nectarlink.core.ScreenShape
import java.util.Locale

internal fun normalizeArgbSeed(argb: Int): UInt = (argb.toUInt() or 0xFF00_0000u)

/**
 * Reads the phone's Material You seed color (`0xFFRRGGBB` ARGB) on Android 12+
 * (`API 31+`) without requiring any runtime permission.
 */
fun materialYouAccent(context: Context, wallpaperColors: WallpaperColors? = null): UInt? {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return null
    val sys = runCatching { context.getColor(android.R.color.system_accent1_500) }.getOrNull()
    val wp = wallpaperColors?.primaryColor?.toArgb()
        ?: runCatching {
            context.getSystemService(WallpaperManager::class.java)
                ?.getWallpaperColors(WallpaperManager.FLAG_SYSTEM)
                ?.primaryColor
                ?.toArgb()
        }.getOrNull()
    val raw = sys ?: wp ?: return null
    return normalizeArgbSeed(raw)
}

/** How this phone presents itself to PCs. */
fun deviceInfo(context: Context): DeviceInfo {
    val name = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
        ?.takeIf { it.isNotBlank() } ?: Build.MODEL
    val tablet = context.resources.configuration.smallestScreenWidthDp >= 600
    return DeviceInfo(
        name = name,
        kind = if (tablet) DeviceKind.TABLET else DeviceKind.PHONE,
        os = "android",
        osVersion = Build.VERSION.RELEASE,
        model = "${Build.MANUFACTURER.replaceFirstChar(Char::uppercase)} ${Build.MODEL}",
        accent = materialYouAccent(context),
        screen = measureScreenShape(context),
    )
}

/**
 * Measures the phone's physical front screen shape in its natural (`ROTATION_0`)
 * orientation, normalized to fractions of the display so the PC can draw the
 * exact aspect ratio, corner radii and camera cutout without model tables.
 */
fun measureScreenShape(context: Context): ScreenShape? = runCatching {
    val wm = context.getSystemService(WindowManager::class.java) ?: return null
    val display = context.getSystemService(DisplayManager::class.java)
        ?.getDisplay(Display.DEFAULT_DISPLAY)
    val rotation = display?.rotation ?: Surface.ROTATION_0

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        val metrics = wm.maximumWindowMetrics
        val bounds = metrics.bounds
        val rawW = bounds.width()
        val rawH = bounds.height()
        val (w0, h0) = unrotateDimensions(rawW, rawH, rotation)
        if (w0 <= 0 || h0 <= 0) return null

        val insets = metrics.windowInsets
        val cutout = insets.displayCutout
        val cutouts = cutout?.boundingRects.orEmpty()
            .take(4)
            .mapNotNull { r ->
                normalizeCutoutRect(r.left, r.top, r.right, r.bottom, rawW, rawH, rotation)
            }

        val corners = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            fun cornerRadius(pos: Int): Int =
                insets.getRoundedCorner(pos)?.radius
                    ?: display?.getRoundedCorner(pos)?.radius
                    ?: 0
            normalizeCorners(
                topLeftPx = cornerRadius(RoundedCorner.POSITION_TOP_LEFT),
                topRightPx = cornerRadius(RoundedCorner.POSITION_TOP_RIGHT),
                bottomRightPx = cornerRadius(RoundedCorner.POSITION_BOTTOM_RIGHT),
                bottomLeftPx = cornerRadius(RoundedCorner.POSITION_BOTTOM_LEFT),
                rawW = rawW,
                rawH = rawH,
                rotation = rotation,
            )
        } else {
            null
        }

        val cutoutPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            cutout?.cutoutPath?.let { sampleCutoutPath(it, rawW, rawH, rotation) }
        } else {
            null
        }

        ScreenShape(
            aspect = (w0.toFloat() / h0.toFloat()).coerceIn(0.25f, 2.5f),
            corners = corners,
            cutouts = cutouts,
            cutoutPath = cutoutPath,
        )
    } else {
        @Suppress("DEPRECATION")
        val legacyDisplay = display ?: wm.defaultDisplay ?: return null
        val dm = DisplayMetrics()
        @Suppress("DEPRECATION")
        legacyDisplay.getRealMetrics(dm)
        val rawW = dm.widthPixels
        val rawH = dm.heightPixels
        val (w0, h0) = unrotateDimensions(rawW, rawH, rotation)
        if (w0 <= 0 || h0 <= 0) return null

        val cutouts = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            legacyDisplay.cutout?.boundingRects.orEmpty()
                .take(4)
                .mapNotNull { r ->
                    normalizeCutoutRect(r.left, r.top, r.right, r.bottom, rawW, rawH, rotation)
                }
        } else {
            emptyList()
        }

        ScreenShape(
            aspect = (w0.toFloat() / h0.toFloat()).coerceIn(0.25f, 2.5f),
            corners = null,
            cutouts = cutouts,
            cutoutPath = null,
        )
    }
}.getOrNull()

internal fun unrotateDimensions(rawW: Int, rawH: Int, rotation: Int): Pair<Int, Int> = when (rotation) {
    Surface.ROTATION_90, Surface.ROTATION_270 -> rawH to rawW
    else -> rawW to rawH
}

internal fun unrotatePoint(x: Float, y: Float, rawW: Float, rawH: Float, rotation: Int): Pair<Float, Float> =
    when (rotation) {
        Surface.ROTATION_90 -> (rawH - y) to x
        Surface.ROTATION_180 -> (rawW - x) to (rawH - y)
        Surface.ROTATION_270 -> y to (rawW - x)
        else -> x to y
    }

internal fun normalizeCutoutRect(
    left: Int,
    top: Int,
    right: Int,
    bottom: Int,
    rawW: Int,
    rawH: Int,
    rotation: Int,
): ScreenRect? {
    if (rawW <= 0 || rawH <= 0 || right <= left || bottom <= top) return null
    val (w0, h0) = unrotateDimensions(rawW, rawH, rotation)
    if (w0 <= 0 || h0 <= 0) return null
    val (x1, y1) = unrotatePoint(left.toFloat(), top.toFloat(), rawW.toFloat(), rawH.toFloat(), rotation)
    val (x2, y2) = unrotatePoint(right.toFloat(), bottom.toFloat(), rawW.toFloat(), rawH.toFloat(), rotation)
    val minX = minOf(x1, x2)
    val maxX = maxOf(x1, x2)
    val minY = minOf(y1, y2)
    val maxY = maxOf(y1, y2)
    val nx = (minX / w0.toFloat()).coerceIn(0f, 1f)
    val ny = (minY / h0.toFloat()).coerceIn(0f, 1f)
    val nw = ((maxX - minX) / w0.toFloat()).coerceIn(0f, 1f - nx)
    val nh = ((maxY - minY) / h0.toFloat()).coerceIn(0f, 1f - ny)
    if (nw <= 0f || nh <= 0f) return null
    return ScreenRect(x = nx, y = ny, w = nw, h = nh)
}

internal fun normalizeCorners(
    topLeftPx: Int,
    topRightPx: Int,
    bottomRightPx: Int,
    bottomLeftPx: Int,
    rawW: Int,
    rawH: Int,
    rotation: Int,
): ScreenCorners? {
    val (w0, _) = unrotateDimensions(rawW, rawH, rotation)
    if (w0 <= 0) return null
    val (tl, tr, br, bl) = when (rotation) {
        Surface.ROTATION_90 -> listOf(bottomLeftPx, topLeftPx, topRightPx, bottomRightPx)
        Surface.ROTATION_180 -> listOf(bottomRightPx, bottomLeftPx, topLeftPx, topRightPx)
        Surface.ROTATION_270 -> listOf(topRightPx, bottomRightPx, bottomLeftPx, topLeftPx)
        else -> listOf(topLeftPx, topRightPx, bottomRightPx, bottomLeftPx)
    }
    if (tl <= 0 && tr <= 0 && br <= 0 && bl <= 0) return null
    val w = w0.toFloat()
    return ScreenCorners(
        tl = (tl.coerceAtLeast(0) / w).coerceIn(0f, 0.5f),
        tr = (tr.coerceAtLeast(0) / w).coerceIn(0f, 0.5f),
        br = (br.coerceAtLeast(0) / w).coerceIn(0f, 0.5f),
        bl = (bl.coerceAtLeast(0) / w).coerceIn(0f, 0.5f),
    )
}

private fun sampleCutoutPath(path: Path, rawW: Int, rawH: Int, rotation: Int): String? {
    val pm = PathMeasure(path, false)
    val contours = mutableListOf<List<Pair<Float, Float>>>()
    val pos = FloatArray(2)
    do {
        val len = pm.length
        if (len > 0f && contours.size < 4) {
            val steps = 20
            val pts = ArrayList<Pair<Float, Float>>(steps)
            for (i in 0 until steps) {
                if (pm.getPosTan(len * i / steps.toFloat(), pos, null)) {
                    pts.add(pos[0] to pos[1])
                }
            }
            if (pts.size >= 3) contours.add(pts)
        }
    } while (pm.nextContour() && contours.size < 4)
    return buildNormalizedSvgPath(contours, rawW, rawH, rotation)
}

internal fun buildNormalizedSvgPath(
    contours: List<List<Pair<Float, Float>>>,
    rawW: Int,
    rawH: Int,
    rotation: Int,
): String? {
    val (w0, h0) = unrotateDimensions(rawW, rawH, rotation)
    if (w0 <= 0 || h0 <= 0 || contours.isEmpty()) return null
    val sb = StringBuilder()
    fun fmt(v: Float): String =
        String.format(Locale.US, "%.4f", v.coerceIn(0f, 1f))
            .trimEnd('0')
            .trimEnd('.')
            .ifEmpty { "0" }

    for (pts in contours) {
        if (pts.size < 3) continue
        pts.forEachIndexed { idx, (px, py) ->
            val (ux, uy) = unrotatePoint(px, py, rawW.toFloat(), rawH.toFloat(), rotation)
            val nx = fmt(ux / w0.toFloat())
            val ny = fmt(uy / h0.toFloat())
            if (idx == 0) {
                if (sb.isNotEmpty()) sb.append(' ')
                sb.append("M ").append(nx).append(' ').append(ny)
            } else {
                sb.append(" L ").append(nx).append(' ').append(ny)
            }
        }
        sb.append(" Z")
    }
    val out = sb.toString()
    return out.takeIf { it.isNotEmpty() && it.length <= 512 }
}

/** Reports the battery whenever it changes meaningfully. */
class BatteryMonitor(private val context: Context, private val onChange: (Battery) -> Unit) {
    private var last: Battery? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            if (level < 0 || scale <= 0) return
            val plugged = when (intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)) {
                BatteryManager.BATTERY_PLUGGED_AC -> "ac"
                BatteryManager.BATTERY_PLUGGED_USB -> "usb"
                BatteryManager.BATTERY_PLUGGED_WIRELESS -> "wireless"
                else -> null
            }
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            val fullIn: UShort? = if (charging && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                val bm = context.getSystemService(BatteryManager::class.java)
                val remainingMs = runCatching { bm?.computeChargeTimeRemaining() ?: -1L }.getOrDefault(-1L)
                if (remainingMs > 0L) {
                    ((remainingMs + 59_999L) / 60_000L).coerceIn(1L, 1440L).toUShort()
                } else {
                    null
                }
            } else {
                null
            }
            val battery = Battery((level * 100 / scale).coerceIn(0, 100).toUByte(), charging, plugged, fullIn)
            if (battery != last) {
                last = battery
                onChange(battery)
            }
        }
    }

    fun start() {
        // ACTION_BATTERY_CHANGED is sticky: the current state arrives at once.
        ContextCompat.registerReceiver(
            context, receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    fun stop() {
        runCatching { context.unregisterReceiver(receiver) }
    }
}

/**
 * Tells the core when connectivity changes. Android doesn't let native code
 * watch the network, so iroh needs this to re-check paths quickly.
 */
class NetworkMonitor(context: Context, private val onChange: () -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        /** Whether the default network was confirmed working, last we heard. */
        private var validated = false

        override fun onAvailable(network: Network) = onChange()
        override fun onLost(network: Network) {
            validated = false
            onChange()
        }

        // Right after boot a network is "available" before it has its
        // addresses and routes, and before it's known to work: both come
        // later, and the core has to hear of them too.
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = onChange()
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            val now = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            if (now != validated) {
                validated = now
                onChange()
            }
        }
    }

    fun start() {
        connectivity?.registerDefaultNetworkCallback(callback)
    }

    fun stop() {
        runCatching { connectivity?.unregisterNetworkCallback(callback) }
    }
}

/**
 * Listens for Material You wallpaper and configuration color changes on
 * Android 12+ (`API 31+`) without polling and without any permission.
 */
class AccentMonitor(private val context: Context, private val onChange: (UInt?) -> Unit) {
    private var lastSys: UInt? = null
    private var lastReported: UInt? = null

    private val colorsListener = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        WallpaperManager.OnColorsChangedListener { colors, which ->
            if ((which and WallpaperManager.FLAG_SYSTEM) == 0) return@OnColorsChangedListener
            val sys = runCatching { normalizeArgbSeed(context.getColor(android.R.color.system_accent1_500)) }.getOrNull()
            val wp = colors?.primaryColor?.toArgb()?.let(::normalizeArgbSeed)
            val next = when {
                sys != null && sys != lastSys -> {
                    lastSys = sys
                    sys
                }
                wp != null -> wp
                else -> sys
            }
            if (next != null && next != lastReported) {
                lastReported = next
                onChange(next)
            }
        }
    } else {
        null
    }

    private val componentCallbacks = object : ComponentCallbacks {
        override fun onConfigurationChanged(newConfig: Configuration) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
            val sys = runCatching { normalizeArgbSeed(context.getColor(android.R.color.system_accent1_500)) }.getOrNull()
            if (sys != null) lastSys = sys
            val next = materialYouAccent(context)
            if (next != null && next != lastReported) {
                lastReported = next
                onChange(next)
            }
        }

        @Deprecated("Deprecated in Java")
        override fun onLowMemory() = Unit
    }

    fun start() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        lastSys = runCatching { normalizeArgbSeed(context.getColor(android.R.color.system_accent1_500)) }.getOrNull()
        lastReported = materialYouAccent(context)
        colorsListener?.let { listener ->
            runCatching {
                context.getSystemService(WallpaperManager::class.java)
                    ?.addOnColorsChangedListener(listener, Handler(Looper.getMainLooper()))
            }
        }
        runCatching { context.registerComponentCallbacks(componentCallbacks) }
    }

    fun stop() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        colorsListener?.let { listener ->
            runCatching {
                context.getSystemService(WallpaperManager::class.java)
                    ?.removeOnColorsChangedListener(listener)
            }
        }
        runCatching { context.unregisterComponentCallbacks(componentCallbacks) }
    }
}

