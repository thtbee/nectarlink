// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Build
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlin.math.hypot

enum class RemoteMode { Touchpad, AirMouse, Presentation }

@Composable
fun RemoteScreen(
    device: Device,
    core: Core,
    sensitivity: Float,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    initialMode: RemoteMode = RemoteMode.Touchpad,
) {
    val view = LocalView.current
    val context = LocalContext.current

    // Keep the phone's screen on while controlling the PC, and release any
    // held laser/button when leaving.
    DisposableEffect(view, device.id) {
        val window = (context as? Activity)?.window
        val prevKeepOn = view.keepScreenOn
        view.keepScreenOn = true
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            view.keepScreenOn = prevKeepOn
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            core.remoteLaser(device.id, false)
        }
    }

    var mode by rememberSaveable(initialMode) { mutableStateOf(initialMode) }
    var access by remember(device.id) { mutableStateOf(Core.RemoteAccess.Allowed) }
    val utteranceJoiner = remember(device.id) { UtteranceJoiner() }

    // Check access on entry and poll while denied so the prompt banner clears
    // as soon as the user clicks Allow on the PC.
    LaunchedEffect(device.id, device.online) {
        if (!device.online) {
            access = Core.RemoteAccess.Offline
            return@LaunchedEffect
        }
        access = core.remoteCheck(device.id)
        while (access == Core.RemoteAccess.Denied && device.online) {
            delay(1_500)
            access = core.remoteCheck(device.id)
        }
    }

    val onStatus: (Core.RemoteAccess) -> Unit = { next -> access = next }

    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Header: Back, PC name, online dot
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(
                onClick = onBack,
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            ) {
                Text("← " + stringResource(R.string.action_back), style = MaterialTheme.typography.labelLarge)
            }
            Spacer(Modifier.width(4.dp))
            Text(
                device.name,
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f),
            )
            Surface(
                shape = CircleShape,
                color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                modifier = Modifier.size(10.dp),
            ) {}
        }

        // Mode switcher: Touchpad | Air mouse | Presentation
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            SegmentedButton(
                selected = mode == RemoteMode.Touchpad,
                onClick = { mode = RemoteMode.Touchpad },
                shape = SegmentedButtonDefaults.itemShape(0, 3),
            ) {
                Text(stringResource(R.string.remote_mode_touchpad), maxLines = 1)
            }
            SegmentedButton(
                selected = mode == RemoteMode.AirMouse,
                onClick = { mode = RemoteMode.AirMouse },
                shape = SegmentedButtonDefaults.itemShape(1, 3),
            ) {
                Text(stringResource(R.string.remote_mode_air), maxLines = 1)
            }
            SegmentedButton(
                selected = mode == RemoteMode.Presentation,
                onClick = { mode = RemoteMode.Presentation },
                shape = SegmentedButtonDefaults.itemShape(2, 3),
            ) {
                Text(stringResource(R.string.remote_mode_presentation), maxLines = 1)
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

        when (mode) {
            RemoteMode.Touchpad -> TouchpadView(
                pcId = device.id,
                pcName = device.name,
                core = core,
                sensitivity = sensitivity,
                utteranceJoiner = utteranceJoiner,
                onStatus = onStatus,
                modifier = Modifier.weight(1f),
            )
            RemoteMode.AirMouse -> AirMouseView(
                pcId = device.id,
                pcName = device.name,
                core = core,
                sensitivity = sensitivity,
                utteranceJoiner = utteranceJoiner,
                onStatus = onStatus,
                modifier = Modifier.weight(1f),
            )
            RemoteMode.Presentation -> PresentationView(
                pcId = device.id,
                pcName = device.name,
                core = core,
                onStatus = onStatus,
                modifier = Modifier.weight(1f),
            )
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
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
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

@Composable
private fun TouchpadView(
    pcId: String,
    pcName: String,
    core: Core,
    sensitivity: Float,
    utteranceJoiner: UtteranceJoiner,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    var showKeyboard by rememberSaveable { mutableStateOf(false) }
    var textBuffer by remember { mutableStateOf("") }
    val activeMods = remember { mutableStateListOf<String>() }
    var dragging by remember { mutableStateOf(false) }
    var touchPoint by remember { mutableStateOf<Offset?>(null) }

    val focusRequester = remember { FocusRequester() }
    val keyboardController = LocalSoftwareKeyboardController.current

    LaunchedEffect(showKeyboard) {
        if (showKeyboard) {
            focusRequester.requestFocus()
            keyboardController?.show()
        } else {
            keyboardController?.hide()
        }
    }

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // Big touchpad surface
        val primaryColor = MaterialTheme.colorScheme.primary
        val outlineColor = MaterialTheme.colorScheme.outlineVariant
        Surface(
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            border = BorderStroke(
                width = if (dragging) 2.dp else 1.dp,
                color = if (dragging) primaryColor else outlineColor,
            ),
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .pointerInput(pcId, sensitivity) {
                    val tapSlop = viewConfiguration.touchSlop
                    val tapTimeoutMs = 240L
                    val doubleTapWindowMs = 260L
                    var lastSingleTapTime = 0L

                    awaitEachGesture {
                        val firstDown = awaitFirstDown(requireUnconsumed = false)
                        val downTime = System.currentTimeMillis()
                        var maxPointers = 1
                        var totalMoved = 0f
                        var scrollAccumX = 0f
                        var scrollAccumY = 0f
                        var isDragHold = (downTime - lastSingleTapTime) <= doubleTapWindowMs
                        var dragStarted = false

                        touchPoint = firstDown.position

                        while (true) {
                            val event = awaitPointerEvent()
                            val pressed = event.changes.filter { it.pressed }
                            if (pressed.isEmpty()) break

                            if (pressed.size > maxPointers) {
                                maxPointers = pressed.size
                                if (dragStarted) {
                                    core.remoteButton(pcId, "left", "up", onStatus)
                                    dragStarted = false
                                    dragging = false
                                }
                                isDragHold = false
                            }

                            if (pressed.size == 1 && maxPointers == 1) {
                                val change = pressed[0]
                                touchPoint = change.position
                                val delta = change.positionChange()
                                val dist = hypot(delta.x, delta.y)
                                totalMoved += dist

                                if (isDragHold && !dragStarted && totalMoved > tapSlop * 0.6f) {
                                    dragStarted = true
                                    dragging = true
                                    core.remoteButton(pcId, "left", "down", onStatus)
                                }

                                if (dist > 0.2f) {
                                    val scaledDx = delta.x * sensitivity
                                    val scaledDy = delta.y * sensitivity
                                    core.remoteMove(pcId, scaledDx, scaledDy)
                                    change.consume()
                                }
                            } else if (pressed.size >= 2) {
                                touchPoint = pressed[0].position
                                var avgDx = 0f
                                var avgDy = 0f
                                for (c in pressed) {
                                    val d = c.positionChange()
                                    avgDx += d.x
                                    avgDy += d.y
                                    c.consume()
                                }
                                avgDx /= pressed.size
                                avgDy /= pressed.size
                                totalMoved += hypot(avgDx, avgDy)

                                scrollAccumX += avgDx
                                scrollAccumY += avgDy
                                val stepPx = 18f
                                if (abs(scrollAccumX) >= stepPx || abs(scrollAccumY) >= stepPx) {
                                    val notchesX = -(scrollAccumX / 48f)
                                    val notchesY = -(scrollAccumY / 36f)
                                    scrollAccumX = 0f
                                    scrollAccumY = 0f
                                    core.remoteScroll(pcId, notchesX, notchesY, fast = true)
                                }
                            }
                        }

                        touchPoint = null
                        val elapsed = System.currentTimeMillis() - downTime
                        if (dragStarted) {
                            core.remoteButton(pcId, "left", "up", onStatus)
                            dragging = false
                            lastSingleTapTime = 0L
                        } else if (elapsed <= tapTimeoutMs && totalMoved <= tapSlop * 1.5f) {
                            if (maxPointers == 1) {
                                lastSingleTapTime = System.currentTimeMillis()
                                core.remoteButton(pcId, "left", "click", onStatus)
                            } else if (maxPointers == 2) {
                                lastSingleTapTime = 0L
                                core.remoteButton(pcId, "right", "click", onStatus)
                            }
                        } else {
                            lastSingleTapTime = 0L
                        }
                    }
                },
        ) {
            Box(Modifier.fillMaxSize()) {
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

        // Mouse buttons + Keyboard toggle row
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FilledTonalButton(
                onClick = { core.remoteButton(pcId, "left", "click", onStatus) },
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_left_click), maxLines = 1)
            }
            OutlinedButton(
                onClick = { core.remoteButton(pcId, "middle", "click", onStatus) },
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_middle_click), maxLines = 1)
            }
            FilledTonalButton(
                onClick = { core.remoteButton(pcId, "right", "click", onStatus) },
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_right_click), maxLines = 1)
            }
            Button(
                onClick = { showKeyboard = !showKeyboard },
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                modifier = Modifier.height(48.dp),
            ) {
                Text(stringResource(R.string.remote_keyboard), maxLines = 1)
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

        // Special keys & modifiers bar (always accessible in Touchpad mode)
        SpecialKeysBar(
            activeMods = activeMods,
            onToggleMod = { mod ->
                if (mod in activeMods) activeMods.remove(mod) else activeMods.add(mod)
            },
            onKey = { key, forceMods ->
                val mods = forceMods ?: activeMods.toList()
                core.remoteKey(pcId, key, mods, onStatus)
                if (forceMods == null) activeMods.clear()
            },
        )

        // Soft keyboard input row when toggled open
        if (showKeyboard) {
            OutlinedTextField(
                value = textBuffer,
                onValueChange = { next ->
                    if (next.length > textBuffer.length) {
                        val added = next.substring(textBuffer.length)
                        if (activeMods.isNotEmpty() && added.length == 1 && added[0].isLetterOrDigit()) {
                            core.remoteKey(pcId, added.lowercase(), activeMods.toList(), onStatus)
                            activeMods.clear()
                        } else {
                            core.remoteText(pcId, added, onStatus)
                        }
                    } else if (next.length < textBuffer.length) {
                        core.remoteKey(pcId, "backspace", activeMods.toList(), onStatus)
                        activeMods.clear()
                    }
                    textBuffer = " "
                },
                placeholder = { Text(stringResource(R.string.remote_type_placeholder, pcName)) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(
                    onSend = {
                        core.remoteKey(pcId, "enter", activeMods.toList(), onStatus)
                        activeMods.clear()
                    },
                ),
                modifier = Modifier
                    .fillMaxWidth()
                    .focusRequester(focusRequester),
            )
        }
    }
}

