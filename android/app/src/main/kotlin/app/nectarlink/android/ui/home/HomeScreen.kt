// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.graphics.BitmapFactory
import android.net.Uri
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.LocalNetwork
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.core.isFinished
import app.nectarlink.android.files.transferTitle
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.UpdateCard
import app.nectarlink.core.AudioOutputDevice
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.ClipboardItemKind
import app.nectarlink.core.DeckState
import app.nectarlink.core.Link
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
    onWebcam: (pcId: String) -> Unit = {},
    updater: AppUpdater,
    onSendFiles: (pcId: String, uris: List<Uri>) -> Unit,
    onSendFolder: (pcId: String, tree: Uri) -> Unit,
    onAccessChanged: () -> Unit,
    onCancelTransfer: (id: String) -> Unit,
    onAllowStorage: (pcId: String) -> Unit = {},
    onDismissStorageRequest: () -> Unit = {},
    onAcceptWebcamRequest: (app.nectarlink.android.webcam.WebcamRequest) -> Unit = {},
    onDismissWebcamRequest: () -> Unit = {},
    onCopyClipboardHistory: (id: String) -> Unit = {},
    onPinClipboardHistory: (id: String, pinned: Boolean) -> Unit = { _, _ -> },
    onDeleteClipboardHistory: (id: String) -> Unit = {},
    onClearClipboardHistory: () -> Unit = {},
    loadClipboardHistoryImage: suspend (id: String) -> ByteArray? = { null },
    onSetPcAudio: (pcId: String, volume: Int?, muted: Boolean?) -> Unit = { _, _, _ -> },
    modifier: Modifier = Modifier,
) {
    var showClipboardHistory by remember { mutableStateOf(false) }
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
        state.webcamRequest?.let { req ->
            item {
                val pcName = state.nameOf(req.pcId) ?: stringResource(R.string.your_pc)
                Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text(
                            stringResource(R.string.webcam_request_title, pcName),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.webcam_request_text),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(12.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { onAcceptWebcamRequest(req) }) {
                                Text(stringResource(R.string.webcam_start))
                            }
                            OutlinedButton(onClick = onDismissWebcamRequest) {
                                Text(stringResource(R.string.action_not_now))
                            }
                        }
                    }
                }
            }
        }
        state.storageRequestedFrom?.let { pcId ->
            item {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                val context = LocalContext.current
                Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text(
                            stringResource(R.string.storage_request_title, pcName),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.storage_request_text, pcName),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(12.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = {
                                onAllowStorage(pcId)
                                if (!state.storageAllFilesAccess && state.storageSafFolders.isEmpty()) {
                                    context.startActivity(
                                        app.nectarlink.android.storage.PhoneStorage.allFilesAccessIntent(context),
                                    )
                                }
                            }) { Text(stringResource(R.string.action_allow)) }
                            OutlinedButton(onClick = onDismissStorageRequest) {
                                Text(stringResource(R.string.action_not_now))
                            }
                        }
                    }
                }
            }
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
            PcCard(
                device = device,
                onRing = onRing,
                onSendFiles = onSendFiles,
                onSendFolder = onSendFolder,
                onPower = onPower,
                onWake = onWake,
                onRemote = onRemote,
                onDeck = onDeck,
                onRecord = onRecord,
                onWebcam = onWebcam,
                onSetPcAudio = onSetPcAudio,
            )
        }
        if (state.clipboardHistoryEnabled && state.clipboardHistory.isNotEmpty()) {
            item {
                ClipboardHistoryCard(
                    entries = state.clipboardHistory,
                    onOpen = { showClipboardHistory = true },
                )
            }
        }
        if (state.transfers.isNotEmpty()) {
            item { TransfersCard(state, onCancelTransfer) }
        }
    }

    if (showClipboardHistory && state.clipboardHistoryEnabled) {
        ClipboardHistoryDialog(
            entries = state.clipboardHistory,
            onCopy = onCopyClipboardHistory,
            onPin = onPinClipboardHistory,
            onDelete = onDeleteClipboardHistory,
            onClearAll = onClearClipboardHistory,
            loadImage = loadClipboardHistoryImage,
            onDismiss = { showClipboardHistory = false },
        )
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
    onWebcam: (String) -> Unit,
    onSetPcAudio: (String, Int?, Boolean?) -> Unit,
) {
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(device.id, uris)
    }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) onSendFolder(device.id, tree)
    }
    var ringing by remember(device.id) { mutableStateOf(false) }
    var showMoreActions by remember(device.id) { mutableStateOf(false) }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.primaryContainer,
        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
    ) {
        Column(Modifier.fillMaxWidth().padding(24.dp)) {
            val context = LocalContext.current
            // Secondary buttons take their colors from the card, so they
            // stand out on it in every theme (the theme's own tonal color
            // can be the card's color, as in Graphite).
            val ink = MaterialTheme.colorScheme.onPrimaryContainer
            val primaryBtn = ButtonDefaults.buttonColors(
                disabledContainerColor = ink.copy(alpha = 0.08f),
                disabledContentColor = ink.copy(alpha = 0.38f),
            )
            val tonal = ButtonDefaults.filledTonalButtonColors(
                containerColor = ink.copy(alpha = 0.10f),
                contentColor = ink,
                disabledContainerColor = ink.copy(alpha = 0.05f),
                disabledContentColor = ink.copy(alpha = 0.38f),
            )
            val outlined = ButtonDefaults.outlinedButtonColors(contentColor = ink, disabledContentColor = ink.copy(alpha = 0.38f))
            val outline = BorderStroke(1.dp, ink.copy(alpha = if (device.online) 0.45f else 0.15f))

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
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
                    modifier = Modifier.weight(1f),
                )
                if (!device.online && device.canWake && device.wakeState != WakeState.Waking) {
                    Spacer(Modifier.width(8.dp))
                    FilledTonalButton(
                        colors = tonal,
                        onClick = { onWake(device.id) },
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp, vertical = 4.dp),
                        modifier = Modifier.heightIn(min = 32.dp),
                    ) {
                        Text(
                            stringResource(R.string.action_wake_pc),
                            style = MaterialTheme.typography.labelMedium,
                            maxLines = 1,
                        )
                    }
                }
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
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                // Primary 2x2 grid: Control PC, Deck, Webcam, Send files
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Button(
                        enabled = device.online,
                        colors = primaryBtn,
                        onClick = { onRemote(device.id) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.action_remote_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    FilledTonalButton(
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onDeck(device.id) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.action_deck_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onWebcam(device.id) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.action_webcam_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    FilledTonalButton(
                        enabled = device.online,
                        colors = tonal,
                        onClick = { pickFiles.launch(arrayOf("*/*")) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.action_send_files), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }

                // Live PC master volume, mute, and active output device bar
                val deckState = device.deckState
                if (device.online && deckState != null) {
                    PcAudioCardSection(
                        pcName = device.name,
                        deckState = deckState,
                        ink = ink,
                        onSetPcAudio = { vol, muted -> onSetPcAudio(device.id, vol, muted) },
                    )
                }

                val expanded = showMoreActions || ringing
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    TextButton(
                        onClick = { showMoreActions = !expanded },
                        colors = ButtonDefaults.textButtonColors(contentColor = ink.copy(alpha = 0.85f)),
                    ) {
                        Text(
                            text = stringResource(if (expanded) R.string.action_less else R.string.action_more),
                            style = MaterialTheme.typography.labelLarge,
                        )
                    }
                    if (ringing && !showMoreActions) {
                        FilledTonalButton(
                            colors = tonal,
                            onClick = {
                                ringing = false
                                onRing(device.id, false)
                            },
                        ) {
                            Text(stringResource(R.string.action_stop_ringing))
                        }
                    }
                }

                if (expanded) {
                    Surface(
                        shape = MaterialTheme.shapes.large,
                        color = ink.copy(alpha = 0.06f),
                        contentColor = ink,
                    ) {
                        Column(
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                FilledTonalButton(
                                    enabled = device.online,
                                    colors = tonal,
                                    onClick = { pickFolder.launch(null) },
                                    modifier = Modifier.weight(1f),
                                ) {
                                    Text(stringResource(R.string.action_send_folder), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                                FilledTonalButton(
                                    enabled = device.online,
                                    colors = tonal,
                                    onClick = { context.startActivity(SendActivity.sendClipboardIntent(context)) },
                                    modifier = Modifier.weight(1f),
                                ) {
                                    Text(stringResource(R.string.action_send_clipboard), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                            }
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                FilledTonalButton(
                                    colors = tonal,
                                    onClick = { onRecord(device.id) },
                                    modifier = Modifier.weight(1f),
                                ) {
                                    Text(stringResource(R.string.action_record_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                                FilledTonalButton(
                                    enabled = device.online,
                                    colors = tonal,
                                    onClick = {
                                        ringing = !ringing
                                        onRing(device.id, ringing)
                                    },
                                    modifier = Modifier.weight(1f),
                                ) {
                                    Text(
                                        stringResource(if (ringing) R.string.action_stop_ringing else R.string.action_ring_pc),
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                            }
                            if (device.has("device.pc_actions")) {
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    OutlinedButton(
                                        enabled = device.online,
                                        colors = outlined,
                                        border = outline,
                                        onClick = { onPower(device.id, false) },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(stringResource(R.string.action_lock_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    }
                                    OutlinedButton(
                                        enabled = device.online,
                                        colors = outlined,
                                        border = outline,
                                        onClick = { onPower(device.id, true) },
                                        modifier = Modifier.weight(1f),
                                    ) {
                                        Text(stringResource(R.string.action_sleep_pc), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    }
                                }
                            }
                        }
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

@Composable
private fun PcAudioCardSection(
    pcName: String,
    deckState: DeckState,
    ink: Color,
    onSetPcAudio: (volume: Int?, muted: Boolean?) -> Unit,
) {
    var draggingVolume by remember { mutableStateOf<Float?>(null) }
    var showOutputs by remember { mutableStateOf(false) }
    val shownVolume = draggingVolume?.roundToInt() ?: deckState.volume.toInt()
    val defaultOutput = deckState.outputDevices.firstOrNull { it.isDefault } ?: deckState.outputDevices.firstOrNull()

    Surface(
        shape = MaterialTheme.shapes.large,
        color = ink.copy(alpha = 0.06f),
        contentColor = ink,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 6.dp),
            verticalArrangement = Arrangement.spacedBy(0.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                IconButton(
                    onClick = { onSetPcAudio(null, !deckState.muted) },
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        painter = painterResource(if (deckState.muted) R.drawable.ic_sound_off else R.drawable.ic_speaker),
                        contentDescription = stringResource(
                            if (deckState.muted) R.string.pc_audio_unmute else R.string.pc_audio_mute,
                        ),
                        tint = ink.copy(alpha = if (deckState.muted) 0.55f else 0.9f),
                        modifier = Modifier.size(20.dp),
                    )
                }
                Slider(
                    value = (draggingVolume ?: deckState.volume.toFloat()).coerceIn(0f, 100f),
                    onValueChange = { v ->
                        draggingVolume = v
                        onSetPcAudio(v.roundToInt(), null)
                    },
                    onValueChangeFinished = { draggingVolume = null },
                    valueRange = 0f..100f,
                    colors = SliderDefaults.colors(
                        thumbColor = ink,
                        activeTrackColor = ink,
                        inactiveTrackColor = ink.copy(alpha = 0.22f),
                    ),
                    modifier = Modifier.weight(1f).height(32.dp),
                )
                Text(
                    text = if (deckState.muted) {
                        stringResource(R.string.pc_audio_muted)
                    } else {
                        stringResource(R.string.deck_volume_percent, shownVolume)
                    },
                    style = MaterialTheme.typography.labelMedium,
                )
            }
            if (defaultOutput != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { showOutputs = true }
                        .padding(start = 38.dp, end = 4.dp, bottom = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = defaultOutput.name,
                        style = MaterialTheme.typography.labelSmall,
                        color = ink.copy(alpha = 0.68f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (deckState.outputDevices.size > 1) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = "${deckState.outputDevices.size}",
                            style = MaterialTheme.typography.labelSmall,
                            color = ink.copy(alpha = 0.55f),
                        )
                    }
                }
            }
        }
    }

    if (showOutputs && deckState.outputDevices.isNotEmpty()) {
        PcAudioOutputsDialog(
            pcName = pcName,
            devices = deckState.outputDevices,
            onDismiss = { showOutputs = false },
        )
    }
}

@Composable
internal fun PcAudioOutputsDialog(
    pcName: String,
    devices: List<AudioOutputDevice>,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.pc_audio_outputs_title, pcName)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    stringResource(R.string.pc_audio_outputs_hint, pcName),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                devices.forEach { out ->
                    Surface(
                        shape = MaterialTheme.shapes.medium,
                        color = if (out.isDefault) {
                            MaterialTheme.colorScheme.secondaryContainer
                        } else {
                            MaterialTheme.colorScheme.surfaceContainerHigh
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(
                                text = out.name,
                                style = MaterialTheme.typography.bodyMedium,
                                color = if (out.isDefault) {
                                    MaterialTheme.colorScheme.onSecondaryContainer
                                } else {
                                    MaterialTheme.colorScheme.onSurface
                                },
                                maxLines = 2,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                            if (out.isDefault) {
                                Spacer(Modifier.width(8.dp))
                                Text(
                                    text = stringResource(R.string.pc_audio_default_badge),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.clipboard_history_done))
            }
        },
    )
}

@Composable
private fun ClipboardHistoryCard(
    entries: List<ClipboardHistoryEntry>,
    onOpen: () -> Unit,
) {
    val latest = entries.firstOrNull() ?: return
    val preview = when (latest.kind) {
        ClipboardItemKind.TEXT -> latest.text
        ClipboardItemKind.IMAGE -> stringResource(R.string.clipboard_history_image)
    }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpen),
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Text(stringResource(R.string.clipboard_history_title), style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(4.dp))
            Text(
                preview,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun ClipboardHistoryDialog(
    entries: List<ClipboardHistoryEntry>,
    onCopy: (String) -> Unit,
    onPin: (String, Boolean) -> Unit,
    onDelete: (String) -> Unit,
    onClearAll: () -> Unit,
    loadImage: suspend (String) -> ByteArray?,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    val trimmed = query.trim().lowercase()
    val filtered = remember(entries, trimmed) {
        if (trimmed.isEmpty()) {
            entries
        } else {
            entries.filter { entry ->
                entry.text.lowercase().contains(trimmed) ||
                    entry.deviceName.lowercase().contains(trimmed) ||
                    (entry.kind == ClipboardItemKind.IMAGE && "image".contains(trimmed))
            }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Column {
                Text(stringResource(R.string.clipboard_history_title))
                Spacer(Modifier.height(4.dp))
                Text(
                    stringResource(R.string.clipboard_history_subtitle),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (entries.isNotEmpty()) {
                    OutlinedTextField(
                        value = query,
                        onValueChange = { query = it },
                        placeholder = { Text(stringResource(R.string.clipboard_history_search)) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                when {
                    entries.isEmpty() -> Text(
                        stringResource(R.string.clipboard_history_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    filtered.isEmpty() -> Text(
                        stringResource(R.string.clipboard_history_no_match, query.trim()),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    else -> LazyColumn(
                        modifier = Modifier.fillMaxWidth().heightIn(max = 360.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        items(filtered, key = { it.id }) { entry ->
                            ClipboardHistoryRow(
                                entry = entry,
                                onCopy = { onCopy(entry.id) },
                                onTogglePin = { onPin(entry.id, !entry.pinned) },
                                onDelete = { onDelete(entry.id) },
                                loadImage = loadImage,
                            )
                        }
                    }
                }
            }
        },
        dismissButton = {
            if (entries.isNotEmpty()) {
                TextButton(onClick = onClearAll) {
                    Text(stringResource(R.string.clipboard_history_clear))
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.clipboard_history_done))
            }
        },
    )
}

@Composable
private fun ClipboardHistoryRow(
    entry: ClipboardHistoryEntry,
    onCopy: () -> Unit,
    onTogglePin: () -> Unit,
    onDelete: () -> Unit,
    loadImage: suspend (String) -> ByteArray?,
) {
    val direction = if (entry.incoming) {
        stringResource(R.string.clipboard_history_from, entry.deviceName)
    } else {
        stringResource(R.string.clipboard_history_sent_to, entry.deviceName)
    }
    val now = System.currentTimeMillis()
    val whenText = if (now - entry.timestamp * 1000L < DateUtils.MINUTE_IN_MILLIS) {
        stringResource(R.string.clipboard_history_just_now)
    } else {
        DateUtils.getRelativeTimeSpanString(entry.timestamp * 1000L, now, DateUtils.MINUTE_IN_MILLIS).toString()
    }
    val header = buildString {
        append(direction)
        append(" · ")
        append(whenText)
        if (entry.pinned) {
            append(" · ")
            append(stringResource(R.string.clipboard_history_pinned))
        }
    }
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                header,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            when (entry.kind) {
                ClipboardItemKind.TEXT -> Text(
                    entry.text,
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = 4,
                    overflow = TextOverflow.Ellipsis,
                )
                ClipboardItemKind.IMAGE -> {
                    val bitmap by produceState<ImageBitmap?>(initialValue = null, entry.id) {
                        val bytes = loadImage(entry.id) ?: return@produceState
                        value = withContext(Dispatchers.Default) { previewBitmap(bytes)?.asImageBitmap() }
                    }
                    val image = bitmap
                    if (image != null) {
                        Image(
                            bitmap = image,
                            contentDescription = stringResource(R.string.clipboard_history_image),
                            modifier = Modifier.fillMaxWidth().heightIn(max = 120.dp),
                            contentScale = ContentScale.Fit,
                        )
                    } else {
                        Text(
                            stringResource(R.string.clipboard_history_image),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                FilledTonalButton(onClick = onCopy) {
                    Text(stringResource(R.string.clipboard_history_copy))
                }
                TextButton(onClick = onTogglePin) {
                    Text(
                        stringResource(
                            if (entry.pinned) R.string.clipboard_history_unpin else R.string.clipboard_history_pin,
                        ),
                    )
                }
                Spacer(Modifier.weight(1f))
                TextButton(onClick = onDelete) {
                    Text(stringResource(R.string.clipboard_history_delete))
                }
            }
        }
    }
}

/** A history image decoded small enough for its row (a clip can be a large photo). */
private fun previewBitmap(bytes: ByteArray): android.graphics.Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= PREVIEW_PX) sample *= 2
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
}

private const val PREVIEW_PX = 720

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
