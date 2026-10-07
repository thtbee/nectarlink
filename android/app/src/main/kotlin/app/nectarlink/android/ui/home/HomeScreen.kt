// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.net.Uri
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.TextButton
import androidx.compose.ui.text.style.TextOverflow
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
import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.UpdateCard
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.LocalNetwork
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.files.transferTitle
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.core.isFinished
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.core.Link
import androidx.compose.material3.IconButton
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.graphics.graphicsLayer
import kotlinx.coroutines.launch

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
    onRefresh: () -> Unit,
    onPower: (pcId: String, sleep: Boolean) -> Unit,
    onWake: (pcId: String) -> Unit,
    onRemote: (pcId: String) -> Unit,
    onDeck: (pcId: String) -> Unit,
    onRecord: (pcId: String) -> Unit,
    updater: AppUpdater,
    onSendFiles: (pcId: String, uris: List<Uri>) -> Unit,
    onSendFolder: (pcId: String, tree: Uri) -> Unit,
    onAccessChanged: () -> Unit,
    onCancelTransfer: (id: String) -> Unit,
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
                // Reconnects and syncs with the PCs; spins once to say so.
                val spin = remember { androidx.compose.animation.core.Animatable(0f) }
                val scope = rememberCoroutineScope()
                IconButton(onClick = {
                    onRefresh()
                    scope.launch {
                        spin.snapTo(0f)
                        spin.animateTo(360f, androidx.compose.animation.core.tween(700))
                    }
                }) {
                    Icon(
                        painterResource(R.drawable.ic_refresh),
                        contentDescription = stringResource(R.string.action_refresh),
                        modifier = Modifier.graphicsLayer { rotationZ = spin.value },
                    )
                }
                FilledTonalIconButton(onClick = onPairNew) {
                    Icon(painterResource(R.drawable.ic_qr), contentDescription = stringResource(R.string.action_pair_new))
                }
            }
        }
        state.ringingFrom?.let { from ->
            item { RingingBanner(from, onStopRinging) }
        }
        item { UpdateCard(updater) }
        if (!state.localNetwork) {
            item {
                val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { onAccessChanged() }
                SetupCard(
                    stringResource(R.string.local_network_title),
                    stringResource(R.string.local_network_text),
                ) { ask.launch(LocalNetwork.PERMISSION) }
            }
        }
        if (state.devices.isNotEmpty() && !state.backgroundUnrestricted) {
            item {
                val context = LocalContext.current
                SetupCard(
                    stringResource(R.string.background_title),
                    stringResource(R.string.background_text),
                ) { context.startActivity(BackgroundAccess.requestIntent(context)) }
            }
        }
        if (state.devices.isNotEmpty() && !state.notificationAccess) {
            item {
                val context = LocalContext.current
                SetupCard(
                    stringResource(R.string.notifications_title),
                    stringResource(R.string.notifications_off),
                ) { context.startActivity(NotificationListener.settingsIntent(context)) }
            }
        }
        if (state.devices.isNotEmpty() && !state.smsAccess) {
            item {
                val askSms = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
                    onAccessChanged()
                }
                SetupCard(
                    stringResource(R.string.sms_title),
                    stringResource(R.string.sms_off),
                ) { askSms.launch(PhoneSms.permissions) }
            }
        }
        if (state.devices.isNotEmpty() && !state.callAccess) {
            item {
                val askCalls = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
                    onAccessChanged()
                }
                SetupCard(
                    stringResource(R.string.calls_title),
                    stringResource(R.string.calls_off),
                ) { askCalls.launch(PhoneCalls.permissions) }
            }
        }
        if (state.devices.isNotEmpty() && !state.contactsAccess) {
            item {
                val askContacts = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
                    onAccessChanged()
                }
                SetupCard(
                    stringResource(R.string.contacts_title),
                    stringResource(R.string.contacts_off),
                ) { askContacts.launch(PhoneContacts.permissions) }
            }
        }
        if (state.devices.isNotEmpty() && !state.photoAccess) {
            item {
                val askPhotos = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
                    onAccessChanged()
                }
                SetupCard(
                    stringResource(R.string.photos_title),
                    stringResource(R.string.photos_off),
                ) { askPhotos.launch(RecentPhotos.permissions) }
            }
        }
        items(state.devices, key = { it.id }) { device ->
            PcCard(device, onRing, onSendFiles, onSendFolder, onPower, onWake, onRemote, onDeck, onRecord)
        }
        if (state.transfers.isNotEmpty()) {
            item { TransfersCard(state, onCancelTransfer) }
        }
    }
}

