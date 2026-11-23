// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Build
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.ui.theme.LocalReducedMotion
import kotlin.math.abs
import kotlin.math.hypot
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.material3.FilterChip

enum class RemoteMode(val wireName: String) {
    Touchpad("touchpad"),
    AirMouse("air"),
    ScreenMap("screen_map"),
    DeskMouse("desk"),
    Stick("stick"),
    Presentation("presentation"),
    Screen("screen"),
    ;

    companion object {
        fun fromWireName(raw: String?): RemoteMode = when (raw?.trim()?.lowercase()) {
            "air", "air_mouse", "airmouse" -> AirMouse
            "map", "screen_map", "screenmap" -> ScreenMap
            "desk", "desk_mouse", "deskmouse" -> DeskMouse
            "stick", "thumbstick" -> Stick
            "presentation", "slides" -> Presentation
            "screen", "mirror" -> Screen
            else -> Touchpad
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RemoteScreen(
    device: Device,
    core: Core,
    sensitivity: Float,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    initialMode: RemoteMode = RemoteMode.Touchpad,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences = app.nectarlink.android.ui.RemotePreferences(),
    onUpdateRemotePrefs: ((app.nectarlink.android.ui.RemotePreferences) -> app.nectarlink.android.ui.RemotePreferences) -> Unit = {},
    onModeChanged: (RemoteMode) -> Unit = {},
) {
    val view = LocalView.current
    val context = LocalContext.current

    // Keep the phone's screen on while controlling the PC, and release held state on leave.
    app.nectarlink.android.ui.KeepScreenOn(
        device.online && (remotePrefs.keepAwake || app.nectarlink.android.ui.rememberTouchedRecently()),
    )
    DisposableEffect(view, device.id) {
        onDispose {
            core.clearPendingRemoteMotion()
            core.remoteLaser(device.id, false)
        }
    }

    // Apply persisted landscape/portrait orientation preference while in Remote.
    DisposableEffect(context, remotePrefs.landscapeRotated) {
        val activity = context as? Activity
        if (activity != null) {
            activity.requestedOrientation = if (remotePrefs.landscapeRotated) {
                android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            } else {
                android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            }
        }
        onDispose {
            (context as? Activity)?.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        }
    }

    // The Gamepad takes the whole screen while it's open (#442).
    var gamepadOpen by rememberSaveable(device.id) { mutableStateOf(false) }
    if (gamepadOpen && device.has("input.gamepad")) {
        GamepadScreen(device = device, core = core, onClose = { gamepadOpen = false })
        return
    }

    var mode by rememberSaveable(initialMode) { mutableStateOf(initialMode) }
    var screenFullScreen by rememberSaveable { mutableStateOf(false) }
    var access by remember(device.id) { mutableStateOf(Core.RemoteAccess.Allowed) }
    var remoteInfo by remember(device.id) { mutableStateOf<app.nectarlink.core.RemoteInfo?>(null) }
    var showGesturesHelp by rememberSaveable { mutableStateOf(false) }
    var showHyperOsHint by rememberSaveable { mutableStateOf(false) }
    // In landscape the touchpad takes the screen, with its buttons under it.
    var touchpadOnlyChosen by rememberSaveable { mutableStateOf(true) }
    val landscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val touchpadOnly = landscape && touchpadOnlyChosen && mode == RemoteMode.Touchpad &&
        device.online && access == Core.RemoteAccess.Allowed
    val isFullScreen = (mode == RemoteMode.Screen && screenFullScreen) || touchpadOnly
    DisposableEffect(view, touchpadOnly) {
        val window = (context as? Activity)?.window
        val bars = window?.let { WindowCompat.getInsetsController(it, view) }
        if (touchpadOnly) {
            bars?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            bars?.hide(WindowInsetsCompat.Type.systemBars())
        }
        onDispose {
            if (touchpadOnly) bars?.show(WindowInsetsCompat.Type.systemBars())
        }
    }
    val utteranceJoiner = remember(device.id) { UtteranceJoiner() }

    LaunchedEffect(mode) {
        onModeChanged(mode)
        core.clearPendingRemoteMotion()
        if (mode != RemoteMode.Screen && screenFullScreen) {
            screenFullScreen = false
        }
    }

    // Check access, announce foreground presence, and fetch precision display layout (`remote.info`).
    LaunchedEffect(device.id, device.online) {
        if (!device.online) {
            access = Core.RemoteAccess.Offline
            return@LaunchedEffect
        }
        access = core.remoteCheckWithPresence(device.id, presence = true)
        if (device.has("input.precision")) {
            remoteInfo = core.remoteInfo(device.id)
        }
        while (device.online) {
            delay(if (access == Core.RemoteAccess.Denied) 1_500L else 15_000L)
            access = core.remoteCheckWithPresence(device.id, presence = true)
        }
    }
    DisposableEffect(device.id) {
        onDispose {
            if (device.online) {
                kotlinx.coroutines.MainScope().launch {
                    runCatching { core.remoteCheckWithPresence(device.id, presence = false) }
                }
            }
        }
    }

    val performHaptic: (TouchpadHaptic) -> Unit = remember(view, remotePrefs.haptics) {
        { kind ->
            if (remotePrefs.haptics) {
                val feedback = when (kind) {
                    TouchpadHaptic.Soft -> android.view.HapticFeedbackConstants.CLOCK_TICK
                    TouchpadHaptic.Tick -> android.view.HapticFeedbackConstants.KEYBOARD_TAP
                    TouchpadHaptic.Click -> android.view.HapticFeedbackConstants.CONTEXT_CLICK
                    TouchpadHaptic.Heavy -> android.view.HapticFeedbackConstants.LONG_PRESS
                    TouchpadHaptic.Double -> android.view.HapticFeedbackConstants.CONFIRM
                }
                view.performHapticFeedback(feedback)
            }
        }
    }

    val dispatchAction: (TouchpadAction) -> Unit = remember(device.id, core) {
        { action ->
            val wire = when (action) {
                TouchpadAction.SwitchStart -> "switch_start"
                TouchpadAction.SwitchNext -> "switch_next"
                TouchpadAction.SwitchPrevious -> "switch_prev"
                TouchpadAction.SwitchEnd -> "switch_end"
                TouchpadAction.SwitchCancel -> "switch_cancel"
                TouchpadAction.WorkspaceNext -> "workspace_next"
                TouchpadAction.WorkspacePrevious -> "workspace_prev"
                TouchpadAction.Overview -> "overview_open"
                TouchpadAction.OverviewClose -> "overview_close"
                TouchpadAction.ShowDesktop -> "desktop_toggle"
                TouchpadAction.Search -> "search"
                TouchpadAction.Notifications -> "notifications"
            }
            if (device.has("input.precision")) {
                core.remoteGesture(device.id, wire) { access = it }
            }
        }
    }

    if (showGesturesHelp) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { showGesturesHelp = false },
            title = { Text(stringResource(R.string.remote_gestures_help)) },
            text = { Text(stringResource(R.string.remote_gestures_guide_body)) },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = { showGesturesHelp = false }) {
                    Text(stringResource(R.string.remote_close))
                }
            },
        )
    }

    if (showHyperOsHint) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { showHyperOsHint = false },
            title = { Text(stringResource(R.string.remote_hyperos_stolen_title)) },
            text = { Text(stringResource(R.string.remote_hyperos_stolen_body)) },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = {
                    showHyperOsHint = false
                    val oem = Intent().setClassName(
                        "com.android.settings",
                        "com.android.settings.SubSettings",
                    )
                    val fallback = Intent(android.provider.Settings.ACTION_SETTINGS)
                    runCatching { context.startActivity(oem) }
                        .onFailure { runCatching { context.startActivity(fallback) } }
                }) {
                    Text(stringResource(R.string.remote_hyperos_open_settings))
                }
            },
            dismissButton = {
                androidx.compose.material3.TextButton(onClick = { showHyperOsHint = false }) {
                    Text(stringResource(R.string.remote_close))
                }
            },
        )
    }

    val onStatus: (Core.RemoteAccess) -> Unit = { next -> access = next }

    Column(
        modifier = modifier
            .fillMaxSize()
            .then(
                if (isFullScreen) {
                    Modifier
                } else {
                    Modifier
                        .systemBarsPadding()
                        .padding(horizontal = 16.dp, vertical = 12.dp)
                },
            ),
        verticalArrangement = Arrangement.spacedBy(if (isFullScreen) 0.dp else 12.dp),
    ) {
        if (!isFullScreen) {
            // Header: Back, PC name, online dot
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = onBack) {
                    Icon(
                        painter = painterResource(R.drawable.ic_back),
                        contentDescription = stringResource(R.string.action_back),
                    )
                }
                Spacer(Modifier.width(4.dp))
                Text(
                    device.name,
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.weight(1f),
                )
                androidx.compose.material3.TextButton(
                    onClick = { onUpdateRemotePrefs { it.copy(landscapeRotated = !it.landscapeRotated) } },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                ) {
                    Text(stringResource(R.string.remote_rotate), style = MaterialTheme.typography.labelMedium)
                }
                androidx.compose.material3.TextButton(
                    onClick = { showGesturesHelp = true },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                ) {
                    Text(stringResource(R.string.remote_gestures_help), style = MaterialTheme.typography.labelMedium)
                }
                if (device.has("input.gamepad")) {
                    IconButton(onClick = { gamepadOpen = true }) {
                        Icon(
                            painter = painterResource(R.drawable.ic_gamepad),
                            contentDescription = stringResource(R.string.gamepad_open),
                        )
                    }
                    Spacer(Modifier.width(4.dp))
                }
                Surface(
                    shape = CircleShape,
                    color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                    modifier = Modifier.size(10.dp),
                ) {}
            }

            // Mode switcher across all pointer, presentation, and mirror modes.
            val modes = remember {
                listOf(
                    RemoteMode.Touchpad to R.string.remote_mode_touchpad,
                    RemoteMode.AirMouse to R.string.remote_mode_air,
                    RemoteMode.ScreenMap to R.string.remote_mode_screen_map,
                    RemoteMode.DeskMouse to R.string.remote_mode_desk,
                    RemoteMode.Stick to R.string.remote_mode_stick,
                    RemoteMode.Presentation to R.string.remote_mode_presentation,
                    RemoteMode.Screen to R.string.remote_mode_screen,
                )
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for ((m, labelRes) in modes) {
                    FilterChip(
                        selected = mode == m,
                        onClick = { mode = m },
                        label = {
                            Text(
                                stringResource(labelRes),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        },
                    )
                }
            }

            // Status banner when offline, denied, or unsupported
            when {
                !device.online || access == Core.RemoteAccess.Offline -> OfflineWakeBanner(
                    device = device,
                    onWake = { core.wake(device.id) },
                )
                access == Core.RemoteAccess.Denied -> StatusBanner(
                    title = stringResource(R.string.remote_denied_title, device.name),
                    text = stringResource(R.string.remote_denied_text, device.name),
                )
                access == Core.RemoteAccess.Unsupported -> StatusBanner(
                    title = null,
                    text = stringResource(R.string.remote_unsupported_text, device.name),
                )
            }
        }

        val reducedMotion = LocalReducedMotion.current
        AnimatedContent(
            targetState = mode,
            transitionSpec = {
                if (reducedMotion) {
                    EnterTransition.None togetherWith ExitTransition.None
                } else {
                    val forward = targetState.ordinal >= initialState.ordinal
                    (slideInHorizontally(tween(240, easing = FastOutSlowInEasing)) { if (forward) it / 8 else -it / 8 } +
                        fadeIn(tween(200))) togetherWith
                        (slideOutHorizontally(tween(180, easing = FastOutSlowInEasing)) { if (forward) -it / 10 else it / 10 } +
                            fadeOut(tween(150)))
                }
            },
            modifier = Modifier.weight(1f).fillMaxWidth(),
            label = "RemoteModeTransition",
        ) { currentMode ->
            when (currentMode) {
                RemoteMode.Touchpad -> TouchpadView(
                    pcId = device.id,
                    pcName = device.name,
                    core = core,
                    sensitivity = sensitivity,
                    remotePrefs = remotePrefs,
                    hasPrecision = device.has("input.precision"),
                    onAction = dispatchAction,
                    onHaptic = performHaptic,
                    onGestureStolen = { showHyperOsHint = true },
                    utteranceJoiner = utteranceJoiner,
                    onStatus = onStatus,
                    canBeTouchpadOnly = landscape,
                    touchpadOnly = touchpadOnly,
                    onTouchpadOnly = { touchpadOnlyChosen = it },
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.AirMouse -> AirMouseView(
                    pcId = device.id,
                    pcName = device.name,
                    core = core,
                    sensitivity = remotePrefs.airSpeed,
                    remotePrefs = remotePrefs,
                    onUpdateRemotePrefs = onUpdateRemotePrefs,
                    utteranceJoiner = utteranceJoiner,
                    onStatus = onStatus,
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.ScreenMap -> ScreenMapView(
                    pcId = device.id,
                    core = core,
                    remoteInfo = remoteInfo,
                    remotePrefs = remotePrefs,
                    onAction = dispatchAction,
                    onHaptic = performHaptic,
                    onGestureStolen = { showHyperOsHint = true },
                    onStatus = onStatus,
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.DeskMouse -> DeskMouseView(
                    pcId = device.id,
                    core = core,
                    remotePrefs = remotePrefs,
                    onHaptic = performHaptic,
                    onStatus = onStatus,
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.Stick -> StickView(
                    pcId = device.id,
                    core = core,
                    remotePrefs = remotePrefs,
                    onAction = dispatchAction,
                    onHaptic = performHaptic,
                    onGestureStolen = { showHyperOsHint = true },
                    onStatus = onStatus,
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.Presentation -> PresentationView(
                    pcId = device.id,
                    pcName = device.name,
                    core = core,
                    onStatus = onStatus,
                    modifier = Modifier.fillMaxSize(),
                )
                RemoteMode.Screen -> Column(
                    // Above the keyboard while it's open: apps are drawn edge to edge
                    // (Android 15 and later), so it would cover the typing field.
                    modifier = Modifier.fillMaxSize().imePadding(),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    PcMirrorView(
                        pcId = device.id,
                        pcName = device.name,
                        core = core,
                        isFullScreen = screenFullScreen,
                        onFullScreenChange = { screenFullScreen = it },
                        touchpadSensitivity = sensitivity,
                        onStatus = onStatus,
                        modifier = Modifier.weight(1f).fillMaxWidth(),
                    )
                    // The touchpad under the screen: watch the PC and move its pointer together.
                    // While typing, the room goes to the screen and the field.
                    if (!screenFullScreen && !WindowInsets.isImeVisible) {
                        TouchpadSurface(
                            pcId = device.id,
                            core = core,
                            sensitivity = sensitivity,
                            remotePrefs = remotePrefs,
                            hasPrecision = device.has("input.precision"),
                            onAction = dispatchAction,
                            onHaptic = performHaptic,
                            onGestureStolen = { showHyperOsHint = true },
                            onStatus = onStatus,
                            modifier = Modifier.fillMaxWidth().height(180.dp),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun OfflineWakeBanner(device: Device, onWake: () -> Unit) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
            when {
                !device.canWake -> {
                    Text(
                        stringResource(R.string.remote_offline_text, device.name),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                device.wakeState == WakeState.Waking -> {
                    Text(
                        stringResource(R.string.wake_waking, device.name),
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Spacer(Modifier.height(8.dp))
                    if (LocalReducedMotion.current) {
                        LinearProgressIndicator(progress = { 0.5f }, modifier = Modifier.fillMaxWidth())
                    } else {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }
                }
                device.wakeState == WakeState.TimedOut -> {
                    Text(
                        stringResource(R.string.wake_timeout_title, device.name),
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        stringResource(R.string.wake_timeout_text, device.name),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Spacer(Modifier.height(10.dp))
                    Button(onClick = onWake) {
                        Text(stringResource(R.string.action_wake_pc))
                    }
                }
                else -> {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            stringResource(R.string.remote_offline_text, device.name),
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.weight(1f),
                        )
                        Spacer(Modifier.width(12.dp))
                        Button(onClick = onWake) {
                            Text(stringResource(R.string.action_wake_pc))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun StatusBanner(title: String?, text: String) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
            if (title != null) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                Spacer(Modifier.height(2.dp))
            }
            Text(text, style = MaterialTheme.typography.bodySmall)
        }
    }
}

/** The touchpad surface: moves, clicks, drags, pinches, gestures, and scrolls the PC pointer. */
@Composable
private fun TouchpadSurface(
    pcId: String,
    core: Core,
    sensitivity: Float,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences = app.nectarlink.android.ui.RemotePreferences(),
    hasPrecision: Boolean = false,
    onAction: (TouchpadAction) -> Unit = {},
    onHaptic: (TouchpadHaptic) -> Unit = {},
    onGestureStolen: () -> Unit = {},
) {
    var dragging by remember { mutableStateOf(false) }
    var touchPoint by remember { mutableStateOf<Offset?>(null) }
    val primaryColor = MaterialTheme.colorScheme.primary
    val outlineColor = MaterialTheme.colorScheme.outlineVariant
    val touchpadDesc = stringResource(R.string.remote_mode_touchpad)
    val densityPx = androidx.compose.ui.platform.LocalDensity.current.density
    val options = remember(remotePrefs, sensitivity) {
        TouchpadGestureOptions(
            pointerSpeed = remotePrefs.pointerSpeed,
            acceleration = remotePrefs.acceleration,
            scrollSpeed = remotePrefs.scrollSpeed,
            tapToClick = remotePrefs.tapToClick,
            threeFingerMiddleClick = remotePrefs.threeFingerMiddleClick,
            naturalScroll = remotePrefs.naturalScrolling,
            momentum = remotePrefs.momentum,
            relativePointer = true,
        )
    }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        border = BorderStroke(
            width = if (dragging) 2.dp else 1.dp,
            color = if (dragging) primaryColor else outlineColor,
        ),
        modifier = modifier
            .systemGestureExclusion { coords ->
                val strip = (32f * densityPx).coerceAtMost(coords.size.width / 2f)
                val maxH = (200f * densityPx).coerceAtMost(coords.size.height.toFloat())
                val top = (coords.size.height - maxH) / 2f
                androidx.compose.ui.geometry.Rect(0f, top, strip.coerceAtLeast(1f), top + maxH)
            }
            .semantics { contentDescription = touchpadDesc }
            .pointerInput(pcId, options, hasPrecision) {
                touchpadGestures(
                    pcId = pcId,
                    core = core,
                    sensitivity = 1f,
                    onStatus = onStatus,
                    onTouch = { touchPoint = it },
                    onDragging = { dragging = it },
                    options = options,
                    onAction = onAction,
                    onZoom = { dy ->
                        // NectarLink Zoom uses positive = zoom in; pinchNotches returns down-positive (negative = zoom in).
                        if (hasPrecision) core.remoteZoom(pcId, -dy)
                    },
                    onHaptic = onHaptic,
                    onGestureStolen = onGestureStolen,
                    usePreciseMove = hasPrecision,
                )
            },
    ) {
        Box(Modifier.fillMaxSize()) {
            if (remotePrefs.fingerTrails) {
                Canvas(Modifier.fillMaxSize()) {
                    touchPoint?.let { pt ->
                        drawCircle(
                            color = primaryColor.copy(alpha = 0.22f),
                            radius = 28.dp.toPx(),
                            center = pt,
                        )
                        drawCircle(
                            color = primaryColor.copy(alpha = 0.65f),
                            radius = 6.dp.toPx(),
                            center = pt,
                        )
                    }
                }
            }
            Text(
                stringResource(R.string.remote_touchpad_hint),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(horizontal = 20.dp, vertical = 14.dp),
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TouchpadView(
    pcId: String,
    pcName: String,
    core: Core,
    sensitivity: Float,
    utteranceJoiner: UtteranceJoiner,
    onStatus: (Core.RemoteAccess) -> Unit,
    canBeTouchpadOnly: Boolean,
    touchpadOnly: Boolean,
    onTouchpadOnly: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences = app.nectarlink.android.ui.RemotePreferences(),
    hasPrecision: Boolean = false,
    onAction: (TouchpadAction) -> Unit = {},
    onHaptic: (TouchpadHaptic) -> Unit = {},
    onGestureStolen: () -> Unit = {},
) {
    var showKeyboard by rememberSaveable { mutableStateOf(false) }
    // What's been typed since the last Enter: a real field, so the keyboard
    // keeps its page, autocorrects, and has something to delete.
    var typed by remember { mutableStateOf(TextFieldValue("")) }
    val activeMods = remember { mutableStateListOf<String>() }

    val focusRequester = remember { FocusRequester() }
    val keyboardController = LocalSoftwareKeyboardController.current
    val imeVisible = WindowInsets.isImeVisible

    LaunchedEffect(showKeyboard) {
        if (showKeyboard) {
            focusRequester.requestFocus()
            keyboardController?.show()
        } else {
            keyboardController?.hide()
        }
    }

    // Above the keyboard while it's open: apps are drawn edge to edge
    // (Android 15 and later), so the keyboard would cover the typing field.
    BoxWithConstraints(modifier = modifier.fillMaxWidth().imePadding()) {
        // While typing, a strip of touchpad stays when there's room for it and the
        // field, so the pointer can move without closing the keyboard (#52).
        val padWhileTyping = maxHeight >= ROOM_FOR_PAD_WHILE_TYPING
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .then(if (touchpadOnly) Modifier.padding(8.dp) else Modifier),
            verticalArrangement = Arrangement.spacedBy(if (touchpadOnly) 6.dp else 10.dp),
        ) {
            if (!imeVisible) {
                TouchpadSurface(
                    pcId = pcId,
                    core = core,
                    sensitivity = sensitivity,
                    remotePrefs = remotePrefs,
                    hasPrecision = hasPrecision,
                    onAction = onAction,
                    onHaptic = onHaptic,
                    onGestureStolen = onGestureStolen,
                    onStatus = onStatus,
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                )
            }

            // Mouse buttons + Keyboard toggle row (armed modifiers apply to mouse buttons too!)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                FilledTonalButton(
                    onClick = {
                        val mods = activeMods.toList()
                        core.remoteModifiedButton(pcId, "left", "click", mods, onStatus)
                        if (mods.isNotEmpty()) activeMods.clear()
                    },
                    contentPadding = PaddingValues(horizontal = 6.dp, vertical = 6.dp),
                    modifier = Modifier.weight(0.88f).height(46.dp),
                ) {
                    Text(
                        stringResource(R.string.remote_left_click),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                OutlinedButton(
                    onClick = {
                        val mods = activeMods.toList()
                        core.remoteModifiedButton(pcId, "middle", "click", mods, onStatus)
                        if (mods.isNotEmpty()) activeMods.clear()
                    },
                    contentPadding = PaddingValues(horizontal = 6.dp, vertical = 6.dp),
                    modifier = Modifier.weight(1.04f).height(46.dp),
                ) {
                    Text(
                        stringResource(R.string.remote_middle_click),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                FilledTonalButton(
                    onClick = {
                        val mods = activeMods.toList()
                        core.remoteModifiedButton(pcId, "right", "click", mods, onStatus)
                        if (mods.isNotEmpty()) activeMods.clear()
                    },
                    contentPadding = PaddingValues(horizontal = 6.dp, vertical = 6.dp),
                    modifier = Modifier.weight(0.94f).height(46.dp),
                ) {
                    Text(
                        stringResource(R.string.remote_right_click),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Button(
                    onClick = {
                        // Typing needs the keys and the field, so show them all.
                        if (touchpadOnly) onTouchpadOnly(false)
                        showKeyboard = !showKeyboard
                    },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 6.dp),
                    modifier = Modifier.weight(1.26f).height(46.dp),
                ) {
                    Text(
                        stringResource(R.string.remote_keyboard),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (canBeTouchpadOnly) {
                    OutlinedButton(
                        onClick = {
                            if (!touchpadOnly) showKeyboard = false
                            onTouchpadOnly(!touchpadOnly)
                        },
                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 6.dp),
                        modifier = Modifier.weight(1.1f).height(46.dp),
                    ) {
                        Text(
                            stringResource(if (touchpadOnly) R.string.remote_more_controls else R.string.remote_touchpad_only),
                            style = MaterialTheme.typography.labelLarge,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }

            if (touchpadOnly) return@Column

            // Scroll rather than squeeze the typing field when the keyboard is up.
            Column(
                modifier = if (imeVisible) Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()) else Modifier,
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                // Voice typing bar (Dictate button: hold to talk, or tap to start/stop)
                VoiceTypingBar(
                    pcId = pcId,
                    pcName = pcName,
                    core = core,
                    utteranceJoiner = utteranceJoiner,
                    onStatus = onStatus,
                )

                // Special keys & modifiers bar (always accessible in Touchpad mode)
                SpecialKeysBar(
                    activeMods = activeMods,
                    showExtraKeys = showKeyboard,
                    onToggleMod = { mod ->
                        if (mod in activeMods) activeMods.remove(mod) else activeMods.add(mod)
                    },
                    onKey = { key, forceMods ->
                        val mods = forceMods ?: activeMods.toList()
                        core.remoteKey(pcId, key, mods, onStatus)
                        if (forceMods == null) activeMods.clear()
                    },
                )
            }

            // Soft keyboard input row when toggled open
            if (showKeyboard) {
                val typePlaceholder = stringResource(R.string.remote_type_placeholder, pcName)
                OutlinedTextField(
                    value = typed,
                    onValueChange = { next ->
                        val diff = typingDiff(typed.text, next.text)
                        if (activeMods.isNotEmpty() && diff.backspaces == 0 && diff.added.length == 1 &&
                            diff.added[0].isLetterOrDigit()
                        ) {
                            // A shortcut (Ctrl+C...), not text: it stays out of the field.
                            core.remoteKey(pcId, diff.added.lowercase(), activeMods.toList(), onStatus)
                            activeMods.clear()
                            return@OutlinedTextField
                        }
                        repeat(diff.backspaces) { i ->
                            core.remoteKey(pcId, "backspace", if (i == 0) activeMods.toList() else emptyList(), onStatus)
                        }
                        if (diff.added.isNotEmpty()) core.remoteText(pcId, diff.added, onStatus)
                        if (diff.backspaces > 0 || diff.added.isNotEmpty()) activeMods.clear()
                        // The cursor and selection here are free to move: an edit anywhere
                        // goes over as Backspaces from the end, then what follows it again.
                        typed = next
                    },
                    placeholder = { Text(typePlaceholder) },
                    maxLines = 4,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(
                        onSend = {
                            core.remoteKey(pcId, "enter", activeMods.toList(), onStatus)
                            activeMods.clear()
                            typed = TextFieldValue("")
                        },
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .focusRequester(focusRequester)
                        // Backspace with nothing typed here still deletes on the PC.
                        .onPreviewKeyEvent { event ->
                            if (event.type == KeyEventType.KeyDown && event.key == Key.Backspace && typed.text.isEmpty()) {
                                core.remoteKey(pcId, "backspace", activeMods.toList(), onStatus)
                                activeMods.clear()
                                true
                            } else {
                                false
                            }
                        }
                        .semantics { contentDescription = typePlaceholder },
                )
            }

            // While typing, the strip sits right above the keyboard, under the
            // field, where both thumbs reach it (#52).
            if (imeVisible && padWhileTyping) {
                TouchpadSurface(
                    pcId = pcId,
                    core = core,
                    sensitivity = sensitivity,
                    onStatus = onStatus,
                    modifier = Modifier.height(PAD_WHILE_TYPING).fillMaxWidth(),
                )
            }
        }
    }
}

/** The touchpad strip while the keyboard is open. */
private val PAD_WHILE_TYPING = 96.dp

/**
 * Height above the keyboard that fits the strip, the buttons, the field and a row
 * of the keys below them, which scroll to make room.
 */
private val ROOM_FOR_PAD_WHILE_TYPING = 280.dp

@Composable
private fun AirMouseView(
    pcId: String,
    pcName: String,
    core: Core,
    sensitivity: Float,
    utteranceJoiner: UtteranceJoiner,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences = app.nectarlink.android.ui.RemotePreferences(),
    onUpdateRemotePrefs: ((app.nectarlink.android.ui.RemotePreferences) -> app.nectarlink.android.ui.RemotePreferences) -> Unit = {},
) {
    val context = LocalContext.current
    val sensorManager = remember(context) {
        context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    }
    val gyroSensor = remember(sensorManager) {
        sensorManager?.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    }
    val gravitySensor = remember(sensorManager) {
        sensorManager?.getDefaultSensor(Sensor.TYPE_GRAVITY)
    }
    val hasGyro = gyroSensor != null

    var padHeld by remember { mutableStateOf(false) }
    var aimOffset by remember { mutableStateOf(Offset.Zero) }
    val filter = remember { AirMouseFilter() }
    val currentSensitivity by rememberUpdatedState(sensitivity)
    val aimingActive = (padHeld || remotePrefs.airAlwaysOn) && hasGyro

    DisposableEffect(aimingActive, pcId, hasGyro) {
        if (!aimingActive || sensorManager == null) {
            filter.reset()
            aimOffset = Offset.Zero
            return@DisposableEffect onDispose {}
        }
        val gyro = gyroSensor

        filter.reset()
        var gx = 0f
        var gy = 0f
        var gz = 9.81f
        var lastTsNs = 0L
        val listener = object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                if (event.values.size < 3) return
                if (event.sensor.type == Sensor.TYPE_GRAVITY) {
                    gx = event.values[0]
                    gy = event.values[1]
                    gz = event.values[2]
                    return
                }
                if (!aimingActive || event.sensor.type != Sensor.TYPE_GYROSCOPE) return
                val dtSec = if (lastTsNs == 0L || event.timestamp <= lastTsNs) {
                    0.016f
                } else {
                    ((event.timestamp - lastTsNs) * 1e-9f).coerceIn(0.001f, 0.08f)
                }
                lastTsNs = event.timestamp
                val (dx, dy) = if (gravitySensor != null) {
                    val rot = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                        DisplayRotation.fromSurfaceRotation(context.display.rotation)
                    } else {
                        DisplayRotation.Rotation0
                    }
                    val (yaw, pitch) = AirMouseProjection.rates(
                        event.values[0], event.values[1], event.values[2],
                        gx, gy, gz, rot,
                    )
                    AirMouseProjection.pointerDelta(yaw * dtSec, pitch * dtSec, dtSec, currentSensitivity)
                } else {
                    filter.onSample(
                        wx = event.values[0],
                        wy = event.values[1],
                        wz = event.values[2],
                        timestampNs = event.timestamp,
                        sensitivity = currentSensitivity,
                    )
                }
                if (abs(dx) >= 0.05f || abs(dy) >= 0.05f) {
                    core.remoteMovePrecise(pcId, dx, dy)
                    val nextX = (aimOffset.x + dx * 0.35f).coerceIn(-120f, 120f)
                    val nextY = (aimOffset.y + dy * 0.35f).coerceIn(-120f, 120f)
                    aimOffset = Offset(nextX, nextY)
                }
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }

        gravitySensor?.let { sensorManager.registerListener(listener, it, SensorManager.SENSOR_DELAY_GAME) }
        sensorManager.registerListener(listener, gyroSensor, SensorManager.SENSOR_DELAY_GAME)
        onDispose {
            sensorManager.unregisterListener(listener)
            filter.reset()
            aimOffset = Offset.Zero
        }
    }

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(stringResource(R.string.remote_air_always_on), style = MaterialTheme.typography.bodyMedium)
            androidx.compose.material3.Switch(
                checked = remotePrefs.airAlwaysOn,
                onCheckedChange = { on -> onUpdateRemotePrefs { it.copy(airAlwaysOn = on) } },
            )
        }
        if (!hasGyro) {
            StatusBanner(
                title = stringResource(R.string.remote_air_no_gyro_title),
                text = stringResource(R.string.remote_air_no_gyro_text),
            )
        }

        val primaryColor = MaterialTheme.colorScheme.primary
        val outlineColor = MaterialTheme.colorScheme.outlineVariant
        val airDesc = stringResource(R.string.remote_mode_air)
        Surface(
            shape = MaterialTheme.shapes.extraLarge,
            color = if (padHeld) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
            contentColor = if (padHeld) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
            border = BorderStroke(
                width = if (padHeld) 2.dp else 1.dp,
                color = if (padHeld) primaryColor else outlineColor,
            ),
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .semantics { contentDescription = airDesc }
                .pointerInput(pcId, hasGyro) {
                    val tapSlop = viewConfiguration.touchSlop
                    val tapTimeoutMs = 240L
                    val toReference = TOUCHPAD_REFERENCE_DENSITY / density

                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false)
                        val downTime = System.currentTimeMillis()
                        var maxPointers = 1
                        var totalMoved = 0f
                        // Two fingers dragging scroll, up and down or sideways (#442),
                        // like on the touchpad; the pointer holds still meanwhile.
                        var scrolled = false
                        var twoFingerMoved = 0f
                        var scrollAccumX = 0f
                        var scrollAccumY = 0f

                        filter.reset()
                        aimOffset = Offset.Zero
                        padHeld = hasGyro

                        while (true) {
                            val event = awaitPointerEvent()
                            val changes = event.changes
                            var pressedCount = 0
                            var sumDx = 0f
                            var sumDy = 0f
                            for (i in 0 until changes.size) {
                                val c = changes[i]
                                if (c.pressed) {
                                    pressedCount++
                                    val d = c.positionChange()
                                    totalMoved += hypot(d.x, d.y)
                                    sumDx += d.x
                                    sumDy += d.y
                                }
                            }
                            if (pressedCount == 0) break
                            if (pressedCount > maxPointers) maxPointers = pressedCount
                            // Aim with one finger only: a second one stills the pointer.
                            padHeld = hasGyro && pressedCount == 1 && !scrolled

                            if (pressedCount >= 2) {
                                val avgDx = sumDx / pressedCount
                                val avgDy = sumDy / pressedCount
                                twoFingerMoved += hypot(avgDx, avgDy)
                                if (!scrolled && twoFingerMoved > tapSlop) scrolled = true
                                if (scrolled) {
                                    scrollAccumX += avgDx * toReference
                                    scrollAccumY += avgDy * toReference
                                    if (abs(scrollAccumX) >= 18f || abs(scrollAccumY) >= 18f) {
                                        core.remoteScroll(pcId, -(scrollAccumX / 48f), -(scrollAccumY / 36f), fast = true)
                                        scrollAccumX = 0f
                                        scrollAccumY = 0f
                                    }
                                }
                                for (i in 0 until changes.size) changes[i].consume()
                            }
                        }

                        padHeld = false
                        filter.reset()
                        aimOffset = Offset.Zero

                        val elapsed = System.currentTimeMillis() - downTime
                        if (!scrolled && elapsed <= tapTimeoutMs && totalMoved <= tapSlop * 1.5f) {
                            if (maxPointers == 1) {
                                core.remoteButton(pcId, "left", "click", onStatus)
                            } else if (maxPointers == 2) {
                                core.remoteButton(pcId, "right", "click", onStatus)
                            }
                        }
                    }
                },
        ) {
            Box(Modifier.fillMaxSize()) {
                Canvas(Modifier.fillMaxSize()) {
                    val center = Offset(size.width / 2f, size.height / 2f)
                    val ringColor = if (padHeld) primaryColor.copy(alpha = 0.35f) else outlineColor.copy(alpha = 0.5f)
                    drawCircle(
                        color = ringColor,
                        radius = 44.dp.toPx(),
                        center = center,
                    )
                    val dotCenter = Offset(
                        x = (center.x + aimOffset.x.dp.toPx() * 0.4f).coerceIn(24.dp.toPx(), size.width - 24.dp.toPx()),
                        y = (center.y + aimOffset.y.dp.toPx() * 0.4f).coerceIn(24.dp.toPx(), size.height - 24.dp.toPx()),
                    )
                    drawCircle(
                        color = if (padHeld) primaryColor else primaryColor.copy(alpha = 0.45f),
                        radius = if (padHeld) 10.dp.toPx() else 6.dp.toPx(),
                        center = dotCenter,
                    )
                }

                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier
                        .align(Alignment.TopCenter)
                        .padding(horizontal = 20.dp, vertical = 18.dp),
                ) {
                    Text(
                        text = if (!hasGyro) {
                            stringResource(R.string.remote_air_no_gyro_title)
                        } else if (padHeld) {
                            stringResource(R.string.remote_air_active, pcName)
                        } else {
                            stringResource(R.string.remote_air_hold, pcName)
                        },
                        style = MaterialTheme.typography.titleMedium,
                        textAlign = TextAlign.Center,
                    )
                }

                Text(
                    stringResource(R.string.remote_air_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = if (padHeld) {
                        MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f)
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    textAlign = TextAlign.Center,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(horizontal = 20.dp, vertical = 14.dp),
                )
            }
        }

        // Left / Right click buttons under the air mouse pad
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FilledTonalButton(
                onClick = { core.remoteButton(pcId, "left", "click", onStatus) },
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_left_click), maxLines = 1)
            }
            FilledTonalButton(
                onClick = { core.remoteButton(pcId, "right", "click", onStatus) },
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_right_click), maxLines = 1)
            }
        }

        // Voice typing bar (Dictate button: hold to talk, or tap to start/stop)
        VoiceTypingBar(
            pcId = pcId,
            pcName = pcName,
            core = core,
            utteranceJoiner = utteranceJoiner,
            onStatus = onStatus,
        )
    }
}

private enum class DictateError {
    NoRecognizer,
    PermissionNeeded,
    NoSpeech,
    Failed,
}

private sealed interface DictateState {
    data class Idle(val lastTyped: String? = null) : DictateState
    data class Listening(val partial: String = "") : DictateState
    data class Typing(val text: String) : DictateState
    data class Error(val kind: DictateError) : DictateState
}

@Composable
private fun VoiceTypingBar(
    pcId: String,
    pcName: String,
    core: Core,
    utteranceJoiner: UtteranceJoiner,
    onStatus: (Core.RemoteAccess) -> Unit,
) {
    val context = LocalContext.current
    var state by remember(pcId) { mutableStateOf<DictateState>(DictateState.Idle()) }
    var recognizer by remember { mutableStateOf<SpeechRecognizer?>(null) }
    var preferOnDevice by remember { mutableStateOf(true) }

    fun destroyRecognizer() {
        runCatching {
            recognizer?.cancel()
            recognizer?.destroy()
        }
        recognizer = null
    }

    DisposableEffect(pcId) {
        onDispose {
            destroyRecognizer()
        }
    }

    fun startRecognition(allowOnDevice: Boolean = preferOnDevice) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            state = DictateState.Error(DictateError.PermissionNeeded)
            return
        }

        destroyRecognizer()

        val useOnDevice = allowOnDevice &&
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            runCatching { SpeechRecognizer.isOnDeviceRecognitionAvailable(context) }.getOrDefault(false)

        val defaultAvailable = runCatching { SpeechRecognizer.isRecognitionAvailable(context) }.getOrDefault(false)
        if (!useOnDevice && !defaultAvailable) {
            state = DictateState.Error(DictateError.NoRecognizer)
            return
        }

        val sr = runCatching {
            if (useOnDevice && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
            } else {
                SpeechRecognizer.createSpeechRecognizer(context)
            }
        }.getOrNull()

        if (sr == null) {
            state = DictateState.Error(DictateError.NoRecognizer)
            return
        }

        var readyForSpeech = false
        var latestPartial = ""

        sr.setRecognitionListener(object : RecognitionListener {
            override fun onReadyForSpeech(params: Bundle?) {
                readyForSpeech = true
                state = DictateState.Listening(partial = latestPartial)
            }

            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {}

            override fun onPartialResults(partialResults: Bundle?) {
                val partial = partialResults
                    ?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                    ?.firstOrNull()
                    ?.let { UtteranceJoiner.sanitizeUtterance(it) }
                    .orEmpty()
                if (partial.isNotEmpty()) {
                    latestPartial = partial
                    state = DictateState.Listening(partial = partial)
                }
            }

            override fun onResults(results: Bundle?) {
                val raw = results
                    ?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                    ?.firstOrNull()
                    ?.takeIf { it.isNotBlank() }
                    ?: latestPartial
                val joined = utteranceJoiner.next(raw)
                if (joined.isEmpty()) {
                    state = DictateState.Error(DictateError.NoSpeech)
                    return
                }
                val preview = joined.trimStart()
                state = DictateState.Typing(preview)
                core.remoteText(pcId, joined) { access ->
                    onStatus(access)
                    state = DictateState.Idle(lastTyped = preview)
                }
            }

            override fun onError(error: Int) {
                // If the on-device recognizer fails before speech begins (e.g. its
                // offline model isn't downloaded), fall back to the default service.
                if (useOnDevice && !readyForSpeech) {
                    preferOnDevice = false
                    if (defaultAvailable) {
                        startRecognition(allowOnDevice = false)
                        return
                    }
                }
                if (latestPartial.isNotBlank() &&
                    (error == SpeechRecognizer.ERROR_NO_MATCH || error == SpeechRecognizer.ERROR_SPEECH_TIMEOUT)
                ) {
                    val joined = utteranceJoiner.next(latestPartial)
                    if (joined.isNotEmpty()) {
                        val preview = joined.trimStart()
                        state = DictateState.Typing(preview)
                        core.remoteText(pcId, joined) { access ->
                            onStatus(access)
                            state = DictateState.Idle(lastTyped = preview)
                        }
                        return
                    }
                }
                state = when (error) {
                    SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS ->
                        DictateState.Error(DictateError.PermissionNeeded)
                    SpeechRecognizer.ERROR_NO_MATCH,
                    SpeechRecognizer.ERROR_SPEECH_TIMEOUT ->
                        DictateState.Error(DictateError.NoSpeech)
                    SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
                    SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE,
                    SpeechRecognizer.ERROR_CANNOT_CHECK_SUPPORT ->
                        DictateState.Error(DictateError.NoRecognizer)
                    else ->
                        if (!readyForSpeech && error != SpeechRecognizer.ERROR_RECOGNIZER_BUSY) {
                            DictateState.Error(DictateError.NoRecognizer)
                        } else {
                            DictateState.Error(DictateError.Failed)
                        }
                }
            }

            override fun onEvent(eventType: Int, params: Bundle?) {}
        })

        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
        }

        recognizer = sr
        state = DictateState.Listening(partial = "")
        runCatching {
            sr.startListening(intent)
        }.onFailure {
            destroyRecognizer()
            state = DictateState.Error(DictateError.NoRecognizer)
        }
    }

    fun stopRecognition() {
        val sr = recognizer
        if (sr != null) {
            runCatching { sr.stopListening() }
        } else if (state is DictateState.Listening) {
            state = DictateState.Idle()
        }
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            startRecognition()
        } else {
            state = DictateState.Error(DictateError.PermissionNeeded)
        }
    }

    val isListening = state is DictateState.Listening
    val isError = state is DictateState.Error

    val containerColor = when {
        isListening -> MaterialTheme.colorScheme.primaryContainer
        isError -> MaterialTheme.colorScheme.errorContainer
        else -> MaterialTheme.colorScheme.surfaceContainer
    }
    val contentColor = when {
        isListening -> MaterialTheme.colorScheme.onPrimaryContainer
        isError -> MaterialTheme.colorScheme.onErrorContainer
        else -> MaterialTheme.colorScheme.onSurface
    }

    Surface(
        shape = MaterialTheme.shapes.large,
        color = containerColor,
        contentColor = contentColor,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                val statusText = when (val s = state) {
                    is DictateState.Idle -> if (s.lastTyped != null) {
                        stringResource(R.string.remote_dictate_typed, pcName, s.lastTyped)
                    } else {
                        stringResource(R.string.remote_dictate_hint, pcName)
                    }
                    is DictateState.Listening -> stringResource(R.string.remote_dictate_listening, pcName)
                    is DictateState.Typing -> stringResource(R.string.remote_dictate_typing, pcName)
                    is DictateState.Error -> when (s.kind) {
                        DictateError.NoRecognizer -> stringResource(R.string.remote_dictate_no_recognizer)
                        DictateError.PermissionNeeded -> stringResource(R.string.remote_dictate_permission_needed, pcName)
                        DictateError.NoSpeech -> stringResource(R.string.remote_dictate_no_speech)
                        DictateError.Failed -> stringResource(R.string.remote_dictate_error)
                    }
                }
                Text(
                    text = statusText,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )

                val partialOrTyping = when (val s = state) {
                    is DictateState.Listening -> s.partial
                    is DictateState.Typing -> s.text
                    else -> ""
                }
                if (partialOrTyping.isNotEmpty()) {
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = partialOrTyping,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }

            // Dictate button: supports both tap to start/stop and hold to talk.
            val currentIsListening by rememberUpdatedState(isListening)
            val buttonBg = if (isListening) {
                MaterialTheme.colorScheme.primary
            } else {
                MaterialTheme.colorScheme.secondaryContainer
            }
            val buttonFg = if (isListening) {
                MaterialTheme.colorScheme.onPrimary
            } else {
                MaterialTheme.colorScheme.onSecondaryContainer
            }
            val dictateLabel = stringResource(
                if (isListening) R.string.remote_dictate_stop else R.string.remote_dictate,
            )

            Surface(
                shape = CircleShape,
                color = buttonBg,
                contentColor = buttonFg,
                modifier = Modifier
                    .height(40.dp)
                    .semantics(mergeDescendants = true) {
                        role = Role.Button
                        contentDescription = dictateLabel
                        onClick(label = dictateLabel) {
                            if (currentIsListening) {
                                stopRecognition()
                            } else if (
                                ContextCompat.checkSelfPermission(
                                    context,
                                    Manifest.permission.RECORD_AUDIO,
                                ) == PackageManager.PERMISSION_GRANTED
                            ) {
                                startRecognition()
                            } else {
                                permissionLauncher.launch(Manifest.permission.RECORD_AUDIO)
                            }
                            true
                        }
                    }
                    .pointerInput(pcId) {
                        val holdThresholdMs = 350L
                        awaitEachGesture {
                            awaitFirstDown(requireUnconsumed = false)
                            val downTime = System.currentTimeMillis()
                            val wasListening = currentIsListening

                            if (wasListening) {
                                while (true) {
                                    val ev = awaitPointerEvent()
                                    if (ev.changes.none { it.pressed }) break
                                }
                                stopRecognition()
                            } else {
                                val hasPerm = ContextCompat.checkSelfPermission(
                                    context,
                                    Manifest.permission.RECORD_AUDIO,
                                ) == PackageManager.PERMISSION_GRANTED

                                if (!hasPerm) {
                                    while (true) {
                                        val ev = awaitPointerEvent()
                                        if (ev.changes.none { it.pressed }) break
                                    }
                                    permissionLauncher.launch(Manifest.permission.RECORD_AUDIO)
                                } else {
                                    startRecognition()
                                    while (true) {
                                        val ev = awaitPointerEvent()
                                        if (ev.changes.none { it.pressed }) break
                                    }
                                    val heldMs = System.currentTimeMillis() - downTime
                                    if (heldMs >= holdThresholdMs) {
                                        stopRecognition()
                                    }
                                }
                            }
                        }
                    },
            ) {
                Box(
                    contentAlignment = Alignment.Center,
                    modifier = Modifier.padding(horizontal = 16.dp),
                ) {
                    Text(
                        text = dictateLabel,
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

@Composable
private fun SpecialKeysBar(
    activeMods: List<String>,
    showExtraKeys: Boolean,
    onToggleMod: (String) -> Unit,
    onKey: (key: String, forceMods: List<String>?) -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        // Modifier keys row: 4 equal columns that fit at every font scale without overflowing
        val mods = listOf(
            "ctrl" to stringResource(R.string.remote_key_ctrl),
            "alt" to stringResource(R.string.remote_key_alt),
            "shift" to stringResource(R.string.remote_key_shift),
            "win" to stringResource(R.string.remote_key_win),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            for ((id, label) in mods) {
                val selected = id in activeMods
                Surface(
                    shape = MaterialTheme.shapes.small,
                    color = if (selected) {
                        MaterialTheme.colorScheme.secondaryContainer
                    } else {
                        MaterialTheme.colorScheme.surfaceContainer
                    },
                    contentColor = if (selected) {
                        MaterialTheme.colorScheme.onSecondaryContainer
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    },
                    border = BorderStroke(
                        width = 1.dp,
                        color = if (selected) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            MaterialTheme.colorScheme.outlineVariant
                        },
                    ),
                    modifier = Modifier
                        .weight(1f)
                        .height(36.dp)
                        .semantics(mergeDescendants = true) {
                            role = Role.Switch
                            this.selected = selected
                            contentDescription = label
                        }
                        .clickable { onToggleMod(id) },
                ) {
                    Box(contentAlignment = Alignment.Center, modifier = Modifier.padding(horizontal = 4.dp)) {
                        Text(
                            text = label,
                            style = MaterialTheme.typography.labelMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }

        // Navigation & special keys row: 6 equal columns across the full width
        val keys = listOf(
            Triple("escape", stringResource(R.string.remote_key_esc), stringResource(R.string.remote_key_esc_desc)),
            Triple("tab", stringResource(R.string.remote_key_tab), stringResource(R.string.remote_key_tab)),
            Triple("left", "←", stringResource(R.string.remote_key_left_desc)),
            Triple("up", "↑", stringResource(R.string.remote_key_up_desc)),
            Triple("down", "↓", stringResource(R.string.remote_key_down_desc)),
            Triple("right", "→", stringResource(R.string.remote_key_right_desc)),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            for ((key, label, a11yLabel) in keys) {
                Surface(
                    shape = MaterialTheme.shapes.small,
                    color = MaterialTheme.colorScheme.surface,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                    modifier = Modifier
                        .weight(1f)
                        .height(36.dp)
                        .semantics(mergeDescendants = true) {
                            role = Role.Button
                            contentDescription = a11yLabel
                        }
                        .clickable(role = Role.Button) { onKey(key, null) },
                ) {
                    Box(contentAlignment = Alignment.Center, modifier = Modifier.padding(horizontal = 2.dp)) {
                        Text(
                            text = label,
                            style = MaterialTheme.typography.labelMedium,
                            maxLines = 1,
                        )
                    }
                }
            }
        }

        if (showExtraKeys) {
            val extra = listOf(
                Triple("home", stringResource(R.string.remote_key_home), stringResource(R.string.remote_key_home)),
                Triple("end", stringResource(R.string.remote_key_end), stringResource(R.string.remote_key_end)),
                Triple("delete", stringResource(R.string.remote_key_del), stringResource(R.string.remote_key_del_desc)),
                Triple("copy", stringResource(R.string.remote_key_copy), stringResource(R.string.remote_key_copy)),
                Triple("paste", stringResource(R.string.remote_key_paste), stringResource(R.string.remote_key_paste)),
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                for ((key, label, a11yLabel) in extra) {
                    val isShortcut = key == "copy" || key == "paste"
                    Surface(
                        shape = MaterialTheme.shapes.small,
                        color = MaterialTheme.colorScheme.surface,
                        contentColor = MaterialTheme.colorScheme.onSurface,
                        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                        modifier = Modifier
                            .weight(1f)
                            .height(36.dp)
                            .semantics(mergeDescendants = true) {
                                role = Role.Button
                                contentDescription = a11yLabel
                            }
                            .clickable(role = Role.Button) { onKey(key, if (isShortcut) emptyList() else null) },
                    ) {
                        Box(contentAlignment = Alignment.Center, modifier = Modifier.padding(horizontal = 2.dp)) {
                            Text(
                                text = label,
                                style = MaterialTheme.typography.labelMedium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PresentationView(
    pcId: String,
    pcName: String,
    core: Core,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var laserHeld by remember { mutableStateOf(false) }
    val laserXState = remember { mutableFloatStateOf(0.5f) }
    val laserYState = remember { mutableFloatStateOf(0.5f) }

    // While held, say it again every second even if it hasn't moved: the PC
    // hides a dot that stops hearing from its phone.
    LaunchedEffect(laserHeld, pcId) {
        while (laserHeld) {
            delay(1000)
            core.remoteLaser(pcId, true, laserXState.floatValue, laserYState.floatValue)
        }
    }

    // Follow the phone's gyroscope / rotation vector while the laser button is held.
    DisposableEffect(laserHeld, pcId) {
        if (!laserHeld) return@DisposableEffect onDispose {}

        val sensorManager = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
        val rotationSensor = sensorManager?.getDefaultSensor(Sensor.TYPE_GAME_ROTATION_VECTOR)
            ?: sensorManager?.getDefaultSensor(Sensor.TYPE_ROTATION_VECTOR)

        var baseYaw: Float? = null
        var basePitch: Float? = null
        val rotationMatrix = FloatArray(9)
        val orientation = FloatArray(3)

        val listener = object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                SensorManager.getRotationMatrixFromVector(rotationMatrix, event.values)
                SensorManager.getOrientation(rotationMatrix, orientation)
                val yaw = orientation[0]
                val pitch = orientation[1]
                val bYaw = baseYaw ?: yaw.also { baseYaw = it }
                val bPitch = basePitch ?: pitch.also { basePitch = it }

                // ~35 degrees (~0.6 rad) full sweep across the screen
                var dYaw = yaw - bYaw
                if (dYaw > Math.PI.toFloat()) dYaw -= (2.0 * Math.PI).toFloat()
                if (dYaw < -Math.PI.toFloat()) dYaw += (2.0 * Math.PI).toFloat()
                val dPitch = pitch - bPitch

                val nx = (0.5f + dYaw / 0.65f).coerceIn(0f, 1f)
                val ny = (0.5f + dPitch / 0.50f).coerceIn(0f, 1f)
                laserXState.floatValue = nx
                laserYState.floatValue = ny
                core.remoteLaser(pcId, true, nx, ny)
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }

        if (rotationSensor != null) {
            sensorManager?.registerListener(listener, rotationSensor, SensorManager.SENSOR_DELAY_GAME)
        }
        onDispose {
            sensorManager?.unregisterListener(listener)
        }
    }

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // Huge Next Slide button (easy to hit without looking)
        Button(
            onClick = { core.remoteSlide(pcId, "next", onStatus) },
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxWidth()
                .height(96.dp),
        ) {
            Text(
                stringResource(R.string.remote_slide_next),
                style = MaterialTheme.typography.headlineSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }

        // Previous Slide button
        FilledTonalButton(
            onClick = { core.remoteSlide(pcId, "previous", onStatus) },
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxWidth()
                .height(64.dp),
        ) {
            Text(
                stringResource(R.string.remote_slide_prev),
                style = MaterialTheme.typography.titleLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }

        // Start / End / Black screen row
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "start", onStatus) },
                contentPadding = PaddingValues(horizontal = 6.dp, vertical = 8.dp),
                modifier = Modifier.weight(0.98f).height(48.dp),
            ) {
                Text(
                    stringResource(R.string.remote_slide_start),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "stop", onStatus) },
                contentPadding = PaddingValues(horizontal = 6.dp, vertical = 8.dp),
                modifier = Modifier.weight(0.88f).height(48.dp),
            ) {
                Text(
                    stringResource(R.string.remote_slide_stop),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "black", onStatus) },
                contentPadding = PaddingValues(horizontal = 6.dp, vertical = 8.dp),
                modifier = Modifier.weight(1.22f).height(48.dp),
            ) {
                Text(
                    stringResource(R.string.remote_slide_black),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }

        // Hold for laser pointer pad (supports both gyroscope tilt and finger drag)
        val primaryColor = MaterialTheme.colorScheme.primary
        val laserDesc = stringResource(
            if (laserHeld) R.string.remote_laser_active else R.string.remote_laser_hold,
            pcName,
        )
        Surface(
            shape = MaterialTheme.shapes.extraLarge,
            color = if (laserHeld) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
            contentColor = if (laserHeld) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
            border = BorderStroke(
                width = if (laserHeld) 2.dp else 1.dp,
                color = if (laserHeld) primaryColor else MaterialTheme.colorScheme.outlineVariant,
            ),
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .semantics { contentDescription = laserDesc }
                .pointerInput(pcId) {
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val w = size.width.coerceAtLeast(1).toFloat()
                        val h = size.height.coerceAtLeast(1).toFloat()
                        val initX = (down.position.x / w).coerceIn(0f, 1f)
                        val initY = (down.position.y / h).coerceIn(0f, 1f)
                        laserXState.floatValue = initX
                        laserYState.floatValue = initY
                        laserHeld = true
                        core.remoteLaser(pcId, true, initX, initY)

                        while (true) {
                            val event = awaitPointerEvent()
                            val active = event.changes.firstOrNull { it.pressed } ?: break
                            val d = active.positionChange()
                            if (hypot(d.x, d.y) > 0.5f) {
                                val nx = (active.position.x / w).coerceIn(0f, 1f)
                                val ny = (active.position.y / h).coerceIn(0f, 1f)
                                laserXState.floatValue = nx
                                laserYState.floatValue = ny
                                core.remoteLaser(pcId, true, nx, ny)
                                active.consume()
                            }
                        }

                        laserHeld = false
                        core.remoteLaser(pcId, false)
                    }
                },
        ) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (laserHeld) {
                    Canvas(Modifier.fillMaxSize()) {
                        val pt = Offset(laserXState.floatValue * size.width, laserYState.floatValue * size.height)
                        drawCircle(
                            color = primaryColor.copy(alpha = 0.28f),
                            radius = 26.dp.toPx(),
                            center = pt,
                        )
                        drawCircle(
                            color = primaryColor,
                            radius = 8.dp.toPx(),
                            center = pt,
                        )
                    }
                }
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.padding(20.dp),
                ) {
                    Text(
                        stringResource(
                            if (laserHeld) R.string.remote_laser_active else R.string.remote_laser_hold,
                            pcName,
                        ),
                        style = MaterialTheme.typography.titleMedium,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(6.dp))
                    Text(
                        stringResource(R.string.remote_volume_hint),
                        style = MaterialTheme.typography.bodySmall,
                        color = if (laserHeld) {
                            MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f)
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

/** Screen map mode: touch jumps directly across the union of monitors; hold 380 ms switches to fine relative control. */
@Composable
private fun ScreenMapView(
    pcId: String,
    core: Core,
    remoteInfo: app.nectarlink.core.RemoteInfo?,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences,
    onAction: (TouchpadAction) -> Unit,
    onHaptic: (TouchpadHaptic) -> Unit,
    onGestureStolen: () -> Unit,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    val monitors = remember(remoteInfo) {
        remoteInfo?.monitors?.map {
            MonitorRect(it.name, it.x, it.y, it.width.toInt(), it.height.toInt(), it.primary)
        }.orEmpty()
    }
    var seq by remember(pcId) { mutableStateOf(0u) }
    var cursorNorm by remember(pcId) { mutableStateOf(0.5f to 0.5f) }
    var fineMode by remember { mutableStateOf(false) }
    val primaryColor = MaterialTheme.colorScheme.primary
    val outlineColor = MaterialTheme.colorScheme.outlineVariant

    Column(modifier = modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        BoxWithConstraints(modifier = Modifier.weight(1f).fillMaxWidth()) {
            val transform = remember(monitors, maxWidth, maxHeight) {
                ScreenMapTransform(monitors, maxWidth.value, maxHeight.value)
            }
            val options = remember(remotePrefs) {
                TouchpadGestureOptions(
                    pointerSpeed = remotePrefs.pointerSpeed,
                    acceleration = remotePrefs.acceleration,
                    scrollSpeed = remotePrefs.scrollSpeed,
                    tapToClick = remotePrefs.tapToClick,
                    threeFingerMiddleClick = remotePrefs.threeFingerMiddleClick,
                    naturalScroll = remotePrefs.naturalScrolling,
                    momentum = remotePrefs.momentum,
                    relativePointer = false,
                )
            }
            Surface(
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                border = BorderStroke(if (fineMode) 2.dp else 1.dp, if (fineMode) primaryColor else outlineColor),
                modifier = Modifier
                    .fillMaxSize()
                    .pointerInput(pcId, transform, options) {
                        var startTimeMs = 0L
                        var startTravel = 0f
                        var fineActive = false
                        touchpadGestures(
                            pcId = pcId,
                            core = core,
                            sensitivity = 1f,
                            onStatus = onStatus,
                            options = options,
                            onAction = onAction,
                            onZoom = { dy -> core.remoteZoom(pcId, -dy) },
                            onHaptic = onHaptic,
                            onGestureStolen = onGestureStolen,
                            onTap = {
                                seq += 1u
                                core.remoteMoveAbsolute(pcId, cursorNorm.first, cursorNorm.second, seq, click = "left", onStatus = onStatus)
                            },
                            onSingleFinger = { ev ->
                                when (ev) {
                                    is TouchpadSingleFinger.Start -> {
                                        startTimeMs = android.os.SystemClock.uptimeMillis()
                                        startTravel = 0f
                                        fineActive = false
                                        fineMode = false
                                        val norm = transform.normalizedAt(ev.contact.xDp, ev.contact.yDp)
                                        cursorNorm = norm
                                        seq += 1u
                                        core.remoteMoveAbsolute(pcId, norm.first, norm.second, seq, click = null)
                                    }
                                    is TouchpadSingleFinger.Move -> {
                                        startTravel += hypot(ev.dxDp, ev.dyDp)
                                        val elapsed = android.os.SystemClock.uptimeMillis() - startTimeMs
                                        if (!fineActive && elapsed >= 380L && startTravel < 7f) {
                                            fineActive = true
                                            fineMode = true
                                            onHaptic(TouchpadHaptic.Tick)
                                        }
                                        if (fineActive) {
                                            core.remoteMovePrecise(pcId, ev.dxDp * 0.25f * remotePrefs.pointerSpeed, ev.dyDp * 0.25f * remotePrefs.pointerSpeed)
                                        } else {
                                            val norm = transform.normalizedAt(ev.contact.xDp, ev.contact.yDp)
                                            cursorNorm = norm
                                            seq += 1u
                                            core.remoteMoveAbsolute(pcId, norm.first, norm.second, seq, click = null)
                                        }
                                    }
                                    TouchpadSingleFinger.End, TouchpadSingleFinger.Cancel -> {
                                        fineActive = false
                                        fineMode = false
                                    }
                                }
                            },
                        )
                    },
            ) {
                Box(Modifier.fillMaxSize()) {
                    Canvas(Modifier.fillMaxSize()) {
                        for (m in transform.monitors) {
                            val r = transform.monitorBounds(m)
                            drawRoundRect(
                                color = if (m.primary) primaryColor.copy(alpha = 0.16f) else outlineColor.copy(alpha = 0.18f),
                                topLeft = Offset(r.x.dp.toPx(), r.y.dp.toPx()),
                                size = androidx.compose.ui.geometry.Size(r.width.dp.toPx(), r.height.dp.toPx()),
                                cornerRadius = androidx.compose.ui.geometry.CornerRadius(8.dp.toPx()),
                            )
                        }
                        val (cx, cy) = transform.padPosition(cursorNorm.first, cursorNorm.second)
                        drawCircle(
                            color = primaryColor,
                            radius = 6.dp.toPx(),
                            center = Offset(cx.dp.toPx(), cy.dp.toPx()),
                        )
                    }
                    Text(
                        stringResource(R.string.remote_map_hint),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                    )
                }
            }
        }
        RemoteMouseButtonsRow(pcId = pcId, core = core, onStatus = onStatus)
    }
}

/** Desk mouse mode: phone flat on desk acts as an optical mouse with holdable L/R halves and center scroll wheel. */
@Composable
private fun DeskMouseView(
    pcId: String,
    core: Core,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences,
    onHaptic: (TouchpadHaptic) -> Unit,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val sensorManager = remember(context) { context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager }
    val tracker = remember(sensorManager) { sensorManager?.let { DeskTracker(it) } }
    var deskStatus by remember { mutableStateOf(DeskStatus.Ready) }

    LaunchedEffect(tracker, remotePrefs.deskSpeed, remotePrefs.deskSensitivity, remotePrefs.deskTwist) {
        tracker?.apply {
            deskSpeed = remotePrefs.deskSpeed
            sensitivity = remotePrefs.deskSensitivity
            deskTwist = remotePrefs.deskTwist
        }
    }

    DisposableEffect(tracker, pcId) {
        if (tracker == null || !tracker.isAvailable) return@DisposableEffect onDispose {}
        tracker.start()
        onDispose { tracker.stop() }
    }

    LaunchedEffect(tracker, pcId) {
        if (tracker == null || !tracker.isAvailable) return@LaunchedEffect
        var prevStatus = DeskStatus.Ready
        while (true) {
            delay(16L)
            val frame = tracker.readFrame()
            if (prevStatus == DeskStatus.Lifted && frame.status == DeskStatus.Ready) {
                onHaptic(TouchpadHaptic.Soft)
            }
            prevStatus = frame.status
            deskStatus = frame.status
            if (abs(frame.dx) >= 0.05f || abs(frame.dy) >= 0.05f) {
                core.remoteMovePrecise(pcId, frame.dx, frame.dy)
            }
        }
    }

    Column(modifier = modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        if (tracker == null || !tracker.isAvailable) {
            StatusBanner(title = null, text = stringResource(R.string.remote_desk_unavailable))
            return@Column
        }
        val statusRes = when (deskStatus) {
            DeskStatus.Ready -> R.string.remote_desk_ready
            DeskStatus.Tracking -> R.string.remote_desk_tracking
            DeskStatus.Lifted -> R.string.remote_desk_lifted
            DeskStatus.Tilted -> R.string.remote_desk_tilted
        }
        Text(
            stringResource(statusRes),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        )
        Row(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            HoldableMouseHalf(
                label = stringResource(R.string.remote_btn_left),
                onDown = { core.remoteButton(pcId, "left", "down", onStatus); onHaptic(TouchpadHaptic.Click) },
                onUp = { core.remoteButton(pcId, "left", "up", onStatus) },
                modifier = Modifier.weight(1f).fillMaxSize(),
            )
            Surface(
                shape = MaterialTheme.shapes.large,
                color = MaterialTheme.colorScheme.surfaceContainerHighest,
                modifier = Modifier
                    .width(64.dp)
                    .fillMaxSize()
                    .pointerInput(pcId, remotePrefs.scrollSpeed, remotePrefs.naturalScrolling) {
                        awaitEachGesture {
                            val down = awaitFirstDown(requireUnconsumed = false)
                            val downMs = down.uptimeMillis
                            var totalY = 0f
                            var moved = 0f
                            while (true) {
                                val ev = awaitPointerEvent()
                                val c = ev.changes.firstOrNull { it.id == down.id } ?: break
                                if (!c.pressed) break
                                val d = c.positionChange()
                                moved += abs(d.y)
                                totalY += d.y / density
                                if (abs(totalY) >= 8f) {
                                    val (_, dyNotch) = TouchpadBallistics.scrollNotches(
                                        0f,
                                        totalY,
                                        remotePrefs.scrollSpeed,
                                        remotePrefs.naturalScrolling,
                                    )
                                    core.remoteScroll(pcId, 0f, dyNotch, fast = true)
                                    totalY = 0f
                                }
                                c.consume()
                            }
                            if (moved < 8f * density && (android.os.SystemClock.uptimeMillis() - downMs) < 260L) {
                                core.remoteButton(pcId, "middle", "click", onStatus)
                                onHaptic(TouchpadHaptic.Click)
                            }
                        }
                    },
            ) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text("⇅", style = MaterialTheme.typography.titleMedium)
                }
            }
            HoldableMouseHalf(
                label = stringResource(R.string.remote_btn_right),
                onDown = { core.remoteButton(pcId, "right", "down", onStatus); onHaptic(TouchpadHaptic.Click) },
                onUp = { core.remoteButton(pcId, "right", "up", onStatus) },
                modifier = Modifier.weight(1f).fillMaxSize(),
            )
        }
    }
}

@Composable
private fun HoldableMouseHalf(
    label: String,
    onDown: () -> Unit,
    onUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var pressed by remember { mutableStateOf(false) }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = if (pressed) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.pointerInput(Unit) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                pressed = true
                onDown()
                try {
                    while (true) {
                        val ev = awaitPointerEvent()
                        val c = ev.changes.firstOrNull { it.id == down.id } ?: break
                        if (!c.pressed) break
                        c.consume()
                    }
                } finally {
                    pressed = false
                    onUp()
                }
            }
        },
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text(label, style = MaterialTheme.typography.titleMedium)
        }
    }
}

/** Floating virtual thumbstick mode using Drift's 64 dp stick curve. */
@Composable
private fun StickView(
    pcId: String,
    core: Core,
    remotePrefs: app.nectarlink.android.ui.RemotePreferences,
    onAction: (TouchpadAction) -> Unit,
    onHaptic: (TouchpadHaptic) -> Unit,
    onGestureStolen: () -> Unit,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    var originDp by remember { mutableStateOf<Offset?>(null) }
    var thumbDp by remember { mutableStateOf(Offset.Zero) }
    val primaryColor = MaterialTheme.colorScheme.primary
    val outlineColor = MaterialTheme.colorScheme.outlineVariant

    LaunchedEffect(pcId, originDp, remotePrefs.stickSpeed) {
        if (originDp == null) return@LaunchedEffect
        while (originDp != null) {
            delay(16L)
            val (vx, vy) = StickCurve.velocity(thumbDp.x, thumbDp.y, remotePrefs.stickSpeed)
            val dx = vx * 0.016f
            val dy = vy * 0.016f
            if (abs(dx) >= 0.05f || abs(dy) >= 0.05f) {
                core.remoteMovePrecise(pcId, dx, dy)
            }
        }
    }

    Column(modifier = modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        val options = remember(remotePrefs) {
            TouchpadGestureOptions(
                pointerSpeed = remotePrefs.pointerSpeed,
                acceleration = remotePrefs.acceleration,
                scrollSpeed = remotePrefs.scrollSpeed,
                tapToClick = remotePrefs.tapToClick,
                threeFingerMiddleClick = remotePrefs.threeFingerMiddleClick,
                naturalScroll = remotePrefs.naturalScrolling,
                momentum = remotePrefs.momentum,
                relativePointer = false,
            )
        }
        Surface(
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            border = BorderStroke(1.dp, outlineColor),
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .pointerInput(pcId, options) {
                    var atRim = false
                    touchpadGestures(
                        pcId = pcId,
                        core = core,
                        sensitivity = 1f,
                        onStatus = onStatus,
                        options = options,
                        onAction = onAction,
                        onZoom = { dy -> core.remoteZoom(pcId, -dy) },
                        onHaptic = onHaptic,
                        onGestureStolen = onGestureStolen,
                        onTap = { core.remoteButton(pcId, "left", "click", onStatus) },
                        onSingleFinger = { ev ->
                            when (ev) {
                                is TouchpadSingleFinger.Start -> {
                                    originDp = Offset(ev.contact.xDp, ev.contact.yDp)
                                    thumbDp = Offset.Zero
                                    atRim = false
                                }
                                is TouchpadSingleFinger.Move -> {
                                    val o = originDp ?: return@touchpadGestures
                                    val rawX = ev.contact.xDp - o.x
                                    val rawY = ev.contact.yDp - o.y
                                    val dist = hypot(rawX, rawY)
                                    val clamped = if (dist > StickCurve.RADIUS_DP) {
                                        if (!atRim) {
                                            atRim = true
                                            onHaptic(TouchpadHaptic.Soft)
                                        }
                                        val s = StickCurve.RADIUS_DP / dist
                                        Offset(rawX * s, rawY * s)
                                    } else {
                                        atRim = false
                                        Offset(rawX, rawY)
                                    }
                                    thumbDp = clamped
                                }
                                TouchpadSingleFinger.End, TouchpadSingleFinger.Cancel -> {
                                    originDp = null
                                    thumbDp = Offset.Zero
                                    atRim = false
                                }
                            }
                        },
                    )
                },
        ) {
            Box(Modifier.fillMaxSize()) {
                Canvas(Modifier.fillMaxSize()) {
                    val o = originDp ?: return@Canvas
                    val center = Offset(o.x.dp.toPx(), o.y.dp.toPx())
                    val knob = Offset((o.x + thumbDp.x).dp.toPx(), (o.y + thumbDp.y).dp.toPx())
                    drawCircle(
                        color = primaryColor.copy(alpha = 0.14f),
                        radius = StickCurve.RADIUS_DP.dp.toPx(),
                        center = center,
                    )
                    drawCircle(
                        color = primaryColor,
                        radius = 18.dp.toPx(),
                        center = knob,
                    )
                }
                Text(
                    stringResource(R.string.remote_stick_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                )
            }
        }
        RemoteMouseButtonsRow(pcId = pcId, core = core, onStatus = onStatus)
    }
}

@Composable
private fun RemoteMouseButtonsRow(
    pcId: String,
    core: Core,
    onStatus: (Core.RemoteAccess) -> Unit,
) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(
            onClick = { core.remoteButton(pcId, "left", "click", onStatus) },
            modifier = Modifier.weight(1f).height(46.dp),
        ) { Text(stringResource(R.string.remote_btn_left)) }
        OutlinedButton(
            onClick = { core.remoteButton(pcId, "middle", "click", onStatus) },
            modifier = Modifier.weight(1f).height(46.dp),
        ) { Text(stringResource(R.string.remote_btn_middle)) }
        FilledTonalButton(
            onClick = { core.remoteButton(pcId, "right", "click", onStatus) },
            modifier = Modifier.weight(1f).height(46.dp),
        ) { Text(stringResource(R.string.remote_btn_right)) }
    }
}

/** The screen density (xxhdpi) the touchpad speeds and scroll steps were tuned at. */
