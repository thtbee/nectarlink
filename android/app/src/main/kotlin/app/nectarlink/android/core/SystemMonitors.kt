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
import android.net.NetworkRequest
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.UserManager
import android.provider.Settings
import android.telephony.CellSignalStrengthCdma
import android.telephony.CellSignalStrengthGsm
import android.telephony.CellSignalStrengthLte
import android.telephony.CellSignalStrengthNr
import android.telephony.CellSignalStrengthTdscdma
import android.telephony.CellSignalStrengthWcdma
import android.telephony.PhoneStateListener
import android.telephony.SignalStrength
import android.telephony.TelephonyCallback
import android.telephony.TelephonyDisplayInfo
import android.telephony.TelephonyManager
import android.util.DisplayMetrics
import android.view.Display
import android.view.RoundedCorner
import android.view.Surface
import android.view.WindowManager
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.core.Battery
import app.nectarlink.core.DeviceInfo
import app.nectarlink.core.DeviceKind
import app.nectarlink.core.ScreenCorners
import app.nectarlink.core.ScreenRect
import app.nectarlink.core.ScreenShape
import java.util.Locale

internal fun normalizeArgbSeed(argb: Int): UInt = (argb.toUInt() or 0xFF00_0000u)

private val PRESET_SEEDS = mapOf(
    "honey" to 0xFFB7_791Fu,
    "lavender" to 0xFF7E_6AA8u,
    "ocean" to 0xFF3D_7DA8u,
    "sage" to 0xFF5B_8C5Au,
)

private fun parseSecureOverlaySeed(context: Context): UInt? = runCatching {
    val json = Settings.Secure.getString(context.contentResolver, "theme_customization_overlay_packages")
        ?.takeIf { it.isNotBlank() } ?: return null
    val obj = org.json.JSONObject(json)
    val hex = obj.optString("android.theme.customization.accent_color").ifBlank {
        obj.optString("android.theme.customization.system_palette")
    }.trim().removePrefix("#").removePrefix("0x").removePrefix("0X")
    if (hex.length >= 6) {
        val rgb = hex.takeLast(6).toUInt(16)
        (rgb or 0xFF00_0000u)
    } else {
        null
    }
}.getOrNull()

/**
 * Reads the phone's active theme seed color (`0xFFRRGGBB` ARGB):
 * 1. If the user selected "Media artwork color" or a preset seed ("honey",
 *    "lavender", "ocean", "sage") in the NectarLink Android app's Appearance
 *    settings, uses that seed so "My phone's colors" on the PC matches the
 *    phone app's chosen theme.
 * 2. When Dynamic Color is enabled on Android 12+ (`API 31+`), reads the
 *    live Material You seed from `Settings.Secure`, fresh configuration
 *    resources (`system_accent1_500`), and `WallpaperManager.getWallpaperColors`.
 */
fun materialYouAccent(
    context: Context,
    wallpaperColors: WallpaperColors? = null,
    activeMediaColor: UInt? = null,
): UInt? {
    val prefs = runCatching {
        context.applicationContext.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    }.getOrNull()
    if (prefs != null) {
        val seed = prefs.getString("seed", "honey") ?: "honey"
        val dynamicColor = prefs.getBoolean("dynamic_color", true)
        if (seed == "artwork" && activeMediaColor != null) {
            return normalizeArgbSeed(activeMediaColor.toInt())
        }
        if (!dynamicColor || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            return PRESET_SEEDS[seed] ?: PRESET_SEEDS["honey"]
        }
    }
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return PRESET_SEEDS["honey"]

    // Kept as is so the stock check below can see it (a neighboring shade of a stock
    // palette is also stock, and would pass for a real choice).
    val sys = runCatching {
        normalizeArgbSeed(context.getColor(android.R.color.system_accent1_500))
    }.getOrNull()
    val secure = parseSecureOverlaySeed(context)
    val wp = (wallpaperColors?.primaryColor?.toArgb()
        ?: runCatching {
            val wm = context.getSystemService(WallpaperManager::class.java)
            (wm?.getWallpaperColors(WallpaperManager.FLAG_SYSTEM)
                ?: wm?.getWallpaperColors(WallpaperManager.FLAG_LOCK))
                ?.primaryColor
                ?.toArgb()
        }.getOrNull())?.let(::normalizeArgbSeed)

    // On Samsung One UI, when the user hasn't enabled One UI's optional Color Palette
    // toggle, system_accent1_500 is a static stock slate-blue (0xFF697E9B). Prefer the
    // live system_accent1_500 or secure accent_color when configured, otherwise wallpaper.
    val isStockUnconfiguredSys = sys == null || sys == 0xFF69_7E9Bu || sys == 0xFF00_6782u
    return when {
        sys != null && !isStockUnconfiguredSys -> sys
        secure != null && secure != 0xFF69_7E9Bu && secure != 0xFF00_6782u -> secure
        wp != null -> wp
        secure != null -> secure
        else -> sys
    }
}

