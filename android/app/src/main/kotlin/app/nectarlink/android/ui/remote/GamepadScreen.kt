// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.app.Activity
import android.content.pm.ActivityInfo
import android.view.HapticFeedbackConstants
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import kotlinx.coroutines.delay
import kotlin.math.min

/** How often the controller is sent again while nothing changes, so the PC knows it's still held. */
private const val PAD_REPEAT_MS = 100L

/** A second tap on a stick within this long clicks it (L3/R3) while the thumb stays down. */
private const val STICK_CLICK_MS = 300L

/** Where each control sits for a screen of one size, in pixels. */
private class PadLayout(size: IntSize, density: Float) {
    val w = size.width.toFloat()
    val h = size.height.toFloat()
    private val unit = min(h, w * 0.5f)
    val stickRadius = unit * 0.15f
    val leftStick = Offset(w * 0.15f, h * 0.55f)
    val rightStick = Offset(w * 0.66f, h * 0.78f)
    val dpad = Offset(w * 0.34f, h * 0.78f)
    val dpadRadius = unit * 0.15f
    val face = Offset(w * 0.85f, h * 0.55f)
    val faceSpread = unit * 0.125f
    val faceRadius = unit * 0.07f
    private val margin = 16f * density
    private val shoulderW = w * 0.14f
    private val shoulderH = unit * 0.13f
    private val top = margin
    val lt = Rect(margin, top, margin + shoulderW, top + shoulderH)
    val lb = Rect(lt.right + margin * 0.5f, top, lt.right + margin * 0.5f + shoulderW, top + shoulderH)
    val rt = Rect(w - margin - shoulderW, top, w - margin, top + shoulderH)
    val rb = Rect(rt.left - margin * 0.5f - shoulderW, top, rt.left - margin * 0.5f, top + shoulderH)
    private val small = unit * 0.06f
    val back = Offset(w * 0.40f, h * 0.36f)
    val guide = Offset(w * 0.50f, h * 0.36f)
    val start = Offset(w * 0.60f, h * 0.36f)
    val smallRadius = small

    /** Face buttons, in the Xbox places: Y up, A down, X left, B right. */
    val faces = listOf(
        PadSnapshot.Y to Offset(face.x, face.y - faceSpread),
        PadSnapshot.A to Offset(face.x, face.y + faceSpread),
        PadSnapshot.X to Offset(face.x - faceSpread, face.y),
        PadSnapshot.B to Offset(face.x + faceSpread, face.y),
    )

    fun inFaces(p: Offset) = (p - face).getDistance() <= faceSpread + faceRadius * 1.6f
    fun nearestFace(p: Offset) = faces.minBy { (it.second - p).getDistance() }.first
    fun inDpad(p: Offset) = (p - dpad).getDistance() <= dpadRadius * 1.5f

    /** A plain button under `p` (shoulders, triggers, Back, Home, Start), or null. */
    fun buttonAt(p: Offset): PadButton? = when {
        lt.inflate(margin * 0.25f).contains(p) -> PadButton.LeftTrigger
        rt.inflate(margin * 0.25f).contains(p) -> PadButton.RightTrigger
        lb.inflate(margin * 0.25f).contains(p) -> PadButton.Bit(PadSnapshot.LB)
        rb.inflate(margin * 0.25f).contains(p) -> PadButton.Bit(PadSnapshot.RB)
        (p - back).getDistance() <= smallRadius * 1.5f -> PadButton.Bit(PadSnapshot.BACK)
        (p - guide).getDistance() <= smallRadius * 1.5f -> PadButton.Bit(PadSnapshot.GUIDE)
        (p - start).getDistance() <= smallRadius * 1.5f -> PadButton.Bit(PadSnapshot.START)
        else -> null
    }
}

private sealed interface PadButton {
    data class Bit(val bit: Int) : PadButton
    data object LeftTrigger : PadButton
    data object RightTrigger : PadButton
}

