// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context

/**
 * Names this phone shows for its paired PCs, by device ID. A PC's own name is
 * used until the user gives it another one here; setting a blank name clears
 * the override. This is what the phone displays, not a rename done on the PC.
 */
object PcDisplayName {
    private const val PREFS = "pc_display_names"
    private const val PREFIX = "alias:"

    fun all(context: Context): Map<String, String> =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .all
            .mapNotNull { (key, value) ->
                val id = key.removePrefix(PREFIX)
                val name = (value as? String)?.trim().orEmpty()
                if (key.startsWith(PREFIX) && id.isNotEmpty() && name.isNotEmpty()) id to name else null
            }
            .toMap()

    fun set(context: Context, pcId: String, name: String) {
        if (pcId.isBlank()) return
        val clean = name.trim()
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().apply {
            if (clean.isEmpty()) remove(PREFIX + pcId) else putString(PREFIX + pcId, clean.take(64))
        }.apply()
    }
}