/**
 * The profile NectarLink runs in when it isn't the phone's main one: "Work" for a work
 * profile, "Second profile" for another user or profile (a cloned app, Samsung's Dual
 * Messenger). Null in the main profile. Each profile is a device of its own to the PC,
 * with the same phone name, so this tells them apart.
 */
internal fun profileLabel(context: Context): String? {
    val users = context.getSystemService(UserManager::class.java) ?: return null
    if (users.isSystemUser) return null
    val work = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && users.isManagedProfile
    return context.getString(if (work) R.string.profile_work else R.string.profile_other)
}

/** "Maker Model", without repeating the maker when the model already starts with it. */
internal fun modelName(manufacturer: String, model: String): String {
    val maker = manufacturer.trim().replaceFirstChar(Char::uppercase)
    val name = model.trim()
    return if (maker.isEmpty() || name.startsWith(maker, ignoreCase = true)) name else "$maker $name"
}

/** How this phone presents itself to PCs. */
/** The phone's name in Android's settings (its model when it has none). */
fun systemDeviceName(context: Context): String =
    Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
        ?.takeIf { it.isNotBlank() } ?: Build.MODEL

fun deviceInfo(context: Context, activeMediaColor: UInt? = null): DeviceInfo {
    // The name the user gave it in NectarLink, else Android's (#648).
    val phoneName = app.nectarlink.android.ui.Preferences.ownName(context).trim().takeIf { it.isNotEmpty() }
        ?: systemDeviceName(context)
    val name = profileLabel(context)?.let { context.getString(R.string.device_name_in_profile, phoneName, it) }
        ?: phoneName
    val tablet = context.resources.configuration.smallestScreenWidthDp >= 600
    return DeviceInfo(
        name = name,
        kind = if (tablet) DeviceKind.TABLET else DeviceKind.PHONE,
        os = "android",
        osVersion = Build.VERSION.RELEASE,
        model = modelName(Build.MANUFACTURER, Build.MODEL),
        accent = materialYouAccent(context, activeMediaColor = activeMediaColor),
        screen = measureScreenShape(context),
        installId = installId(context),
    )
}

/**
 * Identifies this phone across app reinstalls, so a PC replaces the old pairing instead of
 * listing the phone twice. ANDROID_ID is stable for this app's signing key on this phone;
 * only a salted hash of it leaves the phone (see `install_id_from` in the core).
 */
private fun installId(context: Context): String? = runCatching {
    Settings.Secure.getString(context.contentResolver, Settings.Secure.ANDROID_ID)
        ?.let { app.nectarlink.core.installIdFrom(it) }
}.getOrNull()

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
            val battery = Battery(
                level = (level * 100 / scale).coerceIn(0, 100).toUByte(),
                charging = charging,
                plugged = plugged,
                fullIn = fullIn,
                cellBars = null,
                cellType = null,
                wifiBars = null,
            )
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
        last = null
    }
}

/** Cellular and Wi-Fi signal snapshot reported alongside battery state. */
data class SignalSnapshot(
    val cellBars: UByte? = null,
    val cellType: String? = null,
    val wifiBars: UByte? = null,
)

/**
 * Maps a Wi-Fi RSSI in dBm to `0..4` signal bars, or `null` if the RSSI is
 * invalid (`<= -127` or `>= 0`).
 */
internal fun rssiToWifiBars(rssiDbm: Int): UByte? {
    if (rssiDbm <= -127 || rssiDbm >= 0) return null
    return when {
        rssiDbm >= -55 -> 4u
        rssiDbm >= -67 -> 3u
        rssiDbm >= -78 -> 2u
        rssiDbm >= -88 -> 1u
        else -> 0u
    }
}

