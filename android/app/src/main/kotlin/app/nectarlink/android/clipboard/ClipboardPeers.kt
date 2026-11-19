// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Per device, whether this phone's clipboard goes to it by itself and whether its
 * clipboard is taken here (#453). Both are on for every device unless the user turns
 * one off, so the clipboard works as it always has. Only the exceptions are kept.
 *
 * Accepting is enforced by the core (the `clipboard_receive` device toggle); this keeps
 * the same choice for showing it. Sharing only affects what's sent by itself: sending
 * the clipboard by hand still goes where the user picks.
 */
object ClipboardPeers {
    private const val PREFS = "clipboard_peers"
    private const val NOT_SHARED = "not_shared"
    private const val NOT_ACCEPTED = "not_accepted"

    @Volatile private var loaded = false
    private val _notShared = MutableStateFlow(emptySet<String>())
    private val _notAccepted = MutableStateFlow(emptySet<String>())

    /** Devices this phone's copies don't go to by themselves. */
    fun notShared(context: Context): StateFlow<Set<String>> {
        load(context)
        return _notShared.asStateFlow()
    }

    /** Devices whose clipboard this phone doesn't take. */
    fun notAccepted(context: Context): StateFlow<Set<String>> {
        load(context)
        return _notAccepted.asStateFlow()
    }

    fun shares(context: Context, deviceId: String): Boolean = deviceId !in notShared(context).value

    @Synchronized
    fun setShared(context: Context, deviceId: String, on: Boolean) {
        load(context)
        val next = if (on) _notShared.value - deviceId else _notShared.value + deviceId
        prefs(context).edit { putStringSet(NOT_SHARED, next) }
        _notShared.value = next
    }

    @Synchronized
    fun setAccepted(context: Context, deviceId: String, on: Boolean) {
        load(context)
        val next = if (on) _notAccepted.value - deviceId else _notAccepted.value + deviceId
        prefs(context).edit { putStringSet(NOT_ACCEPTED, next) }
        _notAccepted.value = next
    }

    /** Forgets an unpaired device. */
    @Synchronized
    fun forget(context: Context, deviceId: String) {
        load(context)
        if (deviceId in _notShared.value) setShared(context, deviceId, true)
        if (deviceId in _notAccepted.value) setAccepted(context, deviceId, true)
    }

    @Synchronized
    private fun load(context: Context) {
        if (loaded) return
        val prefs = prefs(context)
        _notShared.value = prefs.getStringSet(NOT_SHARED, emptySet()).orEmpty().toSet()
        _notAccepted.value = prefs.getStringSet(NOT_ACCEPTED, emptySet()).orEmpty().toSet()
        loaded = true
    }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
