// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.graphics.BitmapFactory
import android.net.Uri
import android.text.format.DateUtils
import android.text.format.Formatter
import android.util.LruCache
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
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
import androidx.compose.ui.unit.isSpecified
import androidx.compose.ui.unit.sp
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.PcFolder
import app.nectarlink.android.core.LocalNetwork
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.core.isFinished
import app.nectarlink.android.files.ReceivedFiles
import app.nectarlink.android.files.transferTitle
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.mirror.MirrorSession
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.recorder.RecorderSession
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.ui.guide.GuideHighlightTarget
import app.nectarlink.android.ui.theme.LocalGlassTheme
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.bentoWell
import app.nectarlink.android.ui.theme.glassModalBlur
import app.nectarlink.android.ui.theme.glassModalContainerColor
import app.nectarlink.android.ui.theme.glassModalScrimColor
import app.nectarlink.android.ui.theme.liquidGlassCard
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.UpdateCard
import app.nectarlink.android.webcam.WebcamSession
import android.app.DownloadManager
import android.content.Intent
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FilterChip
import androidx.compose.runtime.LaunchedEffect
import androidx.core.content.FileProvider
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.core.QueueItemUi
import app.nectarlink.android.core.formatQueueItemTitle
import app.nectarlink.core.AudioOutputDevice
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.ClipboardItemKind
import app.nectarlink.core.DeckState
import app.nectarlink.core.FfiQueueKind
import app.nectarlink.core.FfiQueueStatus
import app.nectarlink.core.Link
import app.nectarlink.core.LocalSendPeer
import app.nectarlink.core.RemoteStorageEntry
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelinePage
import app.nectarlink.core.TimelineKind
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import java.io.File
import java.text.DateFormat
import java.util.Calendar
import java.util.Date
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
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
    onSetReconnect: (pcId: String, on: Boolean) -> Unit,
    onRemote: (pcId: String) -> Unit,
    onDeck: (pcId: String) -> Unit,
    onDesk: (pcId: String) -> Unit = {},
    onRecord: (pcId: String) -> Unit,
    onWebcam: (pcId: String) -> Unit = {},
    updater: AppUpdater,
    onSendFiles: (pcId: String, uris: List<Uri>) -> Unit,
    onSendFolder: (pcId: String, tree: Uri) -> Unit,
    onAccessChanged: () -> Unit,
    onCancelTransfer: (id: String) -> Unit,
    onPauseTransfer: (id: String) -> Unit = {},
    onResumeTransfer: (id: String) -> Unit = {},
    onAcceptTransfer: (id: String) -> Unit = {},
    onCancelQueueItem: (id: String) -> Unit = {},
    onCancelAllQueueItems: () -> Unit = {},
    onRetryQueueItem: (id: String) -> Unit = {},
    onClearFinishedQueueItems: () -> Unit = {},
    onRefreshLocalSend: () -> Unit = {},
    onAllowStorage: (pcId: String) -> Unit = {},
    onDismissStorageRequest: () -> Unit = {},
    onAcceptWebcamRequest: (app.nectarlink.android.webcam.WebcamRequest) -> Unit = {},
    onDismissWebcamRequest: () -> Unit = {},
    onDeclineMicRequest: (pcId: String) -> Unit = {},
    onStartMic: (pcId: String) -> Unit = {},
    onStopMic: () -> Unit = {},
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
    onListPcFiles: (pcId: String, path: String, onResult: (PcFolder) -> Unit) -> Unit = { _, _, onResult ->
        onResult(PcFolder(emptyList()))
    },
    onDownloadPcFile: (pcId: String, remotePath: String, fileName: String, onSaved: (Uri?) -> Unit) -> Unit = { _, _, _, onSaved ->
        onSaved(null)
    },
    mirrorSession: MirrorSession = MirrorSession(),
    webcamSession: WebcamSession = WebcamSession(),
    recorderSession: RecorderSession = RecorderSession(),
    /** The PC this phone's microphone streams to, if any. */
    micStreamingTo: String? = null,
    onStopMirror: () -> Unit = {},
    onOpenWebcam: (pcId: String) -> Unit = {},
    onStopWebcam: () -> Unit = {},
    onOpenRecorder: (pcId: String) -> Unit = {},
    onStopRecorder: () -> Unit = {},
    showWhatsNewGuideBanner: Boolean = false,
    onDismissWhatsNewGuide: () -> Unit = {},
    showHiveInvite: Boolean = false,
    onTryHive: () -> Unit = {},
    onDismissHiveInvite: () -> Unit = {},
    backgroundServiceEnabled: Boolean = true,
    onTurnOnBackgroundService: () -> Unit = {},
    onOpenFeaturesGuide: () -> Unit = {},
    onOpenWalkthrough: () -> Unit = {},
    highlightTarget: GuideHighlightTarget? = null,
    onHighlightConsumed: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var showClipboardHistory by remember { mutableStateOf(false) }
    var showTimeline by remember { mutableStateOf(false) }
    var browseFilesPcId by remember { mutableStateOf<String?>(null) }
    var pulsingTarget by remember { mutableStateOf<GuideHighlightTarget?>(null) }
    // Keep a screen of cards composed on each side, so scrolling back does not rebuild them.
    @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
    val listState = androidx.compose.foundation.lazy.rememberLazyListState(
        cacheWindow = androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow(aheadFraction = 1f, behindFraction = 1f),
    )
    val reducedMotion = LocalReducedMotion.current
    val isGlass = LocalGlassTheme.current
    val anyModalOpen = browseFilesPcId != null || showClipboardHistory || showTimeline

    LaunchedEffect(highlightTarget) {
        val target = highlightTarget ?: return@LaunchedEffect
        pulsingTarget = target
        onHighlightConsumed()
        if (target == GuideHighlightTarget.HOME_CLIPBOARD_HISTORY && state.clipboardHistory.isEmpty() && state.clipboardHistoryEnabled) {
            showClipboardHistory = true
        } else if (target == GuideHighlightTarget.HOME_TIMELINE && state.timeline.isEmpty()) {
            showTimeline = true
        }
        delay(60)
        val targetKey = when (target) {
            GuideHighlightTarget.HOME_PAIR_PC -> if (state.devices.isEmpty()) "no_paired_pc_card" else "home_header"
            GuideHighlightTarget.HOME_PC_CARD,
            GuideHighlightTarget.HOME_PC_AUDIO,
            -> state.devices.firstOrNull()?.id ?: "no_paired_pc_card"
            GuideHighlightTarget.HOME_SEND_QUEUE -> if (state.sendQueue.isNotEmpty()) "send_queue" else (state.devices.firstOrNull()?.id ?: "home_header")
            GuideHighlightTarget.HOME_SETUP_PERMISSIONS -> "permissions_setup_card"
            GuideHighlightTarget.HOME_LOCALSEND -> "localsend_card"
            GuideHighlightTarget.HOME_CLIPBOARD_HISTORY -> "clipboard_history_card"
            GuideHighlightTarget.HOME_TIMELINE -> "timeline_card"
            else -> "home_header"
        }
        val visibleMatch = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.key == targetKey }
        if (visibleMatch != null) {
            runCatching { listState.animateScrollToItem(visibleMatch.index) }
        } else {
            val orderedKeys = buildList {
                add("home_header")
                if (showWhatsNewGuideBanner) add("whats_new_guide_banner")
                if (showHiveInvite && !showWhatsNewGuideBanner) add("hive_invite")
                if (state.ringingFrom != null) add("ringing_banner")
                if (mirrorSession.active) add("mirror_active_banner")
                if (webcamSession.active) add("webcam_active_banner")
                if (recorderSession.active) add("recorder_active_banner")
                if (micStreamingTo != null) add("mic_active_banner")
                state.keyboardFromPc.forEach { add("keyboard_$it") }
                if (state.webcamRequest != null) add("webcam_request")
                if (state.micRequestFrom?.takeIf { it != micStreamingTo } != null) add("mic_request")
                if (state.storageRequestedFrom != null) add("storage_request")
                add("update_card")
                if (!backgroundServiceEnabled) add("background_connection_off")
                if (!state.localNetwork) add("local_network_setup")
                if (state.remoteInputRequested && !state.inputAccess && !state.elevated) add("remote_input_requested")
                if (state.devices.none { !it.isAndroid }) add("no_paired_pc_card")
                state.devices.filter { !it.isAndroid }.forEach { add(it.id) }
                state.devices.filter { it.isAndroid }.forEach { add(it.id) }
                if (state.sendQueue.isNotEmpty()) add("send_queue")
                if (state.devices.isNotEmpty()) {
                    var missingCount = 0
                    if (!state.backgroundUnrestricted) missingCount++
                    if (!state.notificationAccess) missingCount++
                    if (!state.elevated) missingCount++
                    if (!state.inputAccess && !state.elevated) missingCount++
                    if (!state.smsAccess) missingCount++
                    if (!state.callAccess) missingCount++
                    if (!state.contactsAccess) missingCount++
                    if (!state.photoAccess) missingCount++
                    if (missingCount > 0) add("permissions_setup_card")
                }
                if (state.localsendEnabled) add("localsend_card")
                if (state.clipboardHistoryEnabled && state.clipboardHistory.isNotEmpty()) add("clipboard_history_card")
                if (state.timeline.isNotEmpty()) add("timeline_card")
                if (state.transfers.isNotEmpty()) add("transfers_card")
            }
            val targetIndex = orderedKeys.indexOf(targetKey).takeIf { it >= 0 } ?: 0
            runCatching { listState.animateScrollToItem(targetIndex) }
        }
        delay(2200)
        if (pulsingTarget == target) {
            pulsingTarget = null
        }
    }

    LazyColumn(
        state = listState,
        modifier = modifier.glassModalBlur(isGlass && anyModalOpen),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            start = 16.dp,
            top = 16.dp,
            end = 16.dp,
            bottom = 104.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item(key = "home_header", contentType = "header") {
            Row(Modifier.padding(top = 8.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    stringResource(R.string.home_title),
                    style = MaterialTheme.typography.headlineMedium,
                    modifier = Modifier.weight(1f),
                )
                TextButton(
                    onClick = onOpenFeaturesGuide,
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                ) {
                    Text(stringResource(R.string.home_guide_title))
                }
                // Reconnects and syncs with the PCs; spins once to say so.
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
        if (showWhatsNewGuideBanner) {
            item(key = "whats_new_guide_banner", contentType = "banner") {
                WhatsNewFeaturesGuideCard(
                    onOpenGuide = {
                        onDismissWhatsNewGuide()
                        onOpenFeaturesGuide()
                    },
                    onDismiss = onDismissWhatsNewGuide,
                )
            }
        }
        // After the What's new card, one at a time.
        if (showHiveInvite && !showWhatsNewGuideBanner) {
            item(key = "hive_invite", contentType = "banner") {
                HiveInviteCard(onTry = onTryHive, onDismiss = onDismissHiveInvite)
            }
        }
        state.ringingFrom?.let { from ->
            item(key = "ringing_banner", contentType = "banner") {
                RingingBanner(from, onStopRinging)
            }
        }
        if (mirrorSession.active) {
            item(key = "mirror_active_banner", contentType = "banner") {
                val context = LocalContext.current
                val pcName = mirrorSession.pcId?.let { state.nameOf(it) }
                    ?: stringResource(R.string.your_pc)
                val controlReady = state.inputAccess || state.elevated
                val subtitle = when {
                    mirrorSession.soundOnly -> stringResource(R.string.mirror_banner_subtitle_sound)
                    controlReady -> stringResource(R.string.mirror_banner_subtitle_control_on)
                    mirrorSession.audio -> stringResource(R.string.mirror_banner_subtitle_audio)
                    else -> stringResource(R.string.mirror_banner_subtitle)
                }
                LiveStreamingBanner(
                    title = stringResource(
                        if (mirrorSession.soundOnly) R.string.mirror_sharing_sound else R.string.mirror_sharing,
                        pcName,
                    ),
                    subtitle = subtitle,
                    primaryActionLabel = stringResource(R.string.action_stop),
                    onPrimaryAction = onStopMirror,
                    // Nothing to control without the screen.
                    secondaryActionLabel = if (!controlReady && !mirrorSession.soundOnly) {
                        stringResource(R.string.mirror_allow_pc_control)
                    } else {
                        null
                    },
                    onSecondaryAction = if (!controlReady) {
                        { runCatching { context.startActivity(InputService.settingsIntent()) } }
                    } else {
                        null
                    },
                )
            }
        }
        if (webcamSession.active) {
            item(key = "webcam_active_banner", contentType = "banner") {
                val pcName = webcamSession.pcId?.let { state.nameOf(it) }
                    ?: stringResource(R.string.your_pc)
                val cameraLabel = stringResource(
                    if (webcamSession.camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
                )
                LiveStreamingBanner(
                    title = stringResource(R.string.webcam_notification_streaming, pcName),
                    subtitle = stringResource(
                        R.string.webcam_notification_detail,
                        webcamSession.width,
                        webcamSession.height,
                        cameraLabel,
                    ),
                    primaryActionLabel = stringResource(R.string.webcam_stop),
                    onPrimaryAction = onStopWebcam,
                    secondaryActionLabel = if (webcamSession.pcId != null) {
                        stringResource(R.string.action_open_controls)
                    } else {
                        null
                    },
                    onSecondaryAction = webcamSession.pcId?.let { id -> { onOpenWebcam(id) } },
                )
            }
        }
        if (recorderSession.active) {
            item(key = "recorder_active_banner", contentType = "banner") {
                val pcName = recorderSession.pcId?.let { state.nameOf(it) }
                    ?: stringResource(R.string.your_pc)
                val title = if (recorderSession.paused) {
                    stringResource(R.string.recorder_notification_paused, pcName)
                } else {
                    stringResource(R.string.recorder_notification_recording, pcName)
                }
                LiveStreamingBanner(
                    title = title,
                    subtitle = DateUtils.formatElapsedTime(recorderSession.elapsedMs / 1000L),
                    primaryActionLabel = stringResource(R.string.action_stop),
                    onPrimaryAction = onStopRecorder,
                    secondaryActionLabel = if (recorderSession.pcId != null) {
                        stringResource(R.string.timeline_action_open)
                    } else {
                        null
                    },
                    onSecondaryAction = recorderSession.pcId?.let { id -> { onOpenRecorder(id) } },
                )
            }
        }
        micStreamingTo?.let { pcId ->
            item(key = "mic_active_banner", contentType = "banner") {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                LiveStreamingBanner(
                    title = stringResource(R.string.mic_notification_title, pcName),
                    subtitle = stringResource(R.string.mic_notification_text),
                    primaryActionLabel = stringResource(R.string.action_stop),
                    onPrimaryAction = onStopMic,
                )
            }
        }
        state.keyboardFromPc.forEach { pcId ->
            item(key = "keyboard_$pcId", contentType = "banner") {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                KeyboardBanner(pcName, onStop = { onStopKeyboardFromPc(pcId) })
            }
        }
        state.webcamRequest?.let { req ->
            item(key = "webcam_request", contentType = "request") {
                val pcName = state.nameOf(req.pcId) ?: stringResource(R.string.your_pc)
                Surface(
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
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
        state.micRequestFrom?.takeIf { it != micStreamingTo }?.let { pcId ->
            item(key = "mic_request", contentType = "request") {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                val context = LocalContext.current
                Surface(
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text(
                            stringResource(R.string.mic_request_title, pcName),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.mic_request_card_text),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(12.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = {
                                // Asks for the microphone permission if needed, then starts.
                                context.startActivity(
                                    android.content.Intent(context, app.nectarlink.android.mic.MicStartActivity::class.java)
                                        .putExtra(app.nectarlink.android.mic.MicStartActivity.EXTRA_PC, pcId),
                                )
                            }) { Text(stringResource(R.string.mic_request_start)) }
                            OutlinedButton(onClick = { onDeclineMicRequest(pcId) }) {
                                Text(stringResource(R.string.action_not_now))
                            }
                        }
                    }
                }
            }
        }
        state.storageRequestedFrom?.let { pcId ->
            item(key = "storage_request", contentType = "request") {
                val pcName = state.nameOf(pcId) ?: stringResource(R.string.your_pc)
                val context = LocalContext.current
                Surface(
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
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
        item(key = "update_card", contentType = "update") { UpdateCard(updater) }
        if (!backgroundServiceEnabled) {
            item(key = "background_connection_off", contentType = "setup") {
                SetupCard(
                    title = stringResource(R.string.home_background_off_title),
                    text = stringResource(R.string.home_background_off_text),
                    buttonLabel = stringResource(R.string.action_turn_on),
                    onAllow = onTurnOnBackgroundService,
                )
            }
        }
        if (!state.localNetwork) {
            item(key = "local_network_setup", contentType = "setup") {
                val context = LocalContext.current
                val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
                    onAccessChanged()
                    if (!granted) {
                        openAppDetailsSettings(context)
                    }
                }
                SetupCard(
                    stringResource(R.string.local_network_title),
                    stringResource(R.string.local_network_text),
                ) { ask.launch(LocalNetwork.PERMISSION) }
            }
        }
        if (state.remoteInputRequested && !state.inputAccess && !state.elevated) {
            item(key = "remote_input_requested", contentType = "setup") {
                val context = LocalContext.current
                SetupCard(
                    title = stringResource(R.string.remote_input_requested_title),
                    text = stringResource(R.string.remote_input_requested_text),
                ) {
                    runCatching { context.startActivity(InputService.settingsIntent()) }
                }
            }
        }
        if (state.devices.none { !it.isAndroid }) {
            item(key = "no_paired_pc_card", contentType = "empty_pc") {
                NoPairedPcCard(
                    onPairNew = onPairNew,
                    onOpenWalkthrough = onOpenWalkthrough,
                    onOpenFeaturesGuide = onOpenFeaturesGuide,
                    highlighted = pulsingTarget == GuideHighlightTarget.HOME_PAIR_PC ||
                        pulsingTarget == GuideHighlightTarget.HOME_PC_CARD,
                )
            }
        }
        items(state.devices.filter { !it.isAndroid }, key = { it.id }, contentType = { "pc_card" }) { device ->
            Box(
                modifier = Modifier.guideHighlightRing(
                    highlighted = pulsingTarget == GuideHighlightTarget.HOME_PC_CARD ||
                        pulsingTarget == GuideHighlightTarget.HOME_PC_AUDIO,
                ),
            ) {
                PcCard(
                    device = device,
                    ringing = device.id in state.ringingDevices,
                    onRing = onRing,
                    onSendFiles = onSendFiles,
                    onSendFolder = onSendFolder,
                    onPower = onPower,
                    onWake = onWake,
                    onSetReconnect = onSetReconnect,
                    onRemote = onRemote,
                    onDeck = onDeck,
                    onDesk = onDesk,
                    onRecord = onRecord,
                    onWebcam = onWebcam,
                    onSetPcAudio = onSetPcAudio,
                    onBrowsePcFiles = { browseFilesPcId = it },
                    micOn = micStreamingTo == device.id,
                    onToggleMic = { on -> if (on) onStartMic(device.id) else onStopMic() },
                )
            }
        }
        // Another phone or tablet paired with this one.
        items(state.devices.filter { it.isAndroid }, key = { it.id }, contentType = { "peer_card" }) { device ->
            PeerCard(
                device = device,
                ringing = device.id in state.ringingDevices,
                onRing = onRing,
                onSendFiles = onSendFiles,
            )
        }
        if (state.sendQueue.isNotEmpty()) {
            item(key = "send_queue", contentType = "send_queue") {
                Box(
                    modifier = Modifier.guideHighlightRing(
                        highlighted = pulsingTarget == GuideHighlightTarget.HOME_SEND_QUEUE,
                    ),
                ) {
                    SendQueueCard(
                        state = state,
                        onCancel = onCancelQueueItem,
                        onCancelAll = onCancelAllQueueItems,
                        onRetry = onRetryQueueItem,
                        onClearFinished = onClearFinishedQueueItems,
                    )
                }
            }
        }
        if (state.devices.isNotEmpty()) {
            var missingCount = 0
            if (!state.backgroundUnrestricted) missingCount++
            if (!state.notificationAccess) missingCount++
            if (!state.elevated) missingCount++
            if (!state.inputAccess && !state.elevated) missingCount++
            if (!state.smsAccess) missingCount++
            if (!state.callAccess) missingCount++
            if (!state.contactsAccess) missingCount++
            if (!state.photoAccess) missingCount++
            if (missingCount > 0) {
                item(key = "permissions_setup_card", contentType = "permissions") {
                    Box(
                        modifier = Modifier.guideHighlightRing(
                            highlighted = pulsingTarget == GuideHighlightTarget.HOME_SETUP_PERMISSIONS,
                        ),
                    ) {
                        PermissionsSetupCard(
                            state = state,
                            missingCount = missingCount,
                            onAccessChanged = onAccessChanged,
                        )
                    }
                }
            }
        }
        if (state.localsendEnabled) {
            item(key = "localsend_card", contentType = "localsend") {
                Box(
                    modifier = Modifier.guideHighlightRing(
                        highlighted = pulsingTarget == GuideHighlightTarget.HOME_LOCALSEND,
                    ),
                ) {
                    LocalSendCard(
                        peers = state.localsendPeers,
                        onRefresh = onRefreshLocalSend,
                        onSendFiles = onSendFiles,
                    )
                }
            }
        }
        if (state.clipboardHistoryEnabled && state.clipboardHistory.isNotEmpty()) {
            item(key = "clipboard_history_card", contentType = "clipboard") {
                Box(
                    modifier = Modifier.guideHighlightRing(
                        highlighted = pulsingTarget == GuideHighlightTarget.HOME_CLIPBOARD_HISTORY,
                    ),
                ) {
                    ClipboardHistoryCard(
                        entries = state.clipboardHistory,
                        onOpen = { showClipboardHistory = true },
                    )
                }
            }
        }
        if (state.timeline.isNotEmpty()) {
            item(key = "timeline_card", contentType = "timeline") {
                Box(
                    modifier = Modifier.guideHighlightRing(
                        highlighted = pulsingTarget == GuideHighlightTarget.HOME_TIMELINE,
                    ),
                ) {
                    TimelineCard(
                        entries = state.timeline,
                        total = state.timelineTotal,
                        onOpen = { showTimeline = true },
                    )
                }
            }
        }
        if (state.transfers.isNotEmpty()) {
            item(key = "transfers_card", contentType = "transfers") {
                TransfersCard(
                    state = state,
                    onAccept = onAcceptTransfer,
                    onCancel = onCancelTransfer,
                    onPause = onPauseTransfer,
                    onResume = onResumeTransfer,
                )
            }
        }
    }

    val browseDevice = browseFilesPcId?.let { state.device(it) }
    if (browseDevice != null) {
        PcFilesSheet(
            device = browseDevice,
            onListFiles = { path, onResult -> onListPcFiles(browseDevice.id, path, onResult) },
            onDownloadFile = { remotePath, fileName, onSaved ->
                onDownloadPcFile(browseDevice.id, remotePath, fileName, onSaved)
            },
            onDismiss = { browseFilesPcId = null },
        )
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

@Composable
private fun Modifier.guideHighlightRing(highlighted: Boolean): Modifier {
    if (!highlighted) return this
    val ringColor = MaterialTheme.colorScheme.primary
    return this.drawBehind {
        val strokePx = 2.5.dp.toPx()
        val cornerPx = 28.dp.toPx()
        drawRoundRect(
            color = ringColor,
            cornerRadius = androidx.compose.ui.geometry.CornerRadius(cornerPx, cornerPx),
            style = androidx.compose.ui.graphics.drawscope.Stroke(width = strokePx),
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun WhatsNewFeaturesGuideCard(
    onOpenGuide: () -> Unit,
    onDismiss: () -> Unit,
) {
    val isGlass = LocalGlassTheme.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 24.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.secondaryContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = stringResource(R.string.home_whats_new_guide),
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
        Text(
            text = stringResource(R.string.home_guide_card_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onOpenGuide) {
                Text(stringResource(R.string.walkthrough_action_open_guide))
            }
            OutlinedButton(onClick = onDismiss) {
                Text(stringResource(R.string.action_dismiss))
            }
        }
    }
}

/** "Try Hive, the new theme", with a small window onto the comb in Hive's own colors. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun HiveInviteCard(onTry: () -> Unit, onDismiss: () -> Unit) {
    val isGlass = LocalGlassTheme.current
    val dark = MaterialTheme.colorScheme.surface.luminance() < 0.5f
    val reduced = app.nectarlink.android.ui.theme.LocalReducedMotion.current
    val saving = app.nectarlink.android.ui.theme.rememberPowerSaver()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 24.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        androidx.compose.foundation.layout.Box(
            modifier = Modifier
                .size(56.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(if (dark) Color(0xFF121214) else Color(0xFFFFF8F3)),
            contentAlignment = Alignment.Center,
        ) {
            app.nectarlink.android.ui.theme.HiveComb(
                dark = dark,
                animate = !reduced && !saving,
                cellDp = 9f,
                origin = { androidx.compose.ui.geometry.Offset(it.width / 2f, it.height / 2f) },
            )
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                text = stringResource(R.string.hive_invite_title),
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                text = stringResource(R.string.hive_invite_text),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(onClick = onTry) { Text(stringResource(R.string.hive_invite_try)) }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.hive_invite_dismiss)) }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun NoPairedPcCard(
    onPairNew: () -> Unit,
    onOpenWalkthrough: () -> Unit,
    onOpenFeaturesGuide: () -> Unit,
    highlighted: Boolean = false,
) {
    val isGlass = LocalGlassTheme.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .guideHighlightRing(highlighted)
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.primaryContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(22.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(
            text = stringResource(R.string.home_no_pc_paired),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onPrimaryContainer,
        )
        Text(
            text = stringResource(R.string.home_no_pc_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onPrimaryContainer,
        )
        Spacer(Modifier.height(4.dp))
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onPairNew) {
                Text(stringResource(R.string.action_pair_new))
            }
            FilledTonalButton(onClick = onOpenWalkthrough) {
                Text(stringResource(R.string.guide_open_walkthrough))
            }
            OutlinedButton(onClick = onOpenFeaturesGuide) {
                Text(stringResource(R.string.guide_title))
            }
        }
    }
}

private fun openAppDetailsSettings(context: android.content.Context) {
    runCatching {
        context.startActivity(
            Intent(
                android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", context.packageName, null),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

private fun openDeveloperOptions(context: android.content.Context) {
    val devIntent = Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(devIntent) }.isFailure) {
        runCatching {
            context.startActivity(
                Intent(android.provider.Settings.ACTION_SETTINGS)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun LiveStreamingBanner(
    title: String,
    subtitle: String,
    primaryActionLabel: String,
    onPrimaryAction: () -> Unit,
    secondaryActionLabel: String? = null,
    onSecondaryAction: (() -> Unit)? = null,
) {
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.tertiaryContainer,
        contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val dotColor = MaterialTheme.colorScheme.tertiary
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Spacer(
                    modifier = Modifier
                        .size(10.dp)
                        .drawBehind { drawCircle(dotColor) },
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = title,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onTertiaryContainer,
                    )
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onTertiaryContainer.copy(alpha = 0.85f),
                    )
                }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = onPrimaryAction,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.tertiary,
                        contentColor = MaterialTheme.colorScheme.onTertiary,
                    ),
                ) {
                    Text(primaryActionLabel)
                }
                if (secondaryActionLabel != null && onSecondaryAction != null) {
                    OutlinedButton(
                        onClick = onSecondaryAction,
                        colors = ButtonDefaults.outlinedButtonColors(
                            contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
                        ),
                        border = BorderStroke(
                            1.dp,
                            MaterialTheme.colorScheme.onTertiaryContainer.copy(alpha = 0.4f),
                        ),
                    ) {
                        Text(secondaryActionLabel)
                    }
                }
            }
        }
    }
}

@Composable
private fun PermissionsSetupCard(
    state: CoreState,
    missingCount: Int,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val isGlass = LocalGlassTheme.current
    var expanded by remember { mutableStateOf(false) }
    val reducedMotion = LocalReducedMotion.current
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(durationMillis = if (reducedMotion) 0 else 200),
        label = "PermissionsChevron",
    )

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(20.dp),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button) { expanded = !expanded },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                Text(
                    text = stringResource(R.string.permissions_setup_title, missingCount),
                    style = MaterialTheme.typography.titleMedium,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    text = stringResource(R.string.permissions_setup_subtitle),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                imageVector = ChevronDownIcon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier
                    .size(20.dp)
                    .graphicsLayer { rotationZ = chevronRotation },
            )
        }
        AnimatedVisibility(
            visible = expanded,
            enter = expandVertically(tween(if (reducedMotion) 0 else 220)) + fadeIn(tween(if (reducedMotion) 0 else 180)),
            exit = shrinkVertically(tween(if (reducedMotion) 0 else 180)) + fadeOut(tween(if (reducedMotion) 0 else 140)),
        ) {
            val smsLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
                onAccessChanged()
                if (grants.values.none { it }) openAppDetailsSettings(context)
            }
            val callsLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
                onAccessChanged()
                if (grants.values.none { it }) openAppDetailsSettings(context)
            }
            val contactsLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
                onAccessChanged()
                if (grants.values.none { it }) openAppDetailsSettings(context)
            }
            val photosLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
                onAccessChanged()
                if (grants.values.none { it }) openAppDetailsSettings(context)
            }
            Column(
                modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (!state.backgroundUnrestricted) {
                    val oemGuide = BackgroundAccess.oemGuideResId()?.let { stringResource(it) }
                    SetupCard(
                        title = stringResource(R.string.background_title),
                        text = stringResource(R.string.background_text),
                        guide = oemGuide,
                        secondaryLabel = stringResource(R.string.battery_open_oem_settings),
                        onSecondary = {
                            runCatching { context.startActivity(BackgroundAccess.oemSettingsIntent(context)) }
                        },
                    ) {
                        runCatching { context.startActivity(BackgroundAccess.requestIntent(context)) }
                    }
                }
                if (!state.notificationAccess) {
                    SetupCard(
                        stringResource(R.string.notifications_title),
                        stringResource(R.string.notifications_off),
                    ) {
                        runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }
                    }
                }
                if (!state.elevated) {
                    SetupCard(
                        title = stringResource(R.string.home_background_clipboard_title),
                        text = stringResource(R.string.home_background_clipboard_body),
                        secondaryLabel = stringResource(R.string.elevated_open_developer),
                        onSecondary = { openDeveloperOptions(context) },
                    ) {
                        scope.launch {
                            app.nectarlink.android.elevated.PairingNotification.show(context)
                            if (!app.nectarlink.android.elevated.Elevated.start()) {
                                openDeveloperOptions(context)
                            }
                        }
                    }
                }
                if (!state.inputAccess && !state.elevated) {
                    SetupCard(
                        stringResource(R.string.control_title),
                        stringResource(R.string.control_off),
                    ) {
                        runCatching { context.startActivity(InputService.settingsIntent()) }
                    }
                }
                if (!state.smsAccess) {
                    SetupCard(
                        stringResource(R.string.sms_title),
                        stringResource(R.string.sms_off),
                    ) { smsLauncher.launch(PhoneSms.permissions) }
                }
                if (!state.callAccess) {
                    SetupCard(
                        stringResource(R.string.calls_title),
                        stringResource(R.string.calls_off),
                    ) { callsLauncher.launch(PhoneCalls.permissions) }
                }
                if (!state.contactsAccess) {
                    SetupCard(
                        stringResource(R.string.contacts_title),
                        stringResource(R.string.contacts_off),
                    ) { contactsLauncher.launch(PhoneContacts.permissions) }
                }
                if (!state.photoAccess) {
                    SetupCard(
                        stringResource(R.string.photos_title),
                        stringResource(R.string.photos_off),
                    ) { photosLauncher.launch(RecentPhotos.permissions) }
                }
            }
        }
    }
}

/** Something the phone needs before mirroring works, with the button that fixes it. */
@Composable
private fun SetupCard(
    title: String,
    text: String,
    guide: String? = null,
    buttonLabel: String = stringResource(R.string.action_allow),
    secondaryLabel: String? = null,
    onSecondary: (() -> Unit)? = null,
    onAllow: () -> Unit,
) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
            Spacer(Modifier.height(4.dp))
            Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
            if (!guide.isNullOrBlank()) {
                Spacer(Modifier.height(8.dp))
                Text(
                    guide,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
            }
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = onAllow) { Text(buttonLabel) }
                if (secondaryLabel != null && onSecondary != null) {
                    OutlinedButton(onClick = onSecondary) { Text(secondaryLabel) }
                }
            }
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
private fun PeerCard(
    device: Device,
    ringing: Boolean,
    onRing: (String, Boolean) -> Unit,
    onSendFiles: (String, List<Uri>) -> Unit,
) {
    val context = LocalContext.current
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(device.id, uris)
    }
    val isGlass = LocalGlassTheme.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(22.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(device.name, style = MaterialTheme.typography.titleLarge)
                Text(
                    text = stringResource(
                        if (device.kind == app.nectarlink.core.DeviceKind.TABLET) R.string.peer_kind_tablet else R.string.peer_kind_phone,
                    ) + " · " + linkText(device.link),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            device.battery?.let { battery ->
                Text(
                    text = if (battery.charging) stringResource(R.string.battery_charging, battery.level.toInt())
                    else stringResource(R.string.battery_level, battery.level.toInt()),
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilledTonalButton(
                contentPadding = CardButtonPadding,
                enabled = device.online && device.has("pair.ring"),
                onClick = { onRing(device.id, !ringing) },
                modifier = Modifier.weight(1f),
            ) {
                Icon(imageVector = BellIcon, contentDescription = null, modifier = Modifier.size(15.dp))
                Spacer(Modifier.width(6.dp))
                ButtonLabel(stringResource(if (ringing) R.string.action_stop_ringing else R.string.action_ring_device))
            }
            FilledTonalButton(
                contentPadding = CardButtonPadding,
                enabled = !device.online || device.has("pair.files"),
                onClick = { pickFiles.launch(arrayOf("*/*")) },
                modifier = Modifier.weight(1f),
            ) {
                Icon(imageVector = SendArrowIcon, contentDescription = null, modifier = Modifier.size(15.dp))
                Spacer(Modifier.width(6.dp))
                ButtonLabel(stringResource(R.string.action_send_files))
            }
        }
        FilledTonalButton(
            contentPadding = CardButtonPadding,
            enabled = !device.online || device.has("pair.clipboard"),
            onClick = { context.startActivity(SendActivity.sendClipboardIntent(context, device.id)) },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(imageVector = ClipboardIcon, contentDescription = null, modifier = Modifier.size(15.dp))
            Spacer(Modifier.width(6.dp))
            ButtonLabel(stringResource(R.string.action_send_clipboard))
        }
    }
}

@Composable
private fun PcCard(
    device: Device,
    ringing: Boolean,
    onRing: (String, Boolean) -> Unit,
    onSendFiles: (String, List<Uri>) -> Unit,
    onSendFolder: (String, Uri) -> Unit,
    onPower: (String, Boolean) -> Unit,
    onWake: (String) -> Unit,
    onSetReconnect: (String, Boolean) -> Unit,
    onRemote: (String) -> Unit,
    onDeck: (String) -> Unit,
    onRecord: (String) -> Unit,
    onWebcam: (String) -> Unit,
    onSetPcAudio: (String, Int?, Boolean?) -> Unit,
    onBrowsePcFiles: (String) -> Unit = {},
    onDesk: (String) -> Unit = {},
    /** Whether this phone's microphone streams to this PC now. */
    micOn: Boolean = false,
    onToggleMic: (Boolean) -> Unit = {},
) {
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(device.id, uris)
    }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) onSendFolder(device.id, tree)
    }
    val isGlass = LocalGlassTheme.current
    androidx.compose.runtime.CompositionLocalProvider(
        androidx.compose.material3.LocalContentColor provides MaterialTheme.colorScheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 28.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.primaryContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(22.dp),
        ) {
            val context = LocalContext.current
            val ink = MaterialTheme.colorScheme.onPrimaryContainer
            val primaryBtn = ButtonDefaults.buttonColors(
                disabledContainerColor = ink.copy(alpha = 0.08f),
                disabledContentColor = ink.copy(alpha = 0.38f),
            )
            val tonal = ButtonDefaults.filledTonalButtonColors(
                containerColor = ink.copy(alpha = if (isGlass) 0.14f else 0.10f),
                contentColor = ink,
                disabledContainerColor = ink.copy(alpha = 0.05f),
                disabledContentColor = ink.copy(alpha = 0.38f),
            )
            val outlined = ButtonDefaults.outlinedButtonColors(contentColor = ink, disabledContentColor = ink.copy(alpha = 0.38f))
            val outline = BorderStroke(1.dp, ink.copy(alpha = if (device.online) 0.45f else 0.15f))
            val statusDotColor = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Spacer(
                    modifier = Modifier
                        .size(10.dp)
                        .padding(1.dp)
                        .drawBehind { drawCircle(statusDotColor) },
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    if (!device.online && device.wakeState == WakeState.Waking) {
                        stringResource(R.string.wake_waking, device.name)
                    } else if (!device.online && !device.reconnect) {
                        stringResource(R.string.link_not_reconnecting)
                    } else {
                        linkText(device.link)
                    },
                    style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.weight(1f),
                )
                if (!device.online && !device.reconnect) {
                    Spacer(Modifier.width(8.dp))
                    FilledTonalButton(
                        colors = tonal,
                        onClick = { onSetReconnect(device.id, true) },
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp, vertical = 4.dp),
                        modifier = Modifier.heightIn(min = 32.dp),
                    ) {
                        Text(
                            stringResource(R.string.action_reconnect),
                            style = MaterialTheme.typography.labelMedium,
                            maxLines = 1,
                        )
                    }
                } else if (!device.online && device.canWake && device.wakeState != WakeState.Waking) {
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
                // Stays paired, and stays disconnected until Reconnect (GitHub #48).
                // At the end of the status line, outlined so it reads as a button.
                if (device.reconnect && device.wakeState != WakeState.Waking) {
                    Spacer(Modifier.width(8.dp))
                    OutlinedButton(
                        onClick = { onSetReconnect(device.id, false) },
                        border = BorderStroke(1.dp, ink.copy(alpha = 0.35f)),
                        colors = ButtonDefaults.outlinedButtonColors(contentColor = ink),
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp, vertical = 4.dp),
                        modifier = Modifier.heightIn(min = 32.dp),
                    ) {
                        Text(
                            stringResource(if (device.online) R.string.action_disconnect else R.string.action_stop_reconnecting),
                            style = MaterialTheme.typography.labelMedium,
                            maxLines = 1,
                        )
                    }
                }
            }
            Spacer(Modifier.height(10.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(device.name, style = MaterialTheme.typography.headlineLarge)
                    device.model?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 2.dp))
                    }
                }
                device.battery?.let { battery ->
                    Text(
                        text = if (battery.charging) stringResource(R.string.battery_charging, battery.level.toInt())
                        else stringResource(R.string.battery_level, battery.level.toInt()),
                        style = MaterialTheme.typography.labelMedium,
                        color = ink,
                        modifier = Modifier
                            .bentoWell(
                                cornerRadius = 14.dp,
                                fillColor = ink.copy(alpha = 0.10f),
                            )
                            .padding(horizontal = 10.dp, vertical = 5.dp),
                    )
                }
            }
            Spacer(Modifier.height(16.dp))
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                // 1. Primary Workspace Bento Grid (2x2): Control PC, Deck, Desk Mode, Webcam
                Text(
                    text = stringResource(R.string.home_section_screen),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.68f),
                    modifier = Modifier.padding(start = 2.dp),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Button(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = primaryBtn,
                        onClick = { onRemote(device.id) },
                        modifier = Modifier.weight(1f).heightIn(min = 46.dp),
                    ) {
                        Icon(
                            imageVector = RemotePcIcon,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(R.string.action_remote_pc))
                    }
                    FilledTonalButton(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onDeck(device.id) },
                        modifier = Modifier.weight(1f).heightIn(min = 46.dp),
                    ) {
                        Icon(
                            imageVector = DeckGridIcon,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(R.string.action_deck_pc))
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onDesk(device.id) },
                        modifier = Modifier.weight(1f).heightIn(min = 44.dp),
                    ) {
                        Icon(
                            imageVector = DeskClockIcon,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(R.string.action_desk_pc))
                    }
                    FilledTonalButton(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onWebcam(device.id) },
                        modifier = Modifier.weight(1f).heightIn(min = 44.dp),
                    ) {
                        Icon(
                            imageVector = CameraLensIcon,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(R.string.action_webcam_pc))
                    }
                }

                // 2. Files & Clipboard Bento Strip (Always visible, no hidden expander)
                Text(
                    text = stringResource(R.string.home_section_transfer),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.68f),
                    modifier = Modifier.padding(start = 2.dp, top = 2.dp),
                )
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .bentoWell(
                            cornerRadius = 20.dp,
                            fillColor = ink.copy(alpha = if (isGlass) 0.11f else 0.07f),
                            borderColor = if (isGlass) ink.copy(alpha = 0.16f) else Color.Transparent,
                        )
                        .padding(10.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalButton(
                            contentPadding = CardButtonPadding,
                            enabled = if (device.online) device.has("files.send") else true,
                            colors = tonal,
                            onClick = { pickFiles.launch(arrayOf("*/*")) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                imageVector = SendArrowIcon,
                                contentDescription = null,
                                modifier = Modifier.size(15.dp),
                            )
                            Spacer(Modifier.width(6.dp))
                            ButtonLabel(stringResource(R.string.action_send_files))
                        }
                        FilledTonalButton(
                            contentPadding = CardButtonPadding,
                            enabled = if (device.online) device.has("files.send") else true,
                            colors = tonal,
                            onClick = { pickFolder.launch(null) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                imageVector = FolderIcon,
                                contentDescription = null,
                                modifier = Modifier.size(15.dp),
                            )
                            Spacer(Modifier.width(6.dp))
                            ButtonLabel(stringResource(R.string.action_send_folder))
                        }
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalButton(
                            contentPadding = CardButtonPadding,
                            // Offline, the clip waits in the send queue.
                            enabled = if (device.online) device.usable("clipboard.auto_phone_to_pc") else true,
                            colors = tonal,
                            onClick = { context.startActivity(SendActivity.sendClipboardIntent(context, device.id)) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                imageVector = ClipboardIcon,
                                contentDescription = null,
                                modifier = Modifier.size(15.dp),
                            )
                            Spacer(Modifier.width(6.dp))
                            ButtonLabel(stringResource(R.string.action_send_clipboard))
                        }
                        FilledTonalButton(
                            contentPadding = CardButtonPadding,
                            enabled = device.online,
                            colors = tonal,
                            onClick = { onBrowsePcFiles(device.id) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                imageVector = PcFolderIcon,
                                contentDescription = null,
                                modifier = Modifier.size(15.dp),
                            )
                            Spacer(Modifier.width(6.dp))
                            ButtonLabel(stringResource(R.string.action_browse_pc_files))
                        }
                    }
                }

                // 3. Live PC media, master volume, mute, and active output device bar
                val deckState = device.deckState
                if (device.online) {
                    PcAudioCardSection(
                        pcId = device.id,
                        pcName = device.name,
                        deckState = deckState,
                        ink = ink,
                        onSetPcAudio = { vol, muted -> onSetPcAudio(device.id, vol, muted) },
                        micAvailable = micOn || device.has("media.phone_mic"),
                        micOn = micOn,
                        onToggleMic = onToggleMic,
                    )
                }

                // 4. Utilities & Power Row (Voice Memo, Ring PC, Lock, Sleep)
                Text(
                    text = stringResource(R.string.home_section_system),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.68f),
                    modifier = Modifier.padding(start = 2.dp, top = 2.dp),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onRecord(device.id) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(
                            imageVector = MicIcon,
                            contentDescription = null,
                            modifier = Modifier.size(15.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(R.string.action_record_pc))
                    }
                    FilledTonalButton(
                        contentPadding = CardButtonPadding,
                        enabled = device.online,
                        colors = tonal,
                        onClick = { onRing(device.id, !ringing) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(
                            imageVector = BellIcon,
                            contentDescription = null,
                            modifier = Modifier.size(15.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        ButtonLabel(stringResource(if (ringing) R.string.action_stop_ringing else R.string.action_ring_pc))
                    }
                }
                if (device.has("device.pc_actions")) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        OutlinedButton(
                            contentPadding = CardButtonPadding,
                            enabled = device.online,
                            colors = outlined,
                            border = outline,
                            onClick = { onPower(device.id, false) },
                            modifier = Modifier.weight(1f),
                        ) {
                            ButtonLabel(stringResource(R.string.action_lock_pc))
                        }
                        OutlinedButton(
                            contentPadding = CardButtonPadding,
                            enabled = device.online,
                            colors = outlined,
                            border = outline,
                            onClick = { onPower(device.id, true) },
                            modifier = Modifier.weight(1f),
                        ) {
                            ButtonLabel(stringResource(R.string.action_sleep_pc))
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
                Column(
                    Modifier
                        .fillMaxWidth()
                        .bentoWell(
                            cornerRadius = 20.dp,
                            fillColor = ink.copy(alpha = 0.08f),
                        )
                        .padding(16.dp),
                ) {
                    Text(
                        stringResource(R.string.wake_timeout_title, device.name),
                        style = MaterialTheme.typography.titleSmall,
                        color = ink,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        stringResource(R.string.wake_timeout_text, device.name),
                        style = MaterialTheme.typography.bodySmall,
                        color = ink,
                    )
                }
            }
        }
    }
}

