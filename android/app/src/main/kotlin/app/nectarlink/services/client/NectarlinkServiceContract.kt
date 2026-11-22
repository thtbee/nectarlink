// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services.client

object NectarlinkServiceContract {
    const val SERVICE_PACKAGE = "app.nectarlink.services"
    const val SERVICE_CLASS = "app.nectarlink.services.service.NectarlinkPrivilegedService"
    const val ACTION_BIND = "app.nectarlink.services.ACTION_BIND"
    const val ACTION_SETTINGS = "app.nectarlink.services.ACTION_SETTINGS"

    const val API_VERSION = 1
    const val MIN_SUPPORTED_API_VERSION = 1

    /** Package prefix reserved for NectarLink app variants; never eligible for remote uninstall. */
    const val OWN_PACKAGE_PREFIX = "app.nectarlink"

    /** Only activity that may be launched from the background on behalf of NectarLink. */
    const val ALLOWED_BACKGROUND_ACTIVITY = "app.nectarlink.android.service.PcStartActivity"

    const val MIN_DISPLAY_DIMENSION = 16
    const val MAX_DISPLAY_DIMENSION = 4096
    const val MIN_DISPLAY_DPI = 72
    const val MAX_DISPLAY_DPI = 1000
    const val MIN_BRIGHTNESS = 0
    const val MAX_BRIGHTNESS = 255
    const val MAX_PROTOCOL_LINE_CHARS = 256 * 1024

    val ALLOWED_DISPLAY_CODECS: Set<String> = setOf("h264", "h265", "av1")
    val ALLOWED_POINTER_EDGES: Set<String> = setOf("left", "right")

    private val PACKAGE_NAME_REGEX = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

    private const val BRIGHTNESS_PREFIX = "settings put system screen_brightness "
    private const val UNINSTALL_PREFIX = "pm uninstall "
    private const val AM_START_PREFIX = "am start -n "
    private val FIXED_SHELL_COMMANDS: Set<String> = setOf(
        "cmd statusbar expand-notifications",
        "svc wifi enable",
        "svc wifi disable",
        "svc bluetooth enable",
        "svc bluetooth disable",
        "hotspot on",
        "hotspot off",
        "cmd wifi start-softap",
        "cmd wifi stop-softap",
    )

    fun isSupportedApiVersion(version: Int): Boolean =
        version in MIN_SUPPORTED_API_VERSION..API_VERSION

    fun isValidPackageName(pkg: String): Boolean = PACKAGE_NAME_REGEX.matches(pkg)

    fun isNectarlinkPackage(pkg: String): Boolean =
        pkg == OWN_PACKAGE_PREFIX || pkg.startsWith("$OWN_PACKAGE_PREFIX.")

    fun isAllowedUninstallPackage(pkg: String, extraForbiddenPackage: String? = null): Boolean {
        if (!isValidPackageName(pkg)) return false
        if (isNectarlinkPackage(pkg)) return false
        if (extraForbiddenPackage != null && pkg == extraForbiddenPackage) return false
        return true
    }

    fun isAllowedProvisionPackage(
        pkg: String,
        allowedClientPackages: Set<String> = emptySet(),
    ): Boolean {
        if (!isValidPackageName(pkg)) return false
        return allowedClientPackages.isEmpty() || pkg in allowedClientPackages
    }

    fun isAllowedBackgroundLaunch(
        pkg: String,
        activityClass: String,
        allowedClientPackages: Set<String> = emptySet(),
    ): Boolean {
        if (!isValidPackageName(pkg)) return false
        if (allowedClientPackages.isNotEmpty() && pkg !in allowedClientPackages) return false
        return activityClass == ALLOWED_BACKGROUND_ACTIVITY
    }

    fun isAllowedBrightness(brightness: Int): Boolean = brightness in MIN_BRIGHTNESS..MAX_BRIGHTNESS

    fun isAllowedDisplayRequest(
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String,
    ): Boolean = width in MIN_DISPLAY_DIMENSION..MAX_DISPLAY_DIMENSION &&
        height in MIN_DISPLAY_DIMENSION..MAX_DISPLAY_DIMENSION &&
        dpi in MIN_DISPLAY_DPI..MAX_DISPLAY_DPI &&
        bitrate > 0 &&
        fps > 0 &&
        codec in ALLOWED_DISPLAY_CODECS