/** What one finger is doing. */
private sealed interface Touch {
    data class Button(val button: PadButton) : Touch
    data class Face(var bit: Int) : Touch
    data class Dpad(var bits: Int) : Touch
    class Stick(val left: Boolean, val origin: Offset, val clicked: Boolean) : Touch {
        var at: Offset = origin
    }
}

/**
 * The phone as a game controller for [device] (#442): landscape, full screen, with two
 * sticks, a d-pad, A B X Y, bumpers, triggers, Back, Home and Start. The PC sees an Xbox
 * controller. Each finger works on its own, so thumbs on a stick and a button at once work.
 */
@Composable
internal fun GamepadScreen(device: Device, core: Core, onClose: () -> Unit) {
    val context = LocalContext.current
    val view = LocalView.current
    val density = LocalDensity.current.density
    BackHandler(onBack = onClose)

    // Landscape and the whole screen while it's open.
    DisposableEffect(view) {
        val activity = context as? Activity
        val previous = activity?.requestedOrientation
        activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        val bars = activity?.window?.let { WindowCompat.getInsetsController(it, view) }
        bars?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose {
            bars?.show(WindowInsetsCompat.Type.systemBars())
            if (previous != null) activity.requestedOrientation = previous
        }
    }

    var pad by remember { mutableStateOf(PadSnapshot()) }
    var access by remember(device.id) { mutableStateOf<Core.PadAccess?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    val latestPad by rememberUpdatedState(pad)

    // Plug the controller in, and keep asking while the PC hasn't allowed it yet.
    LaunchedEffect(device.id, device.online, attempt) {
        if (!device.online) {
            access = Core.PadAccess.Offline
            return@LaunchedEffect
        }
        access = null
        var result = core.remotePadPlugIn(device.id, latestPad)
        while (result == Core.PadAccess.Denied && device.online) {
            access = result
            delay(1_500)
            result = core.remotePadPlugIn(device.id, latestPad)
        }
        access = result
    }
    // Only while the controller is on screen: after Home or with the screen off, the
    // PC's controller is unplugged and nothing more is sent; coming back plugs it in again.
    val lifecycleState by androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
        .currentStateFlow.collectAsState()
    val onScreen = lifecycleState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)
    val allowed = access == Core.PadAccess.Allowed && onScreen
    val sending by rememberUpdatedState(allowed)
    LaunchedEffect(device.id, onScreen) {
        if (!onScreen) {
            pad = PadSnapshot()
            core.remotePadOff(device.id)
        }
    }
    // Each change goes from the touch handler as it happens (below). This repeats the
    // newest every 100 ms, so a lost one is made up for and the PC lets go if the phone
    // goes quiet.
    LaunchedEffect(device.id, allowed) {
        if (!allowed) return@LaunchedEffect
        while (true) {
            core.remotePad(device.id, latestPad, coalesce = true)
            delay(PAD_REPEAT_MS)
        }
    }
    DisposableEffect(device.id) {
        onDispose { core.remotePadOff(device.id) }
    }

    var size by remember { mutableStateOf(IntSize.Zero) }
    val layout = remember(size, density) { PadLayout(size, density) }
    val touches = remember { HashMap<PointerId, Touch>() }
    // Bumped when a finger lands or lifts, so a stick's ring follows the thumb at once.
    var touchCount by remember { mutableIntStateOf(0) }
    // Per stick, when and where its last finger lifted (for a click by tapping twice).
    val lastStickUp = remember { HashMap<Boolean, Pair<Long, Offset>>() }

    fun compute(): PadSnapshot {
        var buttons = 0
        var lt = 0
        var rt = 0
        var lx = 0
        var ly = 0
        var rx = 0
        var ry = 0
        for (t in touches.values) {
            when (t) {
                is Touch.Button -> when (val b = t.button) {
                    is PadButton.Bit -> buttons = buttons or b.bit
                    PadButton.LeftTrigger -> lt = 255
                    PadButton.RightTrigger -> rt = 255
                }
                is Touch.Face -> buttons = buttons or t.bit
                is Touch.Dpad -> buttons = buttons or t.bits
                is Touch.Stick -> {
                    val (x, y) = GamepadMath.stick(t.at.x - t.origin.x, t.at.y - t.origin.y, layout.stickRadius)
                    if (t.left) {
                        lx = x
                        ly = y
                        if (t.clicked) buttons = buttons or PadSnapshot.LTHUMB
                    } else {
                        rx = x
                        ry = y
                        if (t.clicked) buttons = buttons or PadSnapshot.RTHUMB
                    }
                }
            }
        }
        return PadSnapshot(buttons, lt, rt, lx, ly, rx, ry)
    }

    fun press(at: Offset, now: Long): Touch {
        layout.buttonAt(at)?.let { return Touch.Button(it) }
        if (layout.inDpad(at)) {
            return Touch.Dpad(GamepadMath.dpad(at.x - layout.dpad.x, at.y - layout.dpad.y, layout.dpadRadius * 0.25f))
        }
        if (layout.inFaces(at)) return Touch.Face(layout.nearestFace(at))
        // Anywhere else on a side moves that side's stick, from where the thumb lands.
        val left = at.x < layout.w / 2f
        val clicked = lastStickUp[left]?.let { (time, where) ->
            now - time <= STICK_CLICK_MS && (where - at).getDistance() <= layout.stickRadius
        } == true
        return Touch.Stick(left, at, clicked)
    }

    val padDesc = stringResource(R.string.gamepad_title, device.name)
    val colors = MaterialTheme.colorScheme
    val textMeasurer = rememberTextMeasurer()
    Box(Modifier.fillMaxSize()) {
        Surface(color = colors.surfaceContainerLowest, modifier = Modifier.fillMaxSize()) {}
        Canvas(
            Modifier
                .fillMaxSize()
                .onSizeChanged { size = it }
                .semantics { contentDescription = padDesc }
                .pointerInput(layout) {
                    // A new layout (the screen turned): fingers from before don't hold anything.
                    touches.clear()
                    pad = PadSnapshot()
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent()
                            val before = pad
                            for (change in event.changes) {
                                val id = change.id
                                when {
                                    change.pressed && !change.previousPressed ->
                                        touches[id] = press(change.position, change.uptimeMillis)
                                    change.pressed -> when (val t = touches[id]) {
                                        is Touch.Stick -> t.at = change.position
                                        // Slide from one face button to the next.
                                        is Touch.Face -> t.bit =
                                            if (layout.inFaces(change.position)) layout.nearestFace(change.position) else 0
                                        is Touch.Dpad -> t.bits = if (layout.inDpad(change.position)) {
                                            GamepadMath.dpad(
                                                change.position.x - layout.dpad.x,
                                                change.position.y - layout.dpad.y,
                                                layout.dpadRadius * 0.25f,
                                            )
                                        } else {
                                            0
                                        }
                                        else -> {}
                                    }
                                    else -> {
                                        val t = touches.remove(id)
                                        if (t is Touch.Stick) lastStickUp[t.left] = change.uptimeMillis to change.position
                                    }
                                }
                                change.consume()
                            }
                            if (touches.size != touchCount) touchCount = touches.size
                            val after = compute()
                            if (after != before) {
                                // A click under the thumb for each newly pressed button.
                                if (after.buttons and before.buttons.inv() != 0 ||
                                    (after.lt > 0 && before.lt == 0) || (after.rt > 0 && before.rt == 0)
                                ) {
                                    view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                                }
                                pad = after
                                if (sending) {
                                    val buttonsChanged = after.buttons != before.buttons ||
                                        after.lt != before.lt || after.rt != before.rt
                                    core.remotePad(device.id, after, coalesce = !buttonsChanged)
                                }
                            }
                        }
                    }
                },
        ) {
            if (size == IntSize.Zero) return@Canvas
            // Read so a thumb landing redraws (where a stick's ring sits).
            @Suppress("UNUSED_VARIABLE") val count = touchCount
            drawPad(layout, pad, touches.values.filterIsInstance<Touch.Stick>(), colors, textMeasurer)
        }

        FilledTonalIconButton(
            onClick = onClose,
            modifier = Modifier
                .align(Alignment.TopCenter)
                .padding(top = 12.dp)
                .size(40.dp),
        ) {
            Icon(painterResource(R.drawable.ic_close), contentDescription = stringResource(R.string.gamepad_close))
        }

        val status = access
        if (status != Core.PadAccess.Allowed) {
            PadStatus(device, status, onRetry = { attempt++ }, onClose = onClose)
        }
    }
}

