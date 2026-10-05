// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import androidx.core.content.edit
import app.nectarlink.android.ui.theme.Appearance
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The app's look, saved in shared preferences and observable. */
class Preferences(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    private val _appearance = MutableStateFlow(load())
    val appearance: StateFlow<Appearance> = _appearance.asStateFlow()

    private fun load(): Appearance {
        val defaults = Appearance()
        return Appearance(
            theme = prefs.getString(THEME, defaults.theme) ?: defaults.theme,
            mode = prefs.getString(MODE, defaults.mode) ?: defaults.mode,
            seed = prefs.getString(SEED, defaults.seed) ?: defaults.seed,
            dynamicColor = prefs.getBoolean(DYNAMIC, defaults.dynamicColor),
        )
    }

    fun update(change: (Appearance) -> Appearance) {
        val next = change(_appearance.value)
        prefs.edit {
            putString(THEME, next.theme)
            putString(MODE, next.mode)
            putString(SEED, next.seed)
            putBoolean(DYNAMIC, next.dynamicColor)
        }
        _appearance.value = next
    }

    private companion object {
        const val THEME = "theme"
        const val MODE = "mode"
        const val SEED = "seed"
        const val DYNAMIC = "dynamic_color"
    }
}
