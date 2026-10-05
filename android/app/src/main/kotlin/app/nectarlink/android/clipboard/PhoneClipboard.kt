// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build

/**
 * The phone's clipboard. Android lets an app read it only while that app
 * is in front (and focused); writing works any time.
 */
object PhoneClipboard {
    sealed interface Read {
        data class Text(val text: String) : Read
        /** Marked sensitive by the app it came from (a password manager): never sent. */
        data object Private : Read
        data object Empty : Read
    }

    fun read(context: Context): Read {
        val clip = manager(context)?.primaryClip ?: return Read.Empty
        if (clip.description.isSensitive()) return Read.Private
        val text = (0 until clip.itemCount)
            .firstNotNullOfOrNull { clip.getItemAt(it).coerceToText(context)?.toString()?.takeIf(String::isNotBlank) }
        return text?.let { Read.Text(it) } ?: Read.Empty
    }

    /** Puts text a PC sent on the clipboard. */
    fun write(context: Context, text: String): Boolean =
        runCatching { manager(context)?.setPrimaryClip(ClipData.newPlainText("Nectarlink", text)) != null }
            .getOrDefault(false)

    private fun manager(context: Context) = context.getSystemService(ClipboardManager::class.java)

    private fun ClipDescription.isSensitive(): Boolean {
        val key = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ClipDescription.EXTRA_IS_SENSITIVE
        } else {
            "android.content.extra.IS_SENSITIVE"
        }
        return extras?.getBoolean(key) == true
    }
}