/**
 * Maps Android's [TelephonyManager] network type and [TelephonyDisplayInfo]
 * override network type to a short generation label (`"5G"`, `"LTE"`, `"3G"`, `"2G"`).
 */
@Suppress("DEPRECATION")
internal fun networkTypeLabel(networkType: Int, overrideNetworkType: Int = 0): String? {
    when (overrideNetworkType) {
        TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_NR_NSA,
        4, // OVERRIDE_NETWORK_TYPE_NR_NSA_MMWAVE
        TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_NR_ADVANCED,
        -> return "5G"
        TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_LTE_CA,
        TelephonyDisplayInfo.OVERRIDE_NETWORK_TYPE_LTE_ADVANCED_PRO,
        -> return "LTE"
    }
    return when (networkType) {
        TelephonyManager.NETWORK_TYPE_NR -> "5G"
        TelephonyManager.NETWORK_TYPE_LTE,
        TelephonyManager.NETWORK_TYPE_IWLAN,
        19, // NETWORK_TYPE_LTE_CA
        -> "LTE"
        TelephonyManager.NETWORK_TYPE_HSPAP,
        TelephonyManager.NETWORK_TYPE_HSPA,
        TelephonyManager.NETWORK_TYPE_HSDPA,
        TelephonyManager.NETWORK_TYPE_HSUPA,
        TelephonyManager.NETWORK_TYPE_UMTS,
        TelephonyManager.NETWORK_TYPE_EVDO_0,
        TelephonyManager.NETWORK_TYPE_EVDO_A,
        TelephonyManager.NETWORK_TYPE_EVDO_B,
        TelephonyManager.NETWORK_TYPE_EHRPD,
        TelephonyManager.NETWORK_TYPE_TD_SCDMA,
        -> "3G"
        TelephonyManager.NETWORK_TYPE_EDGE,
        TelephonyManager.NETWORK_TYPE_GPRS,
        TelephonyManager.NETWORK_TYPE_CDMA,
        TelephonyManager.NETWORK_TYPE_1xRTT,
        TelephonyManager.NETWORK_TYPE_IDEN,
        TelephonyManager.NETWORK_TYPE_GSM,
        -> "2G"
        else -> null
    }
}

/**
 * Event-driven cellular and Wi-Fi signal monitor. Uses Android system callbacks
 * without polling and without requiring any new permissions.
 */
