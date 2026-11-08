// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.view.KeyEvent
import java.util.Base64

/** One helper command, so a Unicode paste cannot be separated from its text. */
internal object TextCommand {
    fun make(text: String, display: Int?): String? {
        if (text.isEmpty()) return null
        val target = display?.let { "@$it " }.orEmpty()
        val command = when {
            text == "\n" || text == "\r" || text == "\r\n" -> "K ${KeyEvent.KEYCODE_ENTER}"
            text.length == 1 && text[0].code in 32..126 -> "T $text"
            else -> "I ${Base64.getEncoder().encodeToString(text.toByteArray(Charsets.UTF_8))}"
        }
        return target + command
    }
}