@Composable
private fun PcAudioCardSection(
    pcId: String,
    pcName: String,
    deckState: DeckState?,
    ink: Color,
    onSetPcAudio: (volume: Int?, muted: Boolean?) -> Unit,
    micAvailable: Boolean = false,
    micOn: Boolean = false,
    onToggleMic: (Boolean) -> Unit = {},
) {
    val context = LocalContext.current
    val pcMedia = remember(context) {
        (context.applicationContext as? NectarlinkApplication)?.core?.pcMedia
    }
    val mediaSummary = if (pcMedia != null) {
        val summaries by pcMedia.summaries.collectAsStateWithLifecycle()
        summaries[pcId]
    } else {
        null
    }
    if (mediaSummary == null && deckState == null) return

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .bentoWell(
                cornerRadius = 20.dp,
                fillColor = ink.copy(alpha = 0.06f),
            )
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (mediaSummary != null) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .bentoWell(
                        cornerRadius = 16.dp,
                        fillColor = ink.copy(alpha = 0.10f),
                    )
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = mediaSummary.title,
                        style = MaterialTheme.typography.labelLarge,
                        color = ink,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (mediaSummary.subtitle.isNotBlank()) {
                        Text(
                            text = mediaSummary.subtitle,
                            style = MaterialTheme.typography.labelSmall,
                            color = ink.copy(alpha = 0.78f),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                IconButton(
                    onClick = {
                        pcMedia?.command(
                            pcId,
                            mediaSummary.playerId,
                            if (mediaSummary.playing) app.nectarlink.core.MediaAction.PAUSE
                            else app.nectarlink.core.MediaAction.PLAY,
                        )
                    },
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        painter = painterResource(if (mediaSummary.playing) R.drawable.ic_pause else R.drawable.ic_play),
                        contentDescription = stringResource(
                            if (mediaSummary.playing) R.string.media_pause else R.string.media_play,
                        ),
                        tint = ink,
                        modifier = Modifier.size(18.dp),
                    )
                }
                IconButton(
                    onClick = { pcMedia?.command(pcId, mediaSummary.playerId, app.nectarlink.core.MediaAction.NEXT) },
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ic_skip_next),
                        contentDescription = stringResource(R.string.media_next),
                        tint = ink,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }
        }
        if (deckState == null) return@Column
        var draggingVolume by remember { mutableStateOf<Float?>(null) }
        var lastSentVolume by remember { mutableStateOf(deckState.volume.toInt()) }
        if (draggingVolume == null) {
            lastSentVolume = deckState.volume.toInt()
        }
        var showOutputs by remember { mutableStateOf(false) }
        val shownVolume = draggingVolume?.roundToInt() ?: deckState.volume.toInt()
        val (streamingToPhone, hardwareOutputs, defaultOutput) = remember(deckState.outputDevices) {
            val streaming = deckState.outputDevices.any { it.id == "nectarlink:phone_speaker" }
            val hw = deckState.outputDevices.filterNot { it.id == "nectarlink:phone_speaker" }
            val def = hw.firstOrNull { it.isDefault } ?: hw.firstOrNull()
            Triple(streaming, hw, def)
        }
        val volumeLabel = stringResource(R.string.pc_audio_volume)

        Row(
            modifier = Modifier
                .fillMaxWidth()
                .bentoWell(
                    cornerRadius = 16.dp,
                    fillColor = ink.copy(alpha = if (streamingToPhone) 0.14f else 0.06f),
                )
                .clickable(role = Role.Switch) {
                    onSetPcAudio(if (!streamingToPhone) 201 else 200, null)
                }
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Icon(
                painter = painterResource(R.drawable.ic_speaker),
                contentDescription = null,
                tint = ink.copy(alpha = 0.9f),
                modifier = Modifier.size(20.dp),
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.pc_audio_use_phone_speaker),
                    style = MaterialTheme.typography.labelLarge,
                    color = ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = stringResource(
                        if (streamingToPhone) {
                            R.string.pc_audio_use_phone_speaker_on
                        } else {
                            R.string.pc_audio_use_phone_speaker_sub
                        }
                    ),
                    style = MaterialTheme.typography.labelSmall,
                    color = ink.copy(alpha = 0.78f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Switch(
                checked = streamingToPhone,
                onCheckedChange = null,
            )
        }
        if (micAvailable) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .bentoWell(
                        cornerRadius = 16.dp,
                        fillColor = ink.copy(alpha = if (micOn) 0.14f else 0.06f),
                    )
                    .clickable(role = Role.Switch) { onToggleMic(!micOn) }
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Icon(
                    imageVector = MicIcon,
                    contentDescription = null,
                    tint = ink.copy(alpha = 0.9f),
                    modifier = Modifier.size(20.dp),
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = stringResource(R.string.pc_audio_use_phone_mic),
                        style = MaterialTheme.typography.labelLarge,
                        color = ink,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = stringResource(
                            if (micOn) R.string.pc_audio_use_phone_mic_on else R.string.pc_audio_use_phone_mic_sub,
                        ),
                        style = MaterialTheme.typography.labelSmall,
                        color = ink.copy(alpha = 0.78f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Switch(
                    checked = micOn,
                    onCheckedChange = null,
                )
            }
        }
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
                    val rounded = v.roundToInt()
                    if (rounded != lastSentVolume) {
                        lastSentVolume = rounded
                        onSetPcAudio(rounded, null)
                    }
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
                color = ink,
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
                if (hardwareOutputs.size > 1) {
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = "${hardwareOutputs.size}",
                        style = MaterialTheme.typography.labelSmall,
                        color = ink.copy(alpha = 0.85f),
                    )
                }
            }
        }
        if (showOutputs && hardwareOutputs.isNotEmpty()) {
            PcAudioOutputsDialog(
                pcName = pcName,
                devices = hardwareOutputs,
                onPick = { output ->
                    (context.applicationContext as? NectarlinkApplication)?.core?.setPcAudioOutput(pcId, output)
                    showOutputs = false
                },
                onDismiss = { showOutputs = false },
            )
        }
    }
}