class SignalMonitor(private val context: Context, private val onChange: (SignalSnapshot) -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val wifiManager = context.applicationContext.getSystemService(WifiManager::class.java)
    private val telephony = context.getSystemService(TelephonyManager::class.java)

    private var current = SignalSnapshot()
    private var cellBars: UByte? = null
    private var signalCellType: String? = null
    private var overrideNetworkType: Int = 0
    private var wifiBars: UByte? = null

    private var signalCallback: Any? = null
    private var displayInfoCallback: Any? = null
    private var phoneStateListener: Any? = null

    private val wifiNetworkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val caps = runCatching { connectivity?.getNetworkCapabilities(network) }.getOrNull()
            wifiBars = extractWifiBars(capabilities = caps)
            emitIfChanged()
        }

        override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) {
            wifiBars = extractWifiBars(capabilities = networkCapabilities)
            emitIfChanged()
        }

        override fun onLost(network: Network) {
            wifiBars = null
            emitIfChanged()
        }
    }

    private val wifiReceiver = object : BroadcastReceiver() {
        @Suppress("DEPRECATION")
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                WifiManager.RSSI_CHANGED_ACTION -> {
                    val rssi = intent.getIntExtra(WifiManager.EXTRA_NEW_RSSI, -127)
                    wifiBars = rssiToWifiBars(rssi) ?: extractWifiBars()
                    emitIfChanged()
                }
                WifiManager.NETWORK_STATE_CHANGED_ACTION -> {
                    wifiBars = extractWifiBars()
                    emitIfChanged()
                }
            }
        }
    }

    private fun extractWifiBars(capabilities: NetworkCapabilities? = null): UByte? {
        val caps = capabilities ?: runCatching {
            connectivity?.activeNetwork?.let { connectivity.getNetworkCapabilities(it) }
        }.getOrNull()
        if (caps != null) {
            if (!caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return null
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val transportRssi = (caps.transportInfo as? WifiInfo)?.rssi
                if (transportRssi != null) {
                    rssiToWifiBars(transportRssi)?.let { return it }
                }
                rssiToWifiBars(caps.signalStrength)?.let { return it }
            }
            @Suppress("DEPRECATION")
            val info = runCatching { wifiManager?.connectionInfo }.getOrNull()
            if (info != null && info.networkId != -1) {
                rssiToWifiBars(info.rssi)?.let { return it }
            }
            return 3u
        }
        return null
    }

    private fun updateFromSignalStrength(signalStrength: SignalStrength?) {
        if (signalStrength == null) return
        val level = runCatching { signalStrength.level }.getOrNull()
        cellBars = level?.coerceIn(0, 4)?.toUByte()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val strengths = runCatching { signalStrength.cellSignalStrengths }.getOrNull().orEmpty()
            val inferred = when {
                strengths.any { it is CellSignalStrengthNr } -> "5G"
                strengths.any { it is CellSignalStrengthLte } -> "LTE"
                strengths.any { it is CellSignalStrengthWcdma || it is CellSignalStrengthTdscdma } -> "3G"
                strengths.any { it is CellSignalStrengthGsm || it is CellSignalStrengthCdma } -> "2G"
                else -> null
            }
            if (inferred != null) {
                signalCellType = inferred
            }
        }
    }

    private fun resolveCellType(): String? {
        @Suppress("MissingPermission")
        val dataNetType = runCatching { telephony?.dataNetworkType ?: TelephonyManager.NETWORK_TYPE_UNKNOWN }
            .getOrDefault(TelephonyManager.NETWORK_TYPE_UNKNOWN)
        val fromManager = networkTypeLabel(dataNetType, overrideNetworkType)
        return when {
            fromManager == "5G" || signalCellType == "5G" -> "5G"
            fromManager != null -> fromManager
            else -> signalCellType
        }
    }

    private var running = false

    private fun emitIfChanged() {
        val next = SignalSnapshot(
            cellBars = cellBars,
            cellType = resolveCellType(),
            wifiBars = wifiBars,
        )
        if (next != current) {
            current = next
            onChange(next)
        }
    }

    fun start() {
        if (running) return
        running = true
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            runCatching { updateFromSignalStrength(telephony?.signalStrength) }
        }
        wifiBars = extractWifiBars()

        runCatching {
            val request = NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .build()
            connectivity?.registerNetworkCallback(request, wifiNetworkCallback)
        }
        @Suppress("DEPRECATION")
        runCatching {
            val filter = IntentFilter().apply {
                addAction(WifiManager.RSSI_CHANGED_ACTION)
                addAction(WifiManager.NETWORK_STATE_CHANGED_ACTION)
            }
            ContextCompat.registerReceiver(context, wifiReceiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val sigCb = object : TelephonyCallback(), TelephonyCallback.SignalStrengthsListener {
                override fun onSignalStrengthsChanged(signalStrength: SignalStrength) {
                    updateFromSignalStrength(signalStrength)
                    emitIfChanged()
                }
            }
            signalCallback = sigCb
            runCatching { telephony?.registerTelephonyCallback(context.mainExecutor, sigCb) }

            val dispCb = object : TelephonyCallback(), TelephonyCallback.DisplayInfoListener {
                override fun onDisplayInfoChanged(telephonyDisplayInfo: TelephonyDisplayInfo) {
                    overrideNetworkType = telephonyDisplayInfo.overrideNetworkType
                    emitIfChanged()
                }
            }
            displayInfoCallback = dispCb
            runCatching { telephony?.registerTelephonyCallback(context.mainExecutor, dispCb) }
        } else {
            @Suppress("DEPRECATION")
            val listener = object : PhoneStateListener() {
                @Deprecated("Deprecated in Java")
                override fun onSignalStrengthsChanged(signalStrength: SignalStrength?) {
                    updateFromSignalStrength(signalStrength)
                    emitIfChanged()
                }
            }
            phoneStateListener = listener
            @Suppress("DEPRECATION")
            runCatching {
                telephony?.listen(listener, PhoneStateListener.LISTEN_SIGNAL_STRENGTHS)
            }
        }

        emitIfChanged()
    }

    fun stop() {
        if (!running) return
        running = false
        runCatching { connectivity?.unregisterNetworkCallback(wifiNetworkCallback) }
        runCatching { context.unregisterReceiver(wifiReceiver) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (signalCallback as? TelephonyCallback)?.let { cb ->
                runCatching { telephony?.unregisterTelephonyCallback(cb) }
            }
            (displayInfoCallback as? TelephonyCallback)?.let { cb ->
                runCatching { telephony?.unregisterTelephonyCallback(cb) }
            }
            signalCallback = null
            displayInfoCallback = null
        } else {
            @Suppress("DEPRECATION")
            (phoneStateListener as? PhoneStateListener)?.let { listener ->
                runCatching { telephony?.listen(listener, PhoneStateListener.LISTEN_NONE) }
            }
            phoneStateListener = null
        }
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

    /**
     * Whether the phone is on a network a PC could share with it: Wi-Fi,
     * Ethernet, a USB link or a VPN such as Tailscale (#397). Mobile data
     * alone isn't, unless Connect over mobile data is on.
     */
    @Suppress("DEPRECATION")
    fun hasLocalNetwork(): Boolean {
        val manager = connectivity ?: return true
        val transports = buildList {
            add(NetworkCapabilities.TRANSPORT_WIFI)
            add(NetworkCapabilities.TRANSPORT_ETHERNET)
            add(NetworkCapabilities.TRANSPORT_VPN)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) add(NetworkCapabilities.TRANSPORT_USB)
        }
        return runCatching {
            manager.allNetworks.any { network ->
                val caps = manager.getNetworkCapabilities(network) ?: return@any false
                transports.any(caps::hasTransport)
            }
        }.getOrDefault(true)
    }
}