    fun isAllowedPointerStart(edge: String, y: Float): Boolean =
        edge in ALLOWED_POINTER_EDGES && y.isFinite()

    fun isAllowedTouch(displayId: Int, action: Int, x: Float, y: Float): Boolean =
        displayId >= -1 && action in TOUCH_DOWN..TOUCH_UP && x.isFinite() && y.isFinite()

    fun isAllowedScroll(displayId: Int, x: Float, y: Float, dx: Float, dy: Float): Boolean =
        displayId >= -1 && x.isFinite() && y.isFinite() && dx.isFinite() && dy.isFinite()

    fun isAllowedKey(displayId: Int, keyCode: Int): Boolean =
        displayId >= -1 && keyCode > 0

    fun isAllowedText(displayId: Int, text: String): Boolean =
        displayId >= -1 &&
            text.isNotEmpty() &&
            text.length <= MAX_PROTOCOL_LINE_CHARS &&
            '\r' !in text &&
            '\n' !in text

    fun isAllowedGamepad(
        displayId: Int,
        lx: Int,
        ly: Int,
        rx: Int,
        ry: Int,
        lt: Int,
        rt: Int,
    ): Boolean = displayId >= -1 &&
        lx in -32768..32767 &&
        ly in -32768..32767 &&
        rx in -32768..32767 &&
        ry in -32768..32767 &&
        lt in 0..255 &&
        rt in 0..255

    fun isAllowedShellCommand(
        cmd: String,
        allowedClientPackages: Set<String>,
        extraForbiddenUninstallPackage: String? = null,
    ): Boolean {
        if (cmd in FIXED_SHELL_COMMANDS) return true
        for (pkg in allowedClientPackages) {
            if (!isValidPackageName(pkg)) continue
            if (cmd == "appops set $pkg GET_USAGE_STATS allow") return true
            if (cmd == "cmd package compile -m speed-profile $pkg") return true
            if (cmd == "$AM_START_PREFIX$pkg/$ALLOWED_BACKGROUND_ACTIVITY") return true
        }
        if (cmd.startsWith(BRIGHTNESS_PREFIX)) {
            val v = cmd.removePrefix(BRIGHTNESS_PREFIX).toIntOrNull()
            return v != null && isAllowedBrightness(v)
        }
        if (cmd.startsWith(UNINSTALL_PREFIX)) {
            val pkg = cmd.removePrefix(UNINSTALL_PREFIX)
            return isAllowedUninstallPackage(pkg, extraForbiddenUninstallPackage)
        }
        return false
    }

    /**
     * Pinned SHA-256 signing certificate digests accepted for [SERVICE_PACKAGE] when verifying
     * the system service with [android.content.pm.PackageManager.CERT_INPUT_SHA256].
     */
    val TRUSTED_SERVICE_CERT_SHA256: Set<String> = setOf(
        "e055b7e6a622168da36e7b9f730369c8f140e5ba87df1e16f31fd754434eea0d",
        "c7c09a125ca5320b1670e710e798486de08b318127243b2790f3ff13511924f1",
    )

    const val TOUCH_DOWN = 0
    const val TOUCH_MOVE = 1
    const val TOUCH_UP = 2

    const val PACKET_KIND_CONFIG = 0
    const val PACKET_KIND_FRAME = 1
    const val PACKET_KIND_KEYFRAME = 2
    const val CODEC_ID_H264 = 1
    const val CODEC_ID_H265 = 2
    const val CODEC_ID_AV1 = 3

    const val CAP_INPUT_INJECTION = 1L shl 0
    const val CAP_SCREEN_MIRROR = 1L shl 1
    const val CAP_APP_VIRTUAL_DISPLAY = 1L shl 2
    const val CAP_DEVICE_AUDIO = 1L shl 3
    const val CAP_CALL_AUDIO = 1L shl 4
    const val CAP_CLIPBOARD_SYNC = 1L shl 5
    const val CAP_DISPLAY_POWER = 1L shl 6
    const val CAP_SYSTEM_TOGGLES = 1L shl 7
    const val CAP_UNINSTALL_PACKAGES = 1L shl 8
    const val CAP_POINTER_FLOW = 1L shl 9
    const val CAP_BACKGROUND_LAUNCH = 1L shl 10
}
