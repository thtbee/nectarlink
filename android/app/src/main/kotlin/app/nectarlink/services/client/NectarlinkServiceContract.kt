// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services.client

object NectarlinkServiceContract {
    const val SERVICE_PACKAGE = "app.nectarlink.services"
    const val SERVICE_CLASS = "app.nectarlink.services.service.NectarlinkPrivilegedService"
    const val ACTION_BIND = "app.nectarlink.services.ACTION_BIND"
    const val ACTION_SETTINGS = "app.nectarlink.services.ACTION_SETTINGS"

    const val API_VERSION = 1

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
