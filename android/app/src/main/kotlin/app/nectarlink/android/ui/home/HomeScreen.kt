// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.text.format.DateUtils
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.core.Link

/**
 * Home: each paired PC with its connection and quick actions. When a PC
 * makes this phone ring, a banner to stop it comes first.
 */
@Composable
fun HomeScreen(
    state: CoreState,
    onRing: (id: String, on: Boolean) -> Unit,
    onStopRinging: () -> Unit,
    onPairNew: () -> Unit,
    modifier: Modifier = Modifier,
) {
    LazyColumn(
        modifier = modifier,
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            Row(Modifier.padding(top = 8.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    stringResource(R.string.home_title),
                    style = MaterialTheme.typography.headlineMedium,
                    modifier = Modifier.weight(1f),
                )
                FilledTonalIconButton(onClick = onPairNew) {
                    Icon(painterResource(R.drawable.ic_qr), contentDescription = stringResource(R.string.action_pair_new))
                }
            }
        }
        state.ringingFrom?.let { from ->
            item { RingingBanner(from, onStopRinging) }
        }
        items(state.devices, key = { it.id }) { device -> PcCard(device, onRing) }
    }
}

@Composable
private fun RingingBanner(from: String, onStop: () -> Unit) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.primary) {
        Row(Modifier.fillMaxWidth().padding(20.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (from.isEmpty()) stringResource(R.string.ringing_title_unknown) else stringResource(R.string.ringing_title, from),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier.weight(1f),
            )
            FilledTonalButton(onClick = onStop) { Text(stringResource(R.string.action_stop_ringing)) }
        }
    }
}

@Composable
private fun PcCard(device: Device, onRing: (String, Boolean) -> Unit) {
    var ringing by remember(device.id) { mutableStateOf(false) }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.primaryContainer,
        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
    ) {
        Column(Modifier.fillMaxWidth().padding(24.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier.size(10.dp).padding(1.dp),
                ) {
                    Surface(
                        shape = CircleShape,
                        color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                        modifier = Modifier.size(8.dp),
                    ) {}
                }
                Spacer(Modifier.width(8.dp))
                Text(linkText(device.link), style = MaterialTheme.typography.labelLarge)
            }
            Spacer(Modifier.height(12.dp))
            Text(device.name, style = MaterialTheme.typography.headlineLarge)
            device.model?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 2.dp))
            }
            device.battery?.let { battery ->
                Spacer(Modifier.height(12.dp))
                AssistChip(
                    onClick = {},
                    label = {
                        Text(
                            if (battery.charging) stringResource(R.string.battery_charging, battery.level.toInt())
                            else stringResource(R.string.battery_level, battery.level.toInt()),
                        )
                    },
                )
            }
            Spacer(Modifier.height(20.dp))
            Button(
                enabled = device.online,
                onClick = {
                    ringing = !ringing
                    onRing(device.id, ringing)
                },
            ) {
                Text(stringResource(if (ringing) R.string.action_stop_ringing else R.string.action_ring_pc))
            }
        }
    }
}

@Composable
private fun linkText(link: Link): String = when (link) {
    is Link.Online -> when {
        link.relayed -> stringResource(R.string.link_away)
        link.rttMs >= 1u -> stringResource(R.string.link_online_rtt, link.rttMs.toInt())
        else -> stringResource(R.string.link_online)
    }
    Link.Connecting -> stringResource(R.string.link_connecting)
    is Link.Offline -> when (val seen = link.lastSeen) {
        null -> stringResource(R.string.link_never)
        else -> {
            val now = System.currentTimeMillis()
            if (now - seen * 1000 < DateUtils.MINUTE_IN_MILLIS) {
                stringResource(R.string.link_offline_now)
            } else {
                stringResource(
                    R.string.link_offline_seen,
                    DateUtils.getRelativeTimeSpanString(seen * 1000, now, DateUtils.MINUTE_IN_MILLIS),
                )
            }
        }
    }
}
