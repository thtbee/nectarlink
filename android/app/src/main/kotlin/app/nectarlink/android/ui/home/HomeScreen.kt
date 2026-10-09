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
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
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
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.UpdateCard
import android.app.DownloadManager
import android.content.Intent
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilterChip
import androidx.compose.runtime.LaunchedEffect
import androidx.core.content.FileProvider
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.core.AudioOutputDevice
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.ClipboardItemKind
import app.nectarlink.core.DeckState
import app.nectarlink.core.Link
import app.nectarlink.core.LocalSendPeer
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import app.nectarlink.core.TimelinePage
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import java.io.File
import java.text.DateFormat
import java.util.Calendar
import java.util.Date
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
    onAcceptTransfer: (id: String) -> Unit = {},
    onRefreshLocalSend: () -> Unit = {},
    onAllowStorage: (pcId: String) -> Unit = {},
    onDismissStorageRequest: () -> Unit = {},
    onAcceptWebcamRequest: (app.nectarlink.android.webcam.WebcamRequest) -> Unit = {},
    onDismissWebcamRequest: () -> Unit = {},
    onCopyClipboardHistory: (id: String) -> Unit = {},
    onPinClipboardHistory: (id: String, pinned: Boolean) -> Unit = { _, _ -> },
    onDeleteClipboardHistory: (id: String) -> Unit = {},
    onClearClipboardHistory: () -> Unit = {},
    loadClipboardHistoryImage: suspend (id: String) -> ByteArray? = { null },
    queryTimelinePage: suspend (kind: TimelineKind?, deviceId: String?, search: String?, offset: UInt) -> TimelinePage? = { _, _, _, _ -> null },
    onDeleteTimelineEntry: (id: Long) -> Unit = {},
    onClearTimeline: () -> Unit = {},
    onResendTimelineEntry: (entry: TimelineEntry) -> Unit = {},
    onSetPcAudio: (pcId: String, volume: Int?, muted: Boolean?) -> Unit = { _, _, _ -> },
    onStopKeyboardFromPc: (pcId: String) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var showClipboardHistory by remember { mutableStateOf(false) }
    var showTimeline by remember { mutableStateOf(false) }
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
                val reducedMotion = LocalReducedMotion.current
                val spin = remember { androidx.compose.animation.core.Animatable(0f) }
                val scope = rememberCoroutineScope()
                IconButton(onClick = {
                    onRefresh()
                    if (!reducedMotion) {
                        scope.launch {
                            spin.snapTo(0f)
                            spin.animateTo(360f, androidx.compose.animation.core.tween(700))
                        }
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
        state.keyboardFromPc.forEach { pcId ->
            item(key = "keyboard_$pcId") {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                KeyboardBanner(pcName, onStop = { onStopKeyboardFromPc(pcId) })
            }
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
        if (state.localsendEnabled) {
            item {
                LocalSendCard(
                    peers = state.localsendPeers,
                    onRefresh = onRefreshLocalSend,
                    onSendFiles = onSendFiles,
                )
            }
        }
        if (state.clipboardHistoryEnabled && state.clipboardHistory.isNotEmpty()) {
            item {
                ClipboardHistoryCard(
                    entries = state.clipboardHistory,
                    onOpen = { showClipboardHistory = true },
                )
            }
        }
        if (state.timeline.isNotEmpty()) {
            item {
                TimelineCard(
                    entries = state.timeline,
                    total = state.timelineTotal,
                    onOpen = { showTimeline = true },
                )
            }
        }
        if (state.transfers.isNotEmpty()) {
            item { TransfersCard(state, onAcceptTransfer, onCancelTransfer) }
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

    if (showTimeline) {
        TimelineDialog(
            state = state,
            queryPage = queryTimelinePage,
            onCopyClip = onCopyClipboardHistory,
            onResend = onResendTimelineEntry,
            onDelete = onDeleteTimelineEntry,
            onClearAll = onClearTimeline,
            loadClipImage = loadClipboardHistoryImage,
            onDismiss = { showTimeline = false },
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
private fun KeyboardBanner(from: String, onStop: () -> Unit) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
        Row(Modifier.fillMaxWidth().padding(20.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.keyboard_from_pc_banner, from),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.weight(1f),
            )
            Spacer(Modifier.width(12.dp))
            Button(onClick = onStop) { Text(stringResource(R.string.keyboard_stop)) }
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
                if (LocalReducedMotion.current) {
                    LinearProgressIndicator(progress = { 0.5f }, modifier = Modifier.fillMaxWidth())
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
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
    val volumeLabel = stringResource(R.string.pc_audio_volume)

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
                        tint = ink.copy(alpha = if (deckState.muted) 0.85f else 0.9f),
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
                    modifier = Modifier
                        .weight(1f)
                        .height(32.dp)
                        .semantics { contentDescription = volumeLabel },
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
                        .clickable(role = Role.Button) { showOutputs = true }
                        .padding(start = 38.dp, end = 4.dp, bottom = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = defaultOutput.name,
                        style = MaterialTheme.typography.labelSmall,
                        color = ink.copy(alpha = 0.85f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (deckState.outputDevices.size > 1) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = "${deckState.outputDevices.size}",
                            style = MaterialTheme.typography.labelSmall,
                            color = ink.copy(alpha = 0.85f),
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
        modifier = Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onOpen),
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
                    val searchLabel = stringResource(R.string.clipboard_history_search)
                    OutlinedTextField(
                        value = query,
                        onValueChange = { query = it },
                        placeholder = { Text(searchLabel) },
                        singleLine = true,
                        modifier = Modifier
                            .fillMaxWidth()
                            .semantics { contentDescription = searchLabel },
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
    val header = if (entry.pinned) {
        stringResource(R.string.clipboard_history_meta_pinned, direction, whenText)
    } else {
        stringResource(R.string.clipboard_history_meta, direction, whenText)
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

@Composable
private fun LocalSendCard(
    peers: List<LocalSendPeer>,
    onRefresh: () -> Unit,
    onSendFiles: (pcId: String, uris: List<Uri>) -> Unit,
) {
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    stringResource(R.string.localsend_card_title),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                IconButton(
                    onClick = onRefresh,
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        painterResource(R.drawable.ic_refresh),
                        contentDescription = stringResource(R.string.action_refresh),
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
            if (peers.isEmpty()) {
                Text(
                    stringResource(R.string.localsend_card_empty),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                peers.forEach { peer ->
                    LocalSendPeerRow(peer = peer, onSendFiles = onSendFiles)
                }
            }
        }
    }
}

@Composable
private fun LocalSendPeerRow(
    peer: LocalSendPeer,
    onSendFiles: (pcId: String, uris: List<Uri>) -> Unit,
) {
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(peer.id, uris)
    }
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                Text(
                    peer.alias,
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val subtitle = peer.deviceModel?.takeIf { it.isNotBlank() } ?: peer.ip
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            FilledTonalButton(onClick = { pickFiles.launch(arrayOf("*/*")) }) {
                Text(stringResource(R.string.action_send_files))
            }
        }
    }
}

/** Running transfers and the latest finished ones. */
@Composable
private fun TransfersCard(
    state: CoreState,
    onAccept: (String) -> Unit,
    onCancel: (String) -> Unit,
) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(stringResource(R.string.transfers_title), style = MaterialTheme.typography.titleMedium)
            state.transfers.forEach { transfer ->
                TransferRow(
                    transfer = transfer,
                    pc = state.nameOf(transfer.deviceId).orEmpty(),
                    onAccept = onAccept,
                    onCancel = onCancel,
                )
            }
        }
    }
}

@Composable
private fun TransferRow(
    transfer: Transfer,
    pc: String,
    onAccept: (String) -> Unit,
    onCancel: (String) -> Unit,
) {
    val context = LocalContext.current
    val incoming = transfer.direction == TransferDirection.INCOMING
    val status = transfer.status
    val title = transferTitle(context.resources, transfer)
    val peerName = pc.ifEmpty { stringResource(R.string.localsend_nearby_device) }
    val detail = when (status) {
        is TransferStatus.Requested ->
            stringResource(
                R.string.transfer_requested_from_size,
                peerName,
                Formatter.formatShortFileSize(context, transfer.total.toLong()),
            )
        is TransferStatus.Waiting -> stringResource(R.string.transfer_waiting, peerName)
        is TransferStatus.Running ->
            stringResource(
                if (incoming) R.string.transfer_receiving_progress else R.string.transfer_sending_progress,
                peerName,
                Formatter.formatShortFileSize(context, transfer.done.toLong()),
                Formatter.formatShortFileSize(context, transfer.total.toLong()),
            )
        is TransferStatus.Done ->
            if (incoming) stringResource(R.string.transfer_received_from, peerName) else stringResource(R.string.transfer_sent_to, peerName)
        is TransferStatus.Cancelled -> stringResource(R.string.transfer_cancelled)
        is TransferStatus.Failed -> when (status.reason) {
            "denied" -> stringResource(R.string.transfer_denied, peerName)
            "unreachable" -> stringResource(R.string.transfer_unreachable, peerName)
            "noSpace" -> stringResource(R.string.transfer_no_space)
            else -> stringResource(R.string.transfer_failed)
        }
    }
    Column(modifier = Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (status is TransferStatus.Running || status is TransferStatus.Waiting) {
                    Spacer(Modifier.height(6.dp))
                    val reducedMotion = LocalReducedMotion.current
                    val progress = if (transfer.total == 0uL) {
                        if (reducedMotion) 0.5f else 1f
                    } else {
                        (transfer.done.toDouble() / transfer.total.toDouble()).toFloat()
                    }
                    LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
                }
                Spacer(Modifier.height(4.dp))
                Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (status is TransferStatus.Running || status is TransferStatus.Waiting) {
                TextButton(onClick = { onCancel(transfer.id) }) { Text(stringResource(R.string.action_cancel)) }
            }
        }
        if (status is TransferStatus.Requested) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { onAccept(transfer.id) }) {
                    Text(stringResource(R.string.action_accept))
                }
                OutlinedButton(onClick = { onCancel(transfer.id) }) {
                    Text(stringResource(R.string.action_decline))
                }
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

@Composable
private fun TimelineCard(
    entries: List<TimelineEntry>,
    total: UInt,
    onOpen: () -> Unit,
) {
    val latest = entries.firstOrNull() ?: return
    val direction = if (latest.incoming) {
        stringResource(R.string.clipboard_history_from, latest.deviceName)
    } else {
        stringResource(R.string.clipboard_history_sent_to, latest.deviceName)
    }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onOpen),
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(
                    stringResource(R.string.timeline_title),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                if (total > 0u) {
                    Text(
                        total.toString(),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.height(4.dp))
            Text(
                stringResource(R.string.timeline_latest_summary, latest.title, direction),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun TimelineDialog(
    state: CoreState,
    queryPage: suspend (TimelineKind?, String?, String?, UInt) -> TimelinePage?,
    onCopyClip: (String) -> Unit,
    onResend: (TimelineEntry) -> Unit,
    onDelete: (Long) -> Unit,
    onClearAll: () -> Unit,
    loadClipImage: suspend (String) -> ByteArray?,
    onDismiss: () -> Unit,
) {
    var search by remember { mutableStateOf("") }
    var kindFilter by remember { mutableStateOf<TimelineKind?>(null) }
    var deviceFilter by remember { mutableStateOf<String?>(null) }
    var pagedEntries by remember { mutableStateOf<List<TimelineEntry>>(state.timeline) }
    var hasMore by remember { mutableStateOf(state.timelineHasMore) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(state.timeline, search, kindFilter, deviceFilter) {
        val trimmed = search.trim().takeIf { it.isNotEmpty() }
        if (trimmed == null && kindFilter == null && deviceFilter == null) {
            pagedEntries = state.timeline
            hasMore = state.timelineHasMore
        } else {
            val page = queryPage(kindFilter, deviceFilter, trimmed, 0u)
            pagedEntries = page?.items ?: emptyList()
            hasMore = page?.hasMore ?: false
        }
    }

    val kindChips = listOf<Pair<TimelineKind?, String>>(
        null to stringResource(R.string.timeline_filter_all),
        TimelineKind.FILE to stringResource(R.string.timeline_filter_files),
        TimelineKind.CLIP to stringResource(R.string.timeline_filter_clips),
        TimelineKind.LINK to stringResource(R.string.timeline_filter_links),
        TimelineKind.PHOTO to stringResource(R.string.timeline_filter_photos),
        TimelineKind.RECORDING to stringResource(R.string.timeline_filter_recordings),
        TimelineKind.SESSION to stringResource(R.string.timeline_filter_sessions),
    )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Column {
                Text(stringResource(R.string.timeline_title))
                Spacer(Modifier.height(4.dp))
                Text(
                    stringResource(R.string.timeline_subtitle),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                val searchLabel = stringResource(R.string.timeline_search)
                OutlinedTextField(
                    value = search,
                    onValueChange = { search = it },
                    placeholder = { Text(searchLabel) },
                    singleLine = true,
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { contentDescription = searchLabel },
                )
                Row(
                    modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    kindChips.forEach { (kind, label) ->
                        FilterChip(
                            selected = kindFilter == kind,
                            onClick = { kindFilter = kind },
                            label = { Text(label) },
                        )
                    }
                }
                if (state.devices.size > 1) {
                    Row(
                        modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        FilterChip(
                            selected = deviceFilter == null,
                            onClick = { deviceFilter = null },
                            label = { Text(stringResource(R.string.timeline_filter_all)) },
                        )
                        state.devices.forEach { dev ->
                            FilterChip(
                                selected = deviceFilter == dev.id,
                                onClick = { deviceFilter = dev.id },
                                label = { Text(dev.name) },
                            )
                        }
                    }
                }
                when {
                    pagedEntries.isEmpty() && search.isBlank() && kindFilter == null && deviceFilter == null -> Text(
                        stringResource(R.string.timeline_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    pagedEntries.isEmpty() -> Text(
                        stringResource(R.string.timeline_no_match, search.trim().ifEmpty { "…" }),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    else -> {
                        val todayLabel = stringResource(R.string.timeline_today)
                        val yesterdayLabel = stringResource(R.string.timeline_yesterday)
                        LazyColumn(
                            modifier = Modifier.fillMaxWidth().heightIn(max = 360.dp),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            pagedEntries.forEachIndexed { idx, entry ->
                                val dayLabel = timelineDayLabel(entry.timestamp, todayLabel, yesterdayLabel)
                                val prevDayLabel = if (idx > 0) {
                                    timelineDayLabel(pagedEntries[idx - 1].timestamp, todayLabel, yesterdayLabel)
                                } else {
                                    null
                                }
                                if (dayLabel != prevDayLabel) {
                                    item(key = "day_${entry.id}") {
                                        Text(
                                            dayLabel,
                                            style = MaterialTheme.typography.labelMedium,
                                            color = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.padding(top = if (idx > 0) 6.dp else 0.dp),
                                        )
                                    }
                                }
                                item(key = entry.id) {
                                    TimelineRow(
                                        entry = entry,
                                        deviceOnline = state.device(entry.deviceId)?.online == true,
                                        onCopyClip = onCopyClip,
                                        onResend = { onResend(entry) },
                                        onDelete = { onDelete(entry.id) },
                                        loadClipImage = loadClipImage,
                                    )
                                }
                            }
                            if (hasMore) {
                                item(key = "load_more") {
                                    OutlinedButton(
                                        onClick = {
                                            scope.launch {
                                                val trimmed = search.trim().takeIf { it.isNotEmpty() }
                                                val next = queryPage(kindFilter, deviceFilter, trimmed, pagedEntries.size.toUInt())
                                                if (next != null) {
                                                    pagedEntries = pagedEntries + next.items
                                                    hasMore = next.hasMore
                                                }
                                            }
                                        },
                                        modifier = Modifier.fillMaxWidth(),
                                    ) {
                                        Text(stringResource(R.string.timeline_action_load_more))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        dismissButton = {
            if (state.timeline.isNotEmpty()) {
                TextButton(onClick = onClearAll) {
                    Text(stringResource(R.string.timeline_action_clear_all))
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

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TimelineRow(
    entry: TimelineEntry,
    deviceOnline: Boolean,
    onCopyClip: (String) -> Unit,
    onResend: () -> Unit,
    onDelete: () -> Unit,
    loadClipImage: suspend (String) -> ByteArray?,
) {
    val context = LocalContext.current
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
    val meta = buildString {
        append(direction)
        append(" · ")
        append(whenText)
        if (entry.sizeBytes > 0uL) {
            append(" · ")
            append(Formatter.formatShortFileSize(context, entry.sizeBytes.toLong()))
        }
        if (entry.durationSecs > 0uL) {
            append(" · ")
            append(DateUtils.formatElapsedTime(entry.durationSecs.toLong()))
        }
        if (entry.detail.isNotEmpty()) {
            append(" · ")
            append(entry.detail)
        }
    }

    val targetExists = remember(entry.target, entry.kind) {
        when (entry.kind) {
            TimelineKind.LINK -> true
            TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING ->
                entry.target.startsWith("content://") || (entry.target.isNotEmpty() && File(entry.target).exists())
            else -> false
        }
    }
    val canOpen = when (entry.kind) {
        TimelineKind.LINK -> true
        TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> targetExists
        else -> false
    }
    val canShowFolder = when (entry.kind) {
        TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> entry.incoming || targetExists
        else -> false
    }
    val canCopy = when (entry.kind) {
        TimelineKind.CLIP -> entry.clipAvailable && entry.refId != null
        TimelineKind.LINK -> true
        else -> false
    }
    val canSendAgain = deviceOnline && when (entry.kind) {
        TimelineKind.CLIP -> entry.clipAvailable && entry.refId != null
        TimelineKind.LINK -> true
        TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> targetExists
        TimelineKind.SESSION -> false
    }

    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                meta,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                entry.title,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
            if (entry.kind == TimelineKind.CLIP && entry.clipAvailable && entry.refId != null && entry.imageDataUrl != null) {
                val clipId = entry.refId!!
                val bitmap by produceState<ImageBitmap?>(initialValue = null, clipId) {
                    val bytes = loadClipImage(clipId) ?: return@produceState
                    value = withContext(Dispatchers.Default) { previewBitmap(bytes)?.asImageBitmap() }
                }
                bitmap?.let { img ->
                    Image(
                        bitmap = img,
                        contentDescription = stringResource(R.string.clipboard_history_image),
                        modifier = Modifier.fillMaxWidth().heightIn(max = 100.dp),
                        contentScale = ContentScale.Fit,
                    )
                }
            }
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                if (canOpen) {
                    FilledTonalButton(onClick = { openTimelineTarget(context, entry) }) {
                        Text(stringResource(R.string.timeline_action_open))
                    }
                }
                if (canShowFolder) {
                    TextButton(onClick = {
                        runCatching {
                            context.startActivity(
                                Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                            )
                        }
                    }) {
                        Text(stringResource(R.string.timeline_action_folder))
                    }
                }
                if (canCopy) {
                    FilledTonalButton(onClick = {
                        when (entry.kind) {
                            TimelineKind.CLIP -> entry.refId?.let(onCopyClip)
                            TimelineKind.LINK -> PhoneClipboard.write(context, entry.target.ifEmpty { entry.title })
                            else -> Unit
                        }
                    }) {
                        Text(stringResource(R.string.timeline_action_copy))
                    }
                }
                if (canSendAgain) {
                    TextButton(onClick = onResend) {
                        Text(stringResource(R.string.timeline_action_send_again))
                    }
                }
                TextButton(onClick = onDelete) {
                    Text(stringResource(R.string.timeline_action_remove))
                }
            }
        }
    }
}

private fun openTimelineTarget(context: android.content.Context, entry: TimelineEntry) {
    runCatching {
        when (entry.kind) {
            TimelineKind.LINK -> {
                val url = entry.target.ifEmpty { entry.title }
                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> {
                val target = entry.target
                val uri = when {
                    target.startsWith("content://") -> Uri.parse(target)
                    target.isNotEmpty() && File(target).exists() ->
                        FileProvider.getUriForFile(context, "${context.packageName}.files", File(target))
                    else -> return
                }
                val view = Intent(Intent.ACTION_VIEW)
                    .setData(uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            else -> Unit
        }
    }
}

internal fun timelineDayLabel(timestampSecs: Long, todayLabel: String, yesterdayLabel: String): String {
    val nowCal = Calendar.getInstance()
    val itemCal = Calendar.getInstance().apply { timeInMillis = timestampSecs * 1000L }
    val sameYear = nowCal.get(Calendar.YEAR) == itemCal.get(Calendar.YEAR)
    val dayDiff = if (sameYear) nowCal.get(Calendar.DAY_OF_YEAR) - itemCal.get(Calendar.DAY_OF_YEAR) else -1
    return when {
        sameYear && dayDiff == 0 -> todayLabel
        sameYear && dayDiff == 1 -> yesterdayLabel
        else -> DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(timestampSecs * 1000L))
    }
}

