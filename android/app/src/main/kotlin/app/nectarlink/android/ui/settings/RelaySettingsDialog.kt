// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.ui.theme.nectarSwitchColors
import app.nectarlink.core.RelayEdit
import app.nectarlink.core.RelayEditResult
import app.nectarlink.core.RelayProbe
import app.nectarlink.core.RelayRegion
import app.nectarlink.core.RelayServer
import app.nectarlink.core.RelaySnapshot
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun RelaySettingsDialog(core: Core, awayMode: Boolean, onDismiss: () -> Unit) {
    var snapshot by remember { mutableStateOf<RelaySnapshot?>(null) }
    var probes by remember { mutableStateOf(emptyMap<String, RelayProbe>()) }
    var checking by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf(false) }
    var address by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<RelayEditResult?>(null) }
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current.lifecycle

    LaunchedEffect(core, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (true) {
                snapshot = core.relaySnapshot()
                delay(2_000)
            }
        }
    }

    fun edit(url: String, action: RelayEdit) {
        if (editing) return
        editing = true
        scope.launch {
            try {
                val result = core.editRelay(url, action)
                if (result == RelayEditResult.SAVED) {
                    error = null
                    if (action == RelayEdit.ADD) address = ""
                    if (action == RelayEdit.REMOVE) probes = probes - url
                    snapshot = core.relaySnapshot()
                } else {
                    error = result
                }
            } finally {
                editing = false
            }
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.relay_settings_title)) },
        confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.watch_close)) } },
        text = {
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                val current = snapshot
                if (current == null) {
                    Text(stringResource(R.string.relay_waiting))
                    return@Column
                }
                if (!awayMode) {
                    Text(stringResource(R.string.relay_away_hint))
                }
                Text(stringResource(R.string.relay_selection_hint))
                Text(stringResource(R.string.relay_home), style = MaterialTheme.typography.titleSmall)
                if (current.home.isEmpty()) Text(stringResource(R.string.relay_home_empty))
                current.home.forEach { relay ->
                    val label = when {
                        relay.connected -> R.string.relay_home_connected
                        relay.failed -> R.string.relay_home_failed
                        else -> R.string.relay_home_connecting
                    }
                    Text(stringResource(label, relay.url))
                }
                Text(stringResource(R.string.relay_device_routes), style = MaterialTheme.typography.titleSmall)
                if (current.routes.isEmpty()) Text(stringResource(R.string.relay_routes_empty))
                current.routes.forEach { route ->
                    Text(stringResource(R.string.relay_route, route.device, route.url))
                }
                HorizontalDivider()
                FilledTonalButton(
                    enabled = !checking,
                    onClick = {
                        checking = true
                        scope.launch {
                            try {
                                probes = core.checkRelays().associateBy { it.url }
                            } finally {
                                checking = false
                            }
                        }
                    },
                ) {
                    Text(stringResource(if (checking) R.string.relay_checking else R.string.relay_check_all))
                }
                OutlinedTextField(
                    modifier = Modifier.fillMaxWidth(),
                    value = address,
                    onValueChange = { address = it.take(1024) },
                    label = { Text(stringResource(R.string.relay_address)) },
                    placeholder = { Text("https://relay.example.com") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
                TextButton(
                    enabled = !editing && address.isNotBlank(),
                    onClick = { edit(address, RelayEdit.ADD) },
                ) {
                    Text(stringResource(R.string.relay_add))
                }
                error?.let {
                    Text(stringResource(editError(it)), color = MaterialTheme.colorScheme.error)
                }
                current.servers.groupBy { it.region }.forEach { (region, servers) ->
                    HorizontalDivider()
                    Text(stringResource(regionTitle(region)), style = MaterialTheme.typography.titleSmall)
                    servers.forEach { server ->
                        RelayServerRow(
                            server = server,
                            probe = probes[server.url],
                            checking = checking,
                            editing = editing,
                            onEdit = { edit(server.url, it) },
                        )
                    }
                }
            }
        },
    )
}

@Composable
private fun RelayServerRow(
    server: RelayServer,
    probe: RelayProbe?,
    checking: Boolean,
    editing: Boolean,
    onEdit: (RelayEdit) -> Unit,
) {
    val switchLabel = stringResource(R.string.relay_enable, server.url)
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(Modifier.weight(1f)) {
            if (server.provider.isNotEmpty()) {
                Text(server.provider, style = MaterialTheme.typography.labelLarge)
            }
            Text(server.url, style = MaterialTheme.typography.bodySmall)
            val rtt = probe?.rttMs
            val status = when {
                checking -> stringResource(R.string.relay_checking_short)
                probe == null -> stringResource(R.string.relay_not_checked)
                rtt != null -> stringResource(R.string.relay_rtt, rtt.toInt())
                probe.timedOut -> stringResource(R.string.relay_timed_out)
                else -> stringResource(R.string.relay_unreachable)
            }
            Text(
                status,
                style = MaterialTheme.typography.labelSmall,
                color = if (checking || probe == null) MaterialTheme.colorScheme.onSurfaceVariant
                        else if (rtt != null) MaterialTheme.colorScheme.primary
                        else MaterialTheme.colorScheme.error,
            )
            if (server.region == RelayRegion.CUSTOM) {
                TextButton(enabled = !editing, onClick = { onEdit(RelayEdit.REMOVE) }) {
                    Text(stringResource(R.string.timeline_action_remove))
                }
            }
        }
        Switch(
            checked = server.enabled,
            enabled = !editing,
            onCheckedChange = { onEdit(if (it) RelayEdit.ENABLE else RelayEdit.DISABLE) },
            colors = nectarSwitchColors(),
            modifier = Modifier.semantics { contentDescription = switchLabel },
        )
    }
}

@StringRes
private fun regionTitle(region: RelayRegion): Int = when (region) {
    RelayRegion.NORTH_AMERICA_EAST -> R.string.relay_region_na_east
    RelayRegion.NORTH_AMERICA_WEST -> R.string.relay_region_na_west
    RelayRegion.EUROPE -> R.string.relay_region_europe
    RelayRegion.ASIA_PACIFIC -> R.string.relay_region_asia_pacific
    RelayRegion.CUSTOM -> R.string.relay_region_custom
}

@StringRes
private fun editError(result: RelayEditResult): Int = when (result) {
    RelayEditResult.INVALID_ADDRESS -> R.string.relay_invalid_address
    RelayEditResult.DUPLICATE -> R.string.relay_duplicate
    RelayEditResult.LIMIT -> R.string.relay_limit
    else -> R.string.relay_save_failed
}