@Composable
private fun AirMouseView(
    pcId: String,
    pcName: String,
    core: Core,
    sensitivity: Float,
    utteranceJoiner: UtteranceJoiner,
    onStatus: (Core.RemoteAccess) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val sensorManager = remember(context) {
        context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    }
    val gyroSensor = remember(sensorManager) {
        sensorManager?.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    }
    val hasGyro = gyroSensor != null

    var padHeld by remember { mutableStateOf(false) }
    var aimOffset by remember { mutableStateOf(Offset.Zero) }
    val filter = remember { AirMouseFilter() }
    val currentSensitivity by rememberUpdatedState(sensitivity)

    DisposableEffect(padHeld, pcId, hasGyro) {
        if (!padHeld || gyroSensor == null || sensorManager == null) {
            filter.reset()
            aimOffset = Offset.Zero
            return@DisposableEffect onDispose {}
        }

        filter.reset()
        val listener = object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                if (!padHeld || event.sensor.type != Sensor.TYPE_GYROSCOPE || event.values.size < 3) return
                val (dx, dy) = filter.onSample(
                    wx = event.values[0],
                    wy = event.values[1],
                    wz = event.values[2],
                    timestampNs = event.timestamp,
                    sensitivity = currentSensitivity,
                )
                if (abs(dx) >= 0.05f || abs(dy) >= 0.05f) {
                    core.remoteMove(pcId, dx, dy)
                    val nextX = (aimOffset.x + dx * 0.35f).coerceIn(-120f, 120f)
                    val nextY = (aimOffset.y + dy * 0.35f).coerceIn(-120f, 120f)
                    aimOffset = Offset(nextX, nextY)
                }
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }

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
        if (!hasGyro) {
            StatusBanner(
                title = stringResource(R.string.remote_air_no_gyro_title),
                text = stringResource(R.string.remote_air_no_gyro_text),
            )
        }

        val primaryColor = MaterialTheme.colorScheme.primary
        val outlineColor = MaterialTheme.colorScheme.outlineVariant
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
                .pointerInput(pcId, hasGyro) {
                    val tapSlop = viewConfiguration.touchSlop
                    val tapTimeoutMs = 240L

                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false)
                        val downTime = System.currentTimeMillis()
                        var maxPointers = 1
                        var totalMoved = 0f

                        filter.reset()
                        aimOffset = Offset.Zero
                        padHeld = hasGyro

                        while (true) {
                            val event = awaitPointerEvent()
                            val pressed = event.changes.filter { it.pressed }
                            if (pressed.isEmpty()) break
                            if (pressed.size > maxPointers) maxPointers = pressed.size
                            for (c in pressed) {
                                val d = c.positionChange()
                                totalMoved += hypot(d.x, d.y)
                            }
                        }

                        padHeld = false
                        filter.reset()
                        aimOffset = Offset.Zero

                        val elapsed = System.currentTimeMillis() - downTime
                        if (elapsed <= tapTimeoutMs && totalMoved <= tapSlop * 1.5f) {
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

            Surface(
                shape = CircleShape,
                color = buttonBg,
                contentColor = buttonFg,
                modifier = Modifier
                    .height(40.dp)
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
                        text = stringResource(
                            if (isListening) R.string.remote_dictate_stop else R.string.remote_dictate,
                        ),
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
    onToggleMod: (String) -> Unit,
    onKey: (key: String, forceMods: List<String>?) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val mods = listOf("ctrl" to "Ctrl", "alt" to "Alt", "shift" to "Shift", "win" to "Win")
        for ((id, label) in mods) {
            FilterChip(
                selected = id in activeMods,
                onClick = { onToggleMod(id) },
                label = { Text(label) },
            )
        }

        val keys = listOf(
            "escape" to "Esc",
            "tab" to "Tab",
            "left" to "←",
            "up" to "↑",
            "down" to "↓",
            "right" to "→",
            "backspace" to "Bksp",
            "enter" to "Enter",
            "space" to "Space",
            "home" to "Home",
            "end" to "End",
            "page_up" to "PgUp",
            "page_down" to "PgDn",
            "delete" to "Del",
        )
        for ((key, label) in keys) {
            OutlinedButton(
                onClick = { onKey(key, null) },
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(label, style = MaterialTheme.typography.labelMedium)
            }
        }

        val shortcuts = listOf(
            "copy" to "Copy",
            "paste" to "Paste",
            "undo" to "Undo",
        )
        for ((shortcut, label) in shortcuts) {
            OutlinedButton(
                onClick = { onKey(shortcut, emptyList()) },
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(label, style = MaterialTheme.typography.labelMedium)
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
    var laserX by remember { mutableFloatStateOf(0.5f) }
    var laserY by remember { mutableFloatStateOf(0.5f) }

    // While held, say it again every second even if it hasn't moved: the PC
    // hides a dot that stops hearing from its phone.
    LaunchedEffect(laserHeld, pcId) {
        while (laserHeld) {
            delay(1000)
            core.remoteLaser(pcId, true, laserX, laserY)
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
                laserX = nx
                laserY = ny
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
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Huge Next Slide button (easy to hit without looking)
        Button(
            onClick = { core.remoteSlide(pcId, "next", onStatus) },
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxWidth()
                .height(116.dp),
        ) {
            Text(
                stringResource(R.string.remote_slide_next),
                style = MaterialTheme.typography.headlineMedium,
            )
        }

        // Previous Slide button
        FilledTonalButton(
            onClick = { core.remoteSlide(pcId, "previous", onStatus) },
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxWidth()
                .height(72.dp),
        ) {
            Text(
                stringResource(R.string.remote_slide_prev),
                style = MaterialTheme.typography.titleLarge,
            )
        }

        // Start / End / Black screen row
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "start", onStatus) },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_slide_start), maxLines = 1)
            }
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "stop", onStatus) },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_slide_stop), maxLines = 1)
            }
            OutlinedButton(
                onClick = { core.remoteSlide(pcId, "black", onStatus) },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
                modifier = Modifier.weight(1f).height(48.dp),
            ) {
                Text(stringResource(R.string.remote_slide_black), maxLines = 1)
            }
        }

        // Hold for laser pointer pad (supports both gyroscope tilt and finger drag)
        val primaryColor = MaterialTheme.colorScheme.primary
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
                .pointerInput(pcId) {
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val w = size.width.coerceAtLeast(1).toFloat()
                        val h = size.height.coerceAtLeast(1).toFloat()
                        laserX = (down.position.x / w).coerceIn(0f, 1f)
                        laserY = (down.position.y / h).coerceIn(0f, 1f)
                        laserHeld = true
                        core.remoteLaser(pcId, true, laserX, laserY)

                        while (true) {
                            val event = awaitPointerEvent()
                            val active = event.changes.firstOrNull { it.pressed } ?: break
                            val d = active.positionChange()
                            if (hypot(d.x, d.y) > 0.5f) {
                                laserX = (active.position.x / w).coerceIn(0f, 1f)
                                laserY = (active.position.y / h).coerceIn(0f, 1f)
                                core.remoteLaser(pcId, true, laserX, laserY)
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
                        val pt = Offset(laserX * size.width, laserY * size.height)
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
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}
