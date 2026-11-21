// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.desk

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.ui.settings.Choice

/** How Desk clock looks (D380): its background, its clock's font and what it shows. */
data class DeskStyle(
    val background: Background = Background.Black,
    val font: Font = Font.Mono,
    val seconds: Boolean = true,
    val date: Boolean = true,
    val media: Boolean = true,
    val battery: Boolean = false,
) {
    enum class Background { Black, Graphite, Accent }

    enum class Font { Mono, Sans, Serif }

    val fontFamily: FontFamily
        get() = when (font) {
            Font.Mono -> FontFamily.Monospace
            Font.Sans -> FontFamily.SansSerif
            Font.Serif -> FontFamily.Serif
        }

    /** The background, from [accent] for [Background.Accent]: always dark, for a clock left on at night. */
    fun brush(accent: Color): Brush = when (background) {
        Background.Black -> Brush.verticalGradient(listOf(Color.Black, Color.Black))
        Background.Graphite -> Brush.verticalGradient(listOf(Color(0xFF1C1F26), Color(0xFF0B0C10)))
        Background.Accent -> Brush.verticalGradient(
            listOf(accent.copy(alpha = 0.32f).compositeOver(Color(0xFF0B0C10)), Color(0xFF050608)),
        )
    }

    fun save(context: Context) {
        prefs(context).edit()
            .putString(BACKGROUND, background.name)
            .putString(FONT, font.name)
            .putBoolean(SECONDS, seconds)
            .putBoolean(DATE, date)
            .putBoolean(MEDIA, media)
            .putBoolean(BATTERY, battery)
            .apply()
    }

    companion object {
        private const val BACKGROUND = "background"
        private const val FONT = "font"
        private const val SECONDS = "seconds"
        private const val DATE = "date"
        private const val MEDIA = "media"
        private const val BATTERY = "battery"

        private fun prefs(context: Context) =
            context.applicationContext.getSharedPreferences("desk", Context.MODE_PRIVATE)

        fun load(context: Context): DeskStyle {
            val p = prefs(context)
            val default = DeskStyle()
            return DeskStyle(
                background = p.getString(BACKGROUND, null)
                    ?.let { name -> Background.entries.firstOrNull { it.name == name } } ?: default.background,
                font = p.getString(FONT, null)?.let { name -> Font.entries.firstOrNull { it.name == name } }
                    ?: default.font,
                seconds = p.getBoolean(SECONDS, default.seconds),
                date = p.getBoolean(DATE, default.date),
                media = p.getBoolean(MEDIA, default.media),
                battery = p.getBoolean(BATTERY, default.battery),
            )
        }
    }
}

/** Changes how Desk clock looks; each change shows at once behind the sheet. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun DeskStyleSheet(style: DeskStyle, onChange: (DeskStyle) -> Unit, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(stringResource(R.string.desk_customize), style = MaterialTheme.typography.titleLarge)

            Text(stringResource(R.string.desk_background), style = MaterialTheme.typography.titleSmall)
            Choice(
                options = listOf(
                    DeskStyle.Background.Black.name to stringResource(R.string.desk_background_black),
                    DeskStyle.Background.Graphite.name to stringResource(R.string.desk_background_graphite),
                    DeskStyle.Background.Accent.name to stringResource(R.string.desk_background_accent),
                ),
                selected = style.background.name,
                onSelect = { name -> onChange(style.copy(background = DeskStyle.Background.valueOf(name))) },
            )

            Text(stringResource(R.string.desk_font), style = MaterialTheme.typography.titleSmall)
            Choice(
                options = listOf(
                    DeskStyle.Font.Mono.name to stringResource(R.string.desk_font_mono),
                    DeskStyle.Font.Sans.name to stringResource(R.string.desk_font_sans),
                    DeskStyle.Font.Serif.name to stringResource(R.string.desk_font_serif),
                ),
                selected = style.font.name,
                onSelect = { name -> onChange(style.copy(font = DeskStyle.Font.valueOf(name))) },
            )

            Text(stringResource(R.string.desk_show), style = MaterialTheme.typography.titleSmall)
            ShowRow(stringResource(R.string.desk_show_seconds), style.seconds) { onChange(style.copy(seconds = it)) }
            ShowRow(stringResource(R.string.desk_show_date), style.date) { onChange(style.copy(date = it)) }
            ShowRow(stringResource(R.string.desk_show_media), style.media) { onChange(style.copy(media = it)) }
            ShowRow(stringResource(R.string.desk_show_battery), style.battery) { onChange(style.copy(battery = it)) }
        }
    }
}

@Composable
private fun ShowRow(label: String, checked: Boolean, onChecked: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(
            checked = checked,
            onCheckedChange = onChecked,
            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
            modifier = Modifier.semantics { contentDescription = label },
        )
    }
}
