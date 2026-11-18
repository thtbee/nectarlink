// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Ask before sharing the screen: PCs whose requests to see this phone's screen always
 * wait for a tap, even while Elevated could share it at once. Off for every PC unless
 * the user turns it on, so screen sharing works as it always has.
 */
object MirrorAsk {
    private const val PREFS = "mirror_ask"
    private const val KEY = "pcs"

    @Volatile private var loaded = false
    private val _pcs = MutableStateFlow(emptySet<String>())

    /** The PCs that ask each time. */
    fun pcs(context: Context): StateFlow<Set<String>> {
        load(context)
        return _pcs.asStateFlow()
    }

    fun isOn(context: Context, pcId: String): Boolean = pcId in pcs(context).value

    @Synchronized
    fun set(context: Context, pcId: String, on: Boolean) {
        load(context)
        val next = if (on) _pcs.value + pcId else _pcs.value - pcId
        prefs(context).edit { putStringSet(KEY, next) }
        _pcs.value = next
    }

    @Synchronized
    private fun load(context: Context) {
        if (loaded) return
        _pcs.value = prefs(context).getStringSet(KEY, emptySet()).orEmpty().toSet()
        loaded = true
    }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