@Composable
private fun PadStatus(device: Device, status: Core.PadAccess?, onRetry: () -> Unit, onClose: () -> Unit) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Surface(
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.widthIn(max = 460.dp),
        ) {
            Column(
                Modifier.padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                val (title, text) = when (status) {
                    null -> stringResource(R.string.gamepad_connecting, device.name) to null
                    Core.PadAccess.Denied ->
                        stringResource(R.string.remote_denied_title, device.name) to
                            stringResource(R.string.remote_denied_text, device.name)
                    Core.PadAccess.DriverMissing ->
                        stringResource(R.string.gamepad_driver_title, device.name) to
                            stringResource(R.string.gamepad_driver_text, device.name)
                    Core.PadAccess.Failed -> stringResource(R.string.gamepad_failed, device.name) to null
                    else -> stringResource(R.string.gamepad_offline, device.name) to null
                }
                Text(title, style = MaterialTheme.typography.titleMedium)
                if (text != null) {
                    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (status != null && status != Core.PadAccess.Denied) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(onClick = onClose) { Text(stringResource(R.string.gamepad_close)) }
                        Button(onClick = onRetry) { Text(stringResource(R.string.gamepad_retry)) }
                    }
                }
            }
        }
    }
}

private fun DrawScope.drawPad(
    layout: PadLayout,
    pad: PadSnapshot,
    sticks: List<Touch.Stick>,
    colors: androidx.compose.material3.ColorScheme,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
) {
    val idle = colors.surfaceContainerHigh
    val down = colors.primaryContainer
    val line = colors.outlineVariant
    val label = colors.onSurface
    val labelDown = colors.onPrimaryContainer
    val stroke = Stroke(width = 1.dp.toPx())

    fun text(s: String, center: Offset, pressed: Boolean, sizeSp: Float = 16f) {
        val measured = textMeasurer.measure(
            s,
            TextStyle(
                color = if (pressed) labelDown else label,
                fontSize = sizeSp.sp,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
            ),
        )
        drawText(measured, topLeft = center - Offset(measured.size.width / 2f, measured.size.height / 2f))
    }

    fun shoulder(rect: Rect, name: String, pressed: Boolean) {
        val r = CornerRadius(rect.height / 2f)
        drawRoundRect(if (pressed) down else idle, rect.topLeft, rect.size, r)
        drawRoundRect(line, rect.topLeft, rect.size, r, style = stroke)
        text(name, rect.center, pressed, 14f)
    }

    fun round(center: Offset, radius: Float, pressed: Boolean, color: Color = idle) {
        drawCircle(if (pressed) down else color, radius, center)
        drawCircle(line, radius, center, style = stroke)
    }

    shoulder(layout.lt, "LT", pad.lt > 0)
    shoulder(layout.lb, "LB", pad.buttons and PadSnapshot.LB != 0)
    shoulder(layout.rb, "RB", pad.buttons and PadSnapshot.RB != 0)
    shoulder(layout.rt, "RT", pad.rt > 0)

    // Back (two windows), Home, Start (three lines).
    val s = layout.smallRadius
    round(layout.back, s, pad.buttons and PadSnapshot.BACK != 0)
    val backInk = if (pad.buttons and PadSnapshot.BACK != 0) labelDown else label
    drawRoundRect(backInk, layout.back + Offset(-s * 0.45f, -s * 0.35f), Size(s * 0.5f, s * 0.45f), CornerRadius(2f), style = stroke)
    drawRoundRect(backInk, layout.back + Offset(-s * 0.15f, -s * 0.1f), Size(s * 0.5f, s * 0.45f), CornerRadius(2f), style = stroke)
    round(layout.guide, s * 1.15f, pad.buttons and PadSnapshot.GUIDE != 0)
    drawCircle(
        if (pad.buttons and PadSnapshot.GUIDE != 0) labelDown else colors.primary,
        s * 0.35f,
        layout.guide,
    )
    round(layout.start, s, pad.buttons and PadSnapshot.START != 0)
    val startInk = if (pad.buttons and PadSnapshot.START != 0) labelDown else label
    for (i in -1..1) {
        val y = layout.start.y + i * s * 0.3f
        drawLine(startInk, Offset(layout.start.x - s * 0.4f, y), Offset(layout.start.x + s * 0.4f, y), 1.5.dp.toPx())
    }

    // D-pad: a cross whose arms light up.
    val d = layout.dpad
    val arm = layout.dpadRadius
    val thick = arm * 0.62f
    val dpadArms = listOf(
        PadSnapshot.UP to Rect(d.x - thick / 2, d.y - arm, d.x + thick / 2, d.y - thick / 2),
        PadSnapshot.DOWN to Rect(d.x - thick / 2, d.y + thick / 2, d.x + thick / 2, d.y + arm),
        PadSnapshot.LEFT to Rect(d.x - arm, d.y - thick / 2, d.x - thick / 2, d.y + thick / 2),
        PadSnapshot.RIGHT to Rect(d.x + thick / 2, d.y - thick / 2, d.x + arm, d.y + thick / 2),
    )
    drawRect(idle, Offset(d.x - thick / 2, d.y - thick / 2), Size(thick, thick))
    for ((bit, rect) in dpadArms) {
        val pressed = pad.buttons and bit != 0
        drawRoundRect(if (pressed) down else idle, rect.topLeft, rect.size, CornerRadius(thick * 0.2f))
    }
    drawRoundRect(line, Offset(d.x - thick / 2, d.y - arm), Size(thick, arm * 2), CornerRadius(thick * 0.2f), style = stroke)
    drawRoundRect(line, Offset(d.x - arm, d.y - thick / 2), Size(arm * 2, thick), CornerRadius(thick * 0.2f), style = stroke)

    // A B X Y.
    val names = mapOf(PadSnapshot.A to "A", PadSnapshot.B to "B", PadSnapshot.X to "X", PadSnapshot.Y to "Y")
    for ((bit, center) in layout.faces) {
        val pressed = pad.buttons and bit != 0
        round(center, layout.faceRadius, pressed)
        text(names.getValue(bit), center, pressed, 18f)
    }

    // Sticks: a ring where they rest, or under the thumb that holds them, and the knob.
    fun stick(left: Boolean, home: Offset) {
        val held = sticks.firstOrNull { it.left == left }
        val base = held?.origin ?: home
        val r = layout.stickRadius
        drawCircle(idle, r, base)
        drawCircle(line, r, base, style = stroke)
        val x = if (left) pad.lx else pad.rx
        val y = if (left) pad.ly else pad.ry
        val clickBit = if (left) PadSnapshot.LTHUMB else PadSnapshot.RTHUMB
        val knob = base + Offset(x / 32767f * r * 0.6f, -y / 32767f * r * 0.6f)
        round(knob, r * 0.45f, held != null || pad.buttons and clickBit != 0, colors.surfaceContainerHighest)
    }
    stick(true, layout.leftStick)
    stick(false, layout.rightStick)
}
