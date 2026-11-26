// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.core.DeckState

/**
 * The PC's Wi-Fi and Bluetooth, each a switch (#636): shown when the PC
 * reports it. Turning Wi-Fi off asks first, since this phone may reach the
 * PC over it.
 */
@Composable
internal fun PcRadioSwitches(
    pcName: String,
    state: DeckState?,
    onSet: (wifi: Boolean?, bluetooth: Boolean?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val wifi = state?.wifi
    val bluetooth = state?.bluetooth
    if (wifi == null && bluetooth == null) return
    var askWifiOff by remember { mutableStateOf(false) }
    val on = stringResource(R.string.deck_on)
    val off = stringResource(R.string.deck_off)
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        wifi?.let { isOn ->
            FilterChip(
                selected = isOn,
                onClick = { if (isOn) askWifiOff = true else onSet(true, null) },
                label = { Text(stringResource(R.string.pc_radio_wifi)) },
                modifier = Modifier.semantics {
                    role = Role.Switch
                    stateDescription = if (isOn) on else off
                },
            )
        }
        bluetooth?.let { isOn ->
            FilterChip(
                selected = isOn,
                onClick = { onSet(null, !isOn) },
                label = { Text(stringResource(R.string.pc_radio_bluetooth)) },
                modifier = Modifier.semantics {
                    role = Role.Switch
                    stateDescription = if (isOn) on else off
                },
            )
        }
    }
    if (askWifiOff) {
        AlertDialog(
            onDismissRequest = { askWifiOff = false },
            title = { Text(stringResource(R.string.pc_radio_wifi_off_title, pcName)) },
            text = { Text(stringResource(R.string.pc_radio_wifi_off_text)) },
            confirmButton = {
                TextButton(onClick = {
                    askWifiOff = false
                    onSet(false, null)
                }) { Text(stringResource(R.string.pc_radio_wifi_off_confirm)) }
            },
            dismissButton = {
                TextButton(onClick = { askWifiOff = false }) { Text(stringResource(android.R.string.cancel)) }
            },
        )
    }
}
