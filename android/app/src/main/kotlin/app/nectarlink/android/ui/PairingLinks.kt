// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import androidx.core.content.edit
import java.security.MessageDigest

/** Remembers pairing links already used, so each is used at most once. */
class PairingLinks(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("pairing_links", Context.MODE_PRIVATE)

    /** True if [link] has already been consumed. */
    fun isUsed(link: String): Boolean {
        val digest = hash(link)
        val used = prefs.getString(KEY, "").orEmpty().split(',').filter { it.isNotEmpty() }
        return digest in used
    }

    /** True the first time a link is seen; false if it was used before. */
    fun consume(link: String): Boolean {
        // Store a hash: the link itself contains a (one-time) secret.
        val digest = hash(link)
        val used = prefs.getString(KEY, "").orEmpty().split(',').filter { it.isNotEmpty() }
        if (digest in used) return false
        prefs.edit { putString(KEY, (used + digest).takeLast(MAX_REMEMBERED).joinToString(",")) }
        return true
    }

    private fun hash(link: String): String =
        MessageDigest.getInstance("SHA-256").digest(link.toByteArray())
            .joinToString("") { "%02x".format(it) }

    private companion object {
        const val KEY = "used"
        const val MAX_REMEMBERED = 20
    }
}