/** Something the phone needs before mirroring works, with the button that fixes it. */
@Composable
private fun SetupCard(title: String, text: String, onAllow: () -> Unit) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
            Spacer(Modifier.height(4.dp))
            Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
            Spacer(Modifier.height(12.dp))
            Button(onClick = onAllow) { Text(stringResource(R.string.action_allow)) }
        }
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
private fun PcCard(
    device: Device,
    onRing: (String, Boolean) -> Unit,
    onSendFiles: (String, List<Uri>) -> Unit,
    onSendFolder: (String, Uri) -> Unit,
    onPower: (String, Boolean) -> Unit,
    onWake: (String) -> Unit,
    onRemote: (String) -> Unit,
    onDeck: (String) -> Unit,
    onRecord: (String) -> Unit,
) {
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(device.id, uris)
    }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) onSendFolder(device.id, tree)
    }
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
                Text(
                    if (!device.online && device.wakeState == WakeState.Waking) {
                        stringResource(R.string.wake_waking, device.name)
                    } else {
                        linkText(device.link)
                    },
                    style = MaterialTheme.typography.labelLarge,
                )
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
            val context = LocalContext.current
            // Secondary buttons take their colors from the card, so they
            // stand out on it in every theme (the theme's own tonal color
            // can be the card's color, as in Graphite).
            val ink = MaterialTheme.colorScheme.onPrimaryContainer
            val tonal = ButtonDefaults.filledTonalButtonColors(
                containerColor = ink.copy(alpha = 0.10f),
                contentColor = ink,
                disabledContainerColor = ink.copy(alpha = 0.05f),
                disabledContentColor = ink.copy(alpha = 0.38f),
            )
            val outlined = ButtonDefaults.outlinedButtonColors(contentColor = ink, disabledContentColor = ink.copy(alpha = 0.38f))
            val outline = BorderStroke(1.dp, ink.copy(alpha = if (device.online) 0.45f else 0.15f))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!device.online && device.canWake) {
                    Button(
                        enabled = device.wakeState != WakeState.Waking,
                        onClick = { onWake(device.id) },
                    ) {
                        Text(
                            if (device.wakeState == WakeState.Waking) stringResource(R.string.wake_waking, device.name)
                            else stringResource(R.string.action_wake_pc),
                        )
                    }
                }
                Button(
                    enabled = device.online,
                    onClick = {
                        ringing = !ringing
                        onRing(device.id, ringing)
                    },
                ) {
                    Text(stringResource(if (ringing) R.string.action_stop_ringing else R.string.action_ring_pc))
                }
                FilledTonalButton(
                    colors = tonal,
                    onClick = { onRemote(device.id) },
                ) {
                    Text(stringResource(R.string.action_remote_pc))
                }
                FilledTonalButton(
                    colors = tonal,
                    onClick = { onDeck(device.id) },
                ) {
                    Text(stringResource(R.string.action_deck_pc))
                }
                FilledTonalButton(
                    colors = tonal,
                    onClick = { onRecord(device.id) },
                ) {
                    Text(stringResource(R.string.action_record_pc))
                }
                FilledTonalButton(
                    enabled = device.online,
                    colors = tonal,
                    onClick = { context.startActivity(SendActivity.sendClipboardIntent(context)) },
                ) {
                    Text(stringResource(R.string.action_send_clipboard))
                }
                FilledTonalButton(enabled = device.online, colors = tonal, onClick = { pickFiles.launch(arrayOf("*/*")) }) {
                    Text(stringResource(R.string.action_send_files))
                }
                FilledTonalButton(enabled = device.online, colors = tonal, onClick = { pickFolder.launch(null) }) {
                    Text(stringResource(R.string.action_send_folder))
                }
                if (device.has("device.pc_actions")) {
                    OutlinedButton(enabled = device.online, colors = outlined, border = outline, onClick = { onPower(device.id, false) }) {
                        Text(stringResource(R.string.action_lock_pc))
                    }
                    OutlinedButton(enabled = device.online, colors = outlined, border = outline, onClick = { onPower(device.id, true) }) {
                        Text(stringResource(R.string.action_sleep_pc))
                    }
                }
            }
            if (!device.online && device.wakeState == WakeState.Waking) {
                Spacer(Modifier.height(14.dp))
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            } else if (!device.online && device.wakeState == WakeState.TimedOut) {
                Spacer(Modifier.height(16.dp))
                Surface(
                    shape = MaterialTheme.shapes.large,
                    color = ink.copy(alpha = 0.08f),
                    contentColor = ink,
                ) {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text(
                            stringResource(R.string.wake_timeout_title, device.name),
                            style = MaterialTheme.typography.titleSmall,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.wake_timeout_text, device.name),
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }
    }
}

/** Running transfers and the latest finished ones. */
@Composable
private fun TransfersCard(state: CoreState, onCancel: (String) -> Unit) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(stringResource(R.string.transfers_title), style = MaterialTheme.typography.titleMedium)
            state.transfers.forEach { transfer -> TransferRow(transfer, state.nameOf(transfer.deviceId).orEmpty(), onCancel) }
        }
    }
}

@Composable
private fun TransferRow(transfer: Transfer, pc: String, onCancel: (String) -> Unit) {
    val incoming = transfer.direction == TransferDirection.INCOMING
    val status = transfer.status
    val title = transferTitle(LocalContext.current.resources, transfer)
    val detail = when (status) {
        is TransferStatus.Waiting -> stringResource(R.string.transfer_waiting, pc)
        is TransferStatus.Running ->
            stringResource(if (incoming) R.string.transfer_receiving else R.string.transfer_sending, pc) +
                " · " + Formatter.formatShortFileSize(LocalContext.current, transfer.done.toLong()) +
                " / " + Formatter.formatShortFileSize(LocalContext.current, transfer.total.toLong())
        is TransferStatus.Done ->
            if (incoming) stringResource(R.string.transfer_received_from, pc) else stringResource(R.string.transfer_sent_to, pc)
        is TransferStatus.Cancelled -> stringResource(R.string.transfer_cancelled)
        is TransferStatus.Failed -> when (status.reason) {
            "denied" -> stringResource(R.string.transfer_denied, pc)
            "unreachable" -> stringResource(R.string.transfer_unreachable, pc)
            "noSpace" -> stringResource(R.string.transfer_no_space)
            else -> stringResource(R.string.transfer_failed)
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!transfer.isFinished()) {
                Spacer(Modifier.height(6.dp))
                val progress = if (transfer.total == 0uL) 1f else (transfer.done.toDouble() / transfer.total.toDouble()).toFloat()
                LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
            }
            Spacer(Modifier.height(4.dp))
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (!transfer.isFinished()) {
            TextButton(onClick = { onCancel(transfer.id) }) { Text(stringResource(R.string.action_cancel)) }
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
