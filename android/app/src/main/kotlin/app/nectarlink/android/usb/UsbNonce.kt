// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.usb

/**
 * The one-time nonce a PC sends through adb before it opens a USB tunnel to a phone
 * it isn't paired with, and repeats in the tunnel's hello (usb.rs). Only the shell can
 * send the wake broadcast that carries it, so a tunnel whose hello has it was opened by
 * adb and not by another app on this phone. The core keeps it for two minutes and lets
 * that one tunnel pair without pairing mode.
 */
internal object UsbNonce {
    /** The wake broadcast's extra holding it. */
    const val EXTRA = "nonce"

    private const val LENGTH = 32

    /** [raw] as a nonce (32 hex digits, lowercased), or null when it isn't one. */
    fun parse(raw: String?): String? {
        val nonce = raw?.trim()?.lowercase() ?: return null
        return nonce.takeIf { it.length == LENGTH && it.all { c -> c in '0'..'9' || c in 'a'..'f' } }
    }
}