@Composable
internal fun PcAudioOutputsDialog(
    pcName: String,
    devices: List<AudioOutputDevice>,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = glassModalContainerColor(),
        title = { Text(stringResource(R.string.pc_audio_outputs_title, pcName)) },
        text = {
            Column(
                verticalArrangement = Arrangement.spacedBy(10.dp),
                modifier = Modifier.verticalScroll(rememberScrollState()),
            ) {
                Text(
                    stringResource(R.string.pc_audio_outputs_hint, pcName),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                devices.forEach { out ->
                    Surface(
                        onClick = { if (!out.isDefault) onPick(out.id) },
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
    var revealLatest by remember(latest.id) { mutableStateOf(false) }
    val isGlass = LocalGlassTheme.current
    val isText = latest.kind == ClipboardItemKind.TEXT
    val preview = when (latest.kind) {
        ClipboardItemKind.TEXT -> if (revealLatest) latest.text else stringResource(R.string.clipboard_preview_hidden)
        ClipboardItemKind.IMAGE -> stringResource(R.string.clipboard_history_image)
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .clickable(role = Role.Button, onClick = onOpen)
            .padding(20.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                stringResource(R.string.clipboard_history_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            if (isText) {
                TextButton(
                    onClick = { revealLatest = !revealLatest },
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                    modifier = Modifier.heightIn(min = 28.dp),
                ) {
                    Text(
                        stringResource(
                            if (revealLatest) R.string.clipboard_preview_hide else R.string.clipboard_preview_reveal,
                        ),
                        style = MaterialTheme.typography.labelMedium,
                    )
                }
            }
        }
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
        containerColor = glassModalContainerColor(),
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
                    val bitmap by produceState<ImageBitmap?>(initialValue = previewBitmapCache.get(entry.id), entry.id) {
                        if (value != null) return@produceState
                        val bytes = loadImage(entry.id) ?: return@produceState
                        val decoded = withContext(Dispatchers.Default) { previewBitmap(bytes)?.asImageBitmap() }
                        if (decoded != null) previewBitmapCache.put(entry.id, decoded)
                        value = decoded
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

private val previewBitmapCache = LruCache<String, ImageBitmap>(24)

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
    val isGlass = LocalGlassTheme.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(20.dp),
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

@Composable
private fun SendQueueCard(
    state: CoreState,
    onCancel: (String) -> Unit,
    onCancelAll: () -> Unit,
    onRetry: (String) -> Unit,
    onClearFinished: () -> Unit,
) {
    val items = state.sendQueue
    if (items.isEmpty()) return
    val isGlass = LocalGlassTheme.current
    val hasActive = items.any { it.status is FfiQueueStatus.Waiting || it.status is FfiQueueStatus.Sending }
    val hasFinished = items.any { it.status is FfiQueueStatus.Failed || it.status is FfiQueueStatus.Expired }
    val showPeerChip = state.devices.size > 1

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(20.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = stringResource(R.string.send_queue_title_count, items.size),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            if (hasActive) {
                TextButton(
                    onClick = onCancelAll,
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                    modifier = Modifier.heightIn(min = 28.dp),
                ) {
                    Text(
                        stringResource(R.string.send_queue_cancel_all),
                        style = MaterialTheme.typography.labelMedium,
                    )
                }
            }
            if (hasFinished) {
                TextButton(
                    onClick = onClearFinished,
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                    modifier = Modifier.heightIn(min = 28.dp),
                ) {
                    Text(
                        stringResource(R.string.send_queue_clear),
                        style = MaterialTheme.typography.labelMedium,
                    )
                }
            }
        }
        Spacer(Modifier.height(6.dp))
        items.forEachIndexed { index, item ->
            if (index > 0) {
                androidx.compose.material3.HorizontalDivider(
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f),
                )
            }
            SendQueueRow(
                item = item,
                transfers = state.transfers,
                showPeerChip = showPeerChip,
                onCancel = { onCancel(item.id) },
                onRetry = { onRetry(item.id) },
            )
        }
    }
}

@Composable
private fun SendQueueRow(
    item: QueueItemUi,
    transfers: List<Transfer>,
    showPeerChip: Boolean = false,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
) {
    val isFileKind = item.kind == FfiQueueKind.FILES ||
        item.kind == FfiQueueKind.FOLDER ||
        item.kind == FfiQueueKind.HANDOFF_DOCUMENT ||
        item.kind == FfiQueueKind.RECORDING
    val kindIcon = when (item.kind) {
        FfiQueueKind.CLIPBOARD, FfiQueueKind.CLIPBOARD_IMAGE -> ClipboardIcon
        FfiQueueKind.LINK -> LinkChainIcon
        FfiQueueKind.FILES,
        FfiQueueKind.FOLDER,
        FfiQueueKind.HANDOFF_DOCUMENT,
        FfiQueueKind.RECORDING,
        FfiQueueKind.TASK_NOTIFY,
        FfiQueueKind.CAMERA_REQUEST -> SendArrowIcon
    }
    val status = item.status
    val targetId = item.activeTransferId ?: item.id
    val activeTransfer = if (status is FfiQueueStatus.Sending && isFileKind) {
        transfers.firstOrNull {
            it.id == targetId &&
                it.direction == TransferDirection.OUTGOING &&
                (it.status is TransferStatus.Running || it.status is TransferStatus.Waiting)
        } ?: transfers.firstOrNull {
            it.deviceId == item.peerId &&
                it.direction == TransferDirection.OUTGOING &&
                (it.status is TransferStatus.Running || it.status is TransferStatus.Waiting)
        }
    } else {
        null
    }
    val progressDone = activeTransfer?.done?.toLong() ?: item.bytesDone
    val progressTotal = activeTransfer?.total?.toLong()?.takeIf { it > 0L } ?: item.totalBytes
    val progressPercent = if (status is FfiQueueStatus.Sending && isFileKind && progressTotal > 0L) {
        ((progressDone.toDouble() / progressTotal.toDouble()) * 100.0).roundToInt().coerceIn(0, 100)
    } else {
        null
    }
    val queueFallback = stringResource(queueKindLabel(item.kind))
    val displayTitle = remember(item.title, queueFallback) { formatQueueItemTitle(item.title, queueFallback) }
    val statusText = when (status) {
        is FfiQueueStatus.Waiting -> stringResource(R.string.send_queue_waiting_for, item.peerName)
        is FfiQueueStatus.Sending -> if (progressPercent != null) {
            stringResource(R.string.send_queue_sending_progress, progressPercent)
        } else {
            stringResource(R.string.send_queue_sending)
        }
        is FfiQueueStatus.Failed -> stringResource(R.string.send_queue_failed_reason, status.error)
        is FfiQueueStatus.Expired -> stringResource(R.string.send_queue_expired_reason, status.error)
    }
    val statusColor = when (status) {
        is FfiQueueStatus.Failed -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Icon(
                imageVector = kindIcon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(18.dp),
            )
            Column(modifier = Modifier.weight(1f)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        text = displayTitle,
                        style = MaterialTheme.typography.titleSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    if (showPeerChip && item.peerName.isNotBlank()) {
                        Text(
                            text = item.peerName,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier
                                .bentoWell(
                                    cornerRadius = 8.dp,
                                    fillColor = MaterialTheme.colorScheme.secondaryContainer,
                                )
                                .padding(horizontal = 6.dp, vertical = 2.dp),
                        )
                    }
                }
                Spacer(Modifier.height(2.dp))
                Text(
                    text = statusText,
                    style = MaterialTheme.typography.bodySmall,
                    color = statusColor,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            when (status) {
                is FfiQueueStatus.Waiting, is FfiQueueStatus.Sending -> {
                    IconButton(
                        onClick = onCancel,
                        modifier = Modifier.size(32.dp),
                    ) {
                        Icon(
                            imageVector = CloseQueueIcon,
                            contentDescription = stringResource(R.string.action_cancel),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
                is FfiQueueStatus.Failed -> {
                    TextButton(onClick = onRetry) {
                        Text(stringResource(R.string.send_queue_retry))
                    }
                    TextButton(onClick = onCancel) {
                        Text(stringResource(R.string.send_queue_dismiss))
                    }
                }
                is FfiQueueStatus.Expired -> {
                    TextButton(onClick = onCancel) {
                        Text(stringResource(R.string.send_queue_dismiss))
                    }
                }
            }
        }
        if (status is FfiQueueStatus.Sending && isFileKind) {
            val reducedMotion = LocalReducedMotion.current
            if (progressPercent != null) {
                LinearProgressIndicator(
                    progress = { progressPercent / 100f },
                    modifier = Modifier.fillMaxWidth(),
                )
            } else if (reducedMotion) {
                LinearProgressIndicator(
                    progress = { 0.5f },
                    modifier = Modifier.fillMaxWidth(),
                )
            } else {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
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
    onPause: (String) -> Unit = {},
    onResume: (String) -> Unit = {},
) {
    val isGlass = LocalGlassTheme.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(stringResource(R.string.transfers_title), style = MaterialTheme.typography.titleMedium)
        state.transfers.forEach { transfer ->
            TransferRow(
                transfer = transfer,
                pc = state.nameOf(transfer.deviceId).orEmpty(),
                onAccept = onAccept,
                onCancel = onCancel,
                onPause = onPause,
                onResume = onResume,
            )
        }
    }
}

@Composable
private fun TransferRow(
    transfer: Transfer,
    pc: String,
    onAccept: (String) -> Unit,
    onCancel: (String) -> Unit,
    onPause: (String) -> Unit = {},
    onResume: (String) -> Unit = {},
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
        is TransferStatus.Paused ->
            stringResource(
                R.string.transfer_paused_progress,
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
            "integrity" -> stringResource(R.string.transfer_failed_integrity)
            else -> stringResource(R.string.transfer_failed)
        }
    }
    Column(modifier = Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (status is TransferStatus.Running || status is TransferStatus.Waiting || status is TransferStatus.Paused) {
                    Spacer(Modifier.height(6.dp))
                    val reducedMotion = LocalReducedMotion.current
                    val progress = if (transfer.total == 0uL) {
                        if (status is TransferStatus.Paused) 0f else if (reducedMotion) 0.5f else 1f
                    } else {
                        (transfer.done.toDouble() / transfer.total.toDouble()).toFloat()
                    }
                    val isPaused = status is TransferStatus.Paused
                    val barAlpha = if (isPaused) 0.5f else 1f
                    LinearProgressIndicator(
                        progress = { progress },
                        color = MaterialTheme.colorScheme.primary.copy(alpha = barAlpha),
                        trackColor = MaterialTheme.colorScheme.secondaryContainer.copy(alpha = barAlpha),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Spacer(Modifier.height(4.dp))
                Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (status is TransferStatus.Running || status is TransferStatus.Waiting) {
                TextButton(onClick = { onPause(transfer.id) }) { Text(stringResource(R.string.transfer_pause)) }
                TextButton(onClick = { onCancel(transfer.id) }) { Text(stringResource(R.string.action_cancel)) }
            } else if (status is TransferStatus.Paused) {
                TextButton(onClick = { onResume(transfer.id) }) { Text(stringResource(R.string.transfer_resume)) }
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
        link.usb -> if (link.rttMs >= 1u) stringResource(R.string.link_usb_rtt, link.rttMs.toInt()) else stringResource(R.string.link_usb)
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
    var revealLatest by remember(latest.id) { mutableStateOf(false) }
    val isGlass = LocalGlassTheme.current
    val isClipText = latest.kind == TimelineKind.CLIP && latest.imageDataUrl == null
    val direction = if (latest.incoming) {
        stringResource(R.string.clipboard_history_from, latest.deviceName)
    } else {
        stringResource(R.string.clipboard_history_sent_to, latest.deviceName)
    }
    val shownTitle = if (isClipText && !revealLatest) {
        stringResource(R.string.clipboard_preview_hidden)
    } else {
        latest.title
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 28.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .clickable(role = Role.Button, onClick = onOpen)
            .padding(20.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Text(
                stringResource(R.string.timeline_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            if (isClipText) {
                TextButton(
                    onClick = { revealLatest = !revealLatest },
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 8.dp, vertical = 0.dp),
                    modifier = Modifier.heightIn(min = 28.dp),
                ) {
                    Text(
                        stringResource(
                            if (revealLatest) R.string.clipboard_preview_hide else R.string.clipboard_preview_reveal,
                        ),
                        style = MaterialTheme.typography.labelMedium,
                    )
                }
                if (total > 0u) {
                    Spacer(Modifier.width(4.dp))
                }
            }
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
            stringResource(R.string.timeline_latest_summary, shownTitle, direction),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
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
        containerColor = glassModalContainerColor(),
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
                        val dayLabels = remember(pagedEntries, todayLabel, yesterdayLabel) {
                            timelineDayLabels(pagedEntries, todayLabel, yesterdayLabel)
                        }
                        LazyColumn(
                            modifier = Modifier.fillMaxWidth().heightIn(max = 360.dp),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            pagedEntries.forEachIndexed { idx, entry ->
                                val dayLabel = dayLabels[idx]
                                val prevDayLabel = if (idx > 0) dayLabels[idx - 1] else null
                                if (dayLabel != prevDayLabel) {
                                    item(key = "day_${entry.id}", contentType = "day_header") {
                                        Text(
                                            dayLabel,
                                            style = MaterialTheme.typography.labelMedium,
                                            color = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.padding(top = if (idx > 0) 6.dp else 0.dp),
                                        )
                                    }
                                }
                                item(key = entry.id, contentType = "timeline_row") {
                                    TimelineRow(
                                        entry = entry,
                                        devicePaired = state.device(entry.deviceId) != null,
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
    devicePaired: Boolean,
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
    val justNowLabel = stringResource(R.string.clipboard_history_just_now)
    val meta = remember(entry.id, entry.timestamp, entry.sizeBytes, entry.durationSecs, entry.detail, direction, justNowLabel) {
        val now = System.currentTimeMillis()
        val whenText = if (now - entry.timestamp * 1000L < DateUtils.MINUTE_IN_MILLIS) {
            justNowLabel
        } else {
            DateUtils.getRelativeTimeSpanString(entry.timestamp * 1000L, now, DateUtils.MINUTE_IN_MILLIS).toString()
        }
        buildString {
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
    }

    val targetExists by produceState(
        initialValue = when (entry.kind) {
            TimelineKind.LINK -> true
            TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> entry.target.startsWith("content://")
            else -> false
        },
        entry.target,
        entry.kind,
    ) {
        if (entry.kind == TimelineKind.FILE || entry.kind == TimelineKind.PHOTO || entry.kind == TimelineKind.RECORDING) {
            if (!entry.target.startsWith("content://") && entry.target.isNotEmpty()) {
                value = withContext(Dispatchers.IO) { File(entry.target).exists() }
            }
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
    val canSendAgain = devicePaired && when (entry.kind) {
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
            val clipId = entry.refId
            if (entry.kind == TimelineKind.CLIP && entry.clipAvailable && clipId != null && entry.imageDataUrl != null) {
                val bitmap by produceState<ImageBitmap?>(initialValue = previewBitmapCache.get(clipId), clipId) {
                    if (value != null) return@produceState
                    val bytes = loadClipImage(clipId) ?: return@produceState
                    val decoded = withContext(Dispatchers.Default) { previewBitmap(bytes)?.asImageBitmap() }
                    if (decoded != null) previewBitmapCache.put(clipId, decoded)
                    value = decoded
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
                val ext = target.substringAfterLast('.', "").lowercase()
                val mime = context.contentResolver.getType(uri)?.takeIf { it != "application/octet-stream" }
                    ?: android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
                    ?: when (entry.kind) {
                        TimelineKind.PHOTO -> "image/*"
                        TimelineKind.RECORDING -> "audio/*"
                        else -> null
                    }
                val view = Intent(Intent.ACTION_VIEW)
                    .apply {
                        if (mime != null) setDataAndType(uri, mime) else setData(uri)
                    }
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            else -> Unit
        }
    }
}

private fun timelineDayLabels(
    entries: List<TimelineEntry>,
    todayLabel: String,
    yesterdayLabel: String,
): List<String> {
    if (entries.isEmpty()) return emptyList()
    val nowCal = Calendar.getInstance()
    val nowYear = nowCal.get(Calendar.YEAR)
    val nowDay = nowCal.get(Calendar.DAY_OF_YEAR)
    val itemCal = Calendar.getInstance()
    val fmt = DateFormat.getDateInstance(DateFormat.MEDIUM)
    val date = Date()
    return entries.map { entry ->
        val ms = entry.timestamp * 1000L
        itemCal.timeInMillis = ms
        val sameYear = nowYear == itemCal.get(Calendar.YEAR)
        val dayDiff = if (sameYear) nowDay - itemCal.get(Calendar.DAY_OF_YEAR) else -1
        when {
            sameYear && dayDiff == 0 -> todayLabel
            sameYear && dayDiff == 1 -> yesterdayLabel
            else -> {
                date.time = ms
                fmt.format(date)
            }
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

private fun RemoteStorageEntry.isDirectoryEntry(): Boolean =
    kind.equals("dir", ignoreCase = true) ||
        kind.equals("directory", ignoreCase = true) ||
        kind.equals("folder", ignoreCase = true)

private fun parentRemotePath(path: String): String {
    val normalized = path.trim('/').replace('\\', '/')
    val slash = normalized.lastIndexOf('/')
    return if (slash <= 0) "" else normalized.substring(0, slash)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PcFilesSheet(
    device: Device,
    onListFiles: (path: String, onResult: (PcFolder) -> Unit) -> Unit,
    onDownloadFile: (remotePath: String, fileName: String, onSaved: (Uri?) -> Unit) -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val snackbarHostState = remember { SnackbarHostState() }

    var currentPath by remember(device.id) { mutableStateOf("") }
    var entries by remember(device.id) { mutableStateOf<List<RemoteStorageEntry>>(emptyList()) }
    var loading by remember(device.id) { mutableStateOf(false) }
    var downloadingPath by remember(device.id) { mutableStateOf<String?>(null) }
    var refreshTick by remember(device.id) { mutableStateOf(0) }
    var problem by remember(device.id) { mutableStateOf<Int?>(null) }

    LaunchedEffect(device.id, currentPath, refreshTick) {
        loading = true
        onListFiles(currentPath) { folder ->
            problem = folder.problem
            entries = folder.entries.sortedWith(
                compareBy<RemoteStorageEntry> { !it.isDirectoryEntry() }
                    .thenBy(String.CASE_INSENSITIVE_ORDER) { it.name },
            )
            loading = false
        }
    }

    val quickFolders = listOf(
        "" to stringResource(R.string.pc_files_root),
        "Downloads" to stringResource(R.string.pc_files_folder_downloads),
        "Documents" to stringResource(R.string.pc_files_folder_documents),
        "Desktop" to stringResource(R.string.pc_files_folder_desktop),
        "Pictures" to stringResource(R.string.pc_files_folder_pictures),
    )

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = glassModalContainerColor(),
        scrimColor = glassModalScrimColor(),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        stringResource(R.string.pc_files_title, device.name),
                        style = MaterialTheme.typography.titleLarge,
                    )
                    val breadcrumb = if (currentPath.isEmpty()) {
                        stringResource(R.string.pc_files_root)
                    } else {
                        currentPath.replace('\\', '/')
                    }
                    Text(
                        breadcrumb,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (currentPath.isNotEmpty()) {
                    OutlinedButton(onClick = { currentPath = parentRemotePath(currentPath) }) {
                        Text(stringResource(R.string.pc_files_up))
                    }
                    Spacer(Modifier.width(4.dp))
                }
                IconButton(onClick = { refreshTick++ }) {
                    Icon(
                        painterResource(R.drawable.ic_refresh),
                        contentDescription = stringResource(R.string.action_refresh),
                    )
                }
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                quickFolders.forEach { (folderPath, label) ->
                    FilterChip(
                        selected = currentPath.equals(folderPath, ignoreCase = true),
                        onClick = { currentPath = folderPath },
                        label = { Text(label) },
                    )
                }
            }

            if (loading || downloadingPath != null) {
                if (LocalReducedMotion.current) {
                    LinearProgressIndicator(progress = { 0.5f }, modifier = Modifier.fillMaxWidth())
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
            }

            when {
                problem != null && !loading -> {
                    Text(
                        stringResource(problem ?: R.string.pc_files_error, device.name),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 16.dp),
                    )
                }
                loading && entries.isEmpty() -> {
                    Text(
                        stringResource(R.string.pc_files_loading),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 16.dp),
                    )
                }
                entries.isEmpty() && currentPath.isEmpty() -> {
                    // Quick-access folder list when root listing is empty.
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 380.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        val rootDefaults = quickFolders.drop(1)
                        items(rootDefaults, key = { it.first }) { (folderPath, label) ->
                            Surface(
                                shape = MaterialTheme.shapes.large,
                                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable(role = Role.Button) { currentPath = folderPath },
                            ) {
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 16.dp, vertical = 14.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                ) {
                                    Text(
                                        label,
                                        style = MaterialTheme.typography.titleSmall,
                                        modifier = Modifier.weight(1f),
                                    )
                                    Text(
                                        "›",
                                        style = MaterialTheme.typography.titleMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        }
                    }
                }
                entries.isEmpty() -> {
                    Text(
                        stringResource(R.string.pc_files_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 16.dp),
                    )
                }
                else -> {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 420.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(entries, key = { it.path.ifEmpty { it.name } }) { entry ->
                            val isDir = entry.isDirectoryEntry()
                            val resolvedPath = entry.path.ifEmpty {
                                if (currentPath.isEmpty()) entry.name else "$currentPath/${entry.name}"
                            }
                            val isDownloading = downloadingPath == resolvedPath
                            val savedMessage = stringResource(R.string.pc_files_saved, entry.name)
                            val openActionLabel = stringResource(R.string.timeline_action_open)
                            val failedMessage = stringResource(R.string.pc_files_failed, entry.name)
                            Surface(
                                shape = MaterialTheme.shapes.large,
                                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable(
                                        enabled = downloadingPath == null,
                                        role = Role.Button,
                                    ) {
                                        if (isDir) {
                                            currentPath = resolvedPath
                                        } else {
                                            downloadingPath = resolvedPath
                                            onDownloadFile(resolvedPath, entry.name) { savedUri ->
                                                downloadingPath = null
                                                scope.launch {
                                                    if (savedUri != null) {
                                                        val res = snackbarHostState.showSnackbar(
                                                            message = savedMessage,
                                                            actionLabel = openActionLabel,
                                                            duration = SnackbarDuration.Short,
                                                        )
                                                        if (res == SnackbarResult.ActionPerformed) {
                                                            openSavedPcFile(context, savedUri, entry.name)
                                                        }
                                                    } else {
                                                        snackbarHostState.showSnackbar(
                                                            message = failedMessage,
                                                            duration = SnackbarDuration.Short,
                                                        )
                                                    }
                                                }
                                            }
                                        }
                                    },
                            ) {
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .padding(horizontal = 16.dp, vertical = 12.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                ) {
                                    Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                                        Text(
                                            entry.name,
                                            style = MaterialTheme.typography.titleSmall,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                        val meta = if (isDownloading) {
                                            stringResource(R.string.pc_files_downloading, entry.name)
                                        } else if (!isDir) {
                                            Formatter.formatShortFileSize(context, entry.size.toLong())
                                        } else {
                                            null
                                        }
                                        if (meta != null) {
                                            Spacer(Modifier.height(2.dp))
                                            Text(
                                                meta,
                                                style = MaterialTheme.typography.bodySmall,
                                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                        }
                                    }
                                    if (isDir) {
                                        Text(
                                            "›",
                                            style = MaterialTheme.typography.titleMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }

            SnackbarHost(hostState = snackbarHostState)
        }
    }
}

private fun openSavedPcFile(context: android.content.Context, uri: Uri, fileName: String) {
    runCatching {
        val mime = ReceivedFiles.mimeOf(fileName)
        val view = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, mime)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

private fun softContinuousVector(name: String, svgPath: String, strokeWidth: Float = 1.9f): ImageVector {
    val nodes = androidx.compose.ui.graphics.vector.PathParser().parsePathString(svgPath).toNodes()
    return ImageVector.Builder(
        name = name,
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).addPath(
        pathData = nodes,
        fill = null,
        stroke = SolidColor(Color.White),
        strokeLineWidth = strokeWidth,
        strokeLineCap = StrokeCap.Round,
        strokeLineJoin = StrokeJoin.Round,
    ).build()
}

private val RemotePcIcon: ImageVector by lazy {
    softContinuousVector(
        name = "RemotePc",
        svgPath = "M2,6A2.5,2.5 0 0,1 4.5,3.5H19.5A2.5,2.5 0 0,1 22,6V15A2.5,2.5 0 0,1 19.5,17.5H4.5A2.5,2.5 0 0,1 2,15Z M8,21H16 M12,17.5V21",
    )
}

private val DeckGridIcon: ImageVector by lazy {
    softContinuousVector(
        name = "DeckGrid",
        svgPath = "M3.5,5.5A2,2 0 0,1 5.5,3.5H9.5A2,2 0 0,1 11.5,5.5V9.5A2,2 0 0,1 9.5,11.5H5.5A2,2 0 0,1 3.5,9.5Z M12.5,5.5A2,2 0 0,1 14.5,3.5H18.5A2,2 0 0,1 20.5,5.5V9.5A2,2 0 0,1 18.5,11.5H14.5A2,2 0 0,1 12.5,9.5Z M3.5,14.5A2,2 0 0,1 5.5,12.5H9.5A2,2 0 0,1 11.5,14.5V18.5A2,2 0 0,1 9.5,20.5H5.5A2,2 0 0,1 3.5,18.5Z M12.5,14.5A2,2 0 0,1 14.5,12.5H18.5A2,2 0 0,1 20.5,14.5V18.5A2,2 0 0,1 18.5,20.5H14.5A2,2 0 0,1 12.5,18.5Z",
    )
}

private val CameraLensIcon: ImageVector by lazy {
    softContinuousVector(
        name = "CameraLens",
        svgPath = "M15.5,10.2L20.2,7.3A1,1 0 0,1 21.7,8.1V15.9A1,1 0 0,1 20.2,16.7L15.5,13.8 M2.5,8A2.5,2.5 0 0,1 5,5.5H13A2.5,2.5 0 0,1 15.5,8V16A2.5,2.5 0 0,1 13,18.5H5A2.5,2.5 0 0,1 2.5,16Z",
    )
}

private val SendArrowIcon: ImageVector by lazy {
    softContinuousVector(
        name = "SendArrow",
        svgPath = "M12,16V4 M7,9L12,4L17,9 M4,20H20",
    )
}

private val ChevronDownIcon: ImageVector by lazy {
    softContinuousVector(
        name = "ChevronDown",
        svgPath = "M6,9L12,15L18,9",
        strokeWidth = 2.2f,
    )
}

private val DeskClockIcon: ImageVector by lazy {
    softContinuousVector(
        name = "DeskClock",
        svgPath = "M3,7A2.5,2.5 0 0,1 5.5,4.5H18.5A2.5,2.5 0 0,1 21,7V17A2.5,2.5 0 0,1 18.5,19.5H5.5A2.5,2.5 0 0,1 3,17Z M12,8.5V12L14.5,13.5",
    )
}

private val FolderIcon: ImageVector by lazy {
    softContinuousVector(
        name = "Folder",
        svgPath = "M20,19.5A2,2 0 0,0 22,17.5V8.5A2,2 0 0,0 20,6.5H12.1A2,2 0 0,1 10.4,5.6L9.5,4.4A2,2 0 0,0 7.9,3.5H4A2,2 0 0,0 2,5.5V17.5A2,2 0 0,0 4,19.5Z",
    )
}

private val ClipboardIcon: ImageVector by lazy {
    softContinuousVector(
        name = "Clipboard",
        svgPath = "M9,3.5H15A1.5,1.5 0 0,1 16.5,5V5.5A1.5,1.5 0 0,1 15,7H9A1.5,1.5 0 0,1 7.5,5.5V5A1.5,1.5 0 0,1 9,3.5Z M16.5,5H18A2.5,2.5 0 0,1 20.5,7.5V19A2.5,2.5 0 0,1 18,21.5H6A2.5,2.5 0 0,1 3.5,19V7.5A2.5,2.5 0 0,1 6,5H7.5",
    )
}

private val PcFolderIcon: ImageVector by lazy {
    softContinuousVector(
        name = "PcFolder",
        svgPath = "M20,19.5A2,2 0 0,0 22,17.5V8.5A2,2 0 0,0 20,6.5H12.1A2,2 0 0,1 10.4,5.6L9.5,4.4A2,2 0 0,0 7.9,3.5H4A2,2 0 0,0 2,5.5V17.5A2,2 0 0,0 4,19.5Z M9,13H15 M12,10.5L14.5,13L12,15.5",
    )
}

private val MicIcon: ImageVector by lazy {
    softContinuousVector(
        name = "Mic",
        svgPath = "M12,2.5A3.2,3.2 0 0,1 15.2,5.7V11.3A3.2,3.2 0 0,1 8.8,11.3V5.7A3.2,3.2 0 0,1 12,2.5Z M18.5,10.5V11.3A6.5,6.5 0 0,1 5.5,11.3V10.5 M12,17.8V21.5 M8.5,21.5H15.5",
    )
}

private val BellIcon: ImageVector by lazy {
    softContinuousVector(
        name = "Bell",
        svgPath = "M6,8.5A6,6 0 0,1 18,8.5C18,14.5 20.2,16.5 20.2,16.5H3.8C3.8,16.5 6,14.5 6,8.5 M10.2,20A2.1,2.1 0 0,0 13.8,20",
    )
}

private val LinkChainIcon: ImageVector by lazy {
    softContinuousVector(
        name = "LinkChain",
        svgPath = "M10,14A4.5,4.5 0 0,0 16.4,14L19.2,11.2A4.5,4.5 0 0,0 12.8,4.8L11.2,6.4 M14,10A4.5,4.5 0 0,0 7.6,10L4.8,12.8A4.5,4.5 0 0,0 11.2,19.2L12.8,17.6",
    )
}

private val CloseQueueIcon: ImageVector by lazy {
    softContinuousVector(
        name = "CloseQueue",
        svgPath = "M6,6L18,18 M18,6L6,18",
    )
}

// The PC card's buttons sit two to a row, so the default 24 dp side padding
// leaves too little room for labels like "Browse PC files" on narrow phones.
private val CardButtonPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 12.dp, vertical = 8.dp)

// One line that shrinks a little on narrow phones or with large font sizes
// before it would ever cut a word off.
@Composable
private fun ButtonLabel(text: String) {
    val size = LocalTextStyle.current.fontSize.takeIf { it.isSpecified } ?: 14.sp
    Text(
        text,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        autoSize = TextAutoSize.StepBased(minFontSize = 11.sp, maxFontSize = size, stepSize = 0.5.sp),
    )
}

/** The localized fallback label for a queue item whose title is empty. */
private fun queueKindLabel(kind: FfiQueueKind): Int = when (kind) {
    FfiQueueKind.CLIPBOARD -> R.string.queue_kind_clipboard
    FfiQueueKind.CLIPBOARD_IMAGE -> R.string.queue_kind_clipboard_image
    FfiQueueKind.FILES -> R.string.queue_kind_files
    FfiQueueKind.FOLDER -> R.string.queue_kind_folder
    FfiQueueKind.LINK -> R.string.queue_kind_link
    FfiQueueKind.HANDOFF_DOCUMENT -> R.string.queue_kind_document
    FfiQueueKind.RECORDING -> R.string.queue_kind_recording
    FfiQueueKind.TASK_NOTIFY -> R.string.queue_kind_task_alert
    FfiQueueKind.CAMERA_REQUEST -> R.string.queue_kind_camera_request
}
