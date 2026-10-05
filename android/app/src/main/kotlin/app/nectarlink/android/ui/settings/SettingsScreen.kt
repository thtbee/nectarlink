// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.os.Build
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.LocalAppFonts
import app.nectarlink.android.ui.theme.Tokens

@Composable
fun SettingsScreen(
    state: CoreState,
    appearance: Appearance,
    onAppearance: ((Appearance) -> Appearance) -> Unit,
    onUnpair: (String) -> Unit,
    onPairNew: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirmUnpair by remember { mutableStateOf<Device?>(null) }

    Column(
        modifier.verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            stringResource(R.string.settings_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
        )

        Section(stringResource(R.string.settings_appearance)) {
            Label(stringResource(R.string.settings_theme))
            Choice(
                listOf("bloom" to stringResource(R.string.theme_bloom), "graphite" to stringResource(R.string.theme_graphite)),
                appearance.theme,
            ) { value -> onAppearance { it.copy(theme = value) } }
            Label(stringResource(R.string.settings_mode))
            Choice(
                listOf(
                    "system" to stringResource(R.string.mode_system),
                    "light" to stringResource(R.string.mode_light),
                    "dark" to stringResource(R.string.mode_dark),
                ),
                appearance.mode,
            ) { value -> onAppearance { it.copy(mode = value) } }
            if (appearance.theme == "bloom") {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f)) {
                            Text(stringResource(R.string.settings_dynamic_color), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_dynamic_color_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = appearance.dynamicColor,
                            onCheckedChange = { on -> onAppearance { it.copy(dynamicColor = on) } },
                        )
                    }
                }
                if (!appearance.dynamicColor || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Tokens.bloomSeeds.forEach { (name, seed) ->
                            val selected = appearance.seed == name
                            Surface(
                                shape = CircleShape,
                                color = seed.color,
                                modifier = Modifier
                                    .size(32.dp)
                                    .then(if (selected) Modifier.border(2.dp, MaterialTheme.colorScheme.onSurface, CircleShape) else Modifier)
                                    .clickable { onAppearance { it.copy(seed = name) } }
                                    .semantics { contentDescription = name },
                            ) {}
                        }
                    }
                }
            }
        }

        Section(stringResource(R.string.settings_devices)) {
            state.devices.forEach { device ->
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Column(Modifier.weight(1f)) {
                        Text(device.name, style = MaterialTheme.typography.titleSmall)
                        Text(
                            stringResource(if (device.online) R.string.connected else R.string.not_connected),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    OutlinedButton(onClick = { confirmUnpair = device }) { Text(stringResource(R.string.action_unpair)) }
                }
            }
            FilledTonalButton(onClick = onPairNew) { Text(stringResource(R.string.action_pair_new)) }
        }

        Section(stringResource(R.string.settings_about)) {
            Text(stringResource(R.string.about_version, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodyMedium)
            (state.status as? CoreStatus.Ready)?.let {
                Text(
                    it.deviceId,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = LocalAppFonts.current.mono,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }

    confirmUnpair?.let { device ->
        AlertDialog(
            onDismissRequest = { confirmUnpair = null },
            title = { Text(stringResource(R.string.unpair_title, device.name)) },
            text = { Text(stringResource(R.string.unpair_text)) },
            confirmButton = {
                TextButton(onClick = { onUnpair(device.id); confirmUnpair = null }) {
                    Text(stringResource(R.string.action_unpair))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmUnpair = null }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Text(
        title,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 8.dp),
    )
    Surface(shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) { content() }
    }
}

@Composable
private fun Label(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall)
}

@Composable
private fun Choice(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { index, (value, label) ->
            SegmentedButton(
                selected = value == selected,
                onClick = { onSelect(value) },
                shape = SegmentedButtonDefaults.itemShape(index, options.size),
            ) { Text(label) }
        }
    }
}