/**
 * Listens for NectarLink appearance preference changes, media artwork color
 * changes, and Material You wallpaper/configuration color changes without polling.
 */
class AccentMonitor(private val context: Context, private val onChange: (UInt?) -> Unit) {
    private var lastReported: UInt? = null
    private var activeMediaColor: UInt? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private val prefs = runCatching {
        context.applicationContext.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    }.getOrNull()

    private val prefsListener = android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, _ ->
        emitIfChanged()
    }

    private val secureObserver = object : android.database.ContentObserver(mainHandler) {
        override fun onChange(selfChange: Boolean) {
            emitIfChanged()
        }
    }

    private val colorsListener = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        WallpaperManager.OnColorsChangedListener { colors, which ->
            if ((which and (WallpaperManager.FLAG_SYSTEM or WallpaperManager.FLAG_LOCK)) == 0) {
                return@OnColorsChangedListener
            }
            emitIfChanged(colors)
        }
    } else {
        null
    }

    private val componentCallbacks = object : ComponentCallbacks {
        override fun onConfigurationChanged(newConfig: Configuration) {
            emitIfChanged()
        }

        @Deprecated("Deprecated in Java")
        override fun onLowMemory() = Unit
    }

    @Synchronized
    private fun emitIfChanged(wallpaperColors: WallpaperColors? = null, force: Boolean = false) {
        val next = materialYouAccent(
            context = context,
            wallpaperColors = wallpaperColors,
            activeMediaColor = activeMediaColor,
        )
        if (next != null && (force || next != lastReported)) {
            lastReported = next
            onChange(next)
        }
    }

    @Synchronized
    fun updateMediaColor(color: UInt?) {
        if (activeMediaColor == color) return
        activeMediaColor = color
        emitIfChanged()
    }

    @Synchronized
    fun syncNow() {
        emitIfChanged(force = true)
    }

    fun start() {
        prefs?.registerOnSharedPreferenceChangeListener(prefsListener)
        runCatching {
            val uri = Settings.Secure.getUriFor("theme_customization_overlay_packages")
            if (uri != null) {
                context.contentResolver.registerContentObserver(uri, false, secureObserver)
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            colorsListener?.let { listener ->
                runCatching {
                    context.getSystemService(WallpaperManager::class.java)
                        ?.addOnColorsChangedListener(listener, mainHandler)
                }
            }
        }
        runCatching { context.registerComponentCallbacks(componentCallbacks) }
        emitIfChanged(force = true)
    }

    fun stop() {
        prefs?.unregisterOnSharedPreferenceChangeListener(prefsListener)
        runCatching { context.contentResolver.unregisterContentObserver(secureObserver) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            colorsListener?.let { listener ->
                runCatching {
                    context.getSystemService(WallpaperManager::class.java)
                        ?.removeOnColorsChangedListener(listener)
                }
            }
        }
        runCatching { context.unregisterComponentCallbacks(componentCallbacks) }
    }
}

