// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.os.SystemClock
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputScope
import app.nectarlink.android.core.Core
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.pow
import kotlin.math.sign

data class TouchpadGestureOptions(
    val pointerSpeed: Float = TouchpadBallistics.DEFAULT_POINTER_SPEED,
    val acceleration: Float = TouchpadBallistics.DEFAULT_ACCELERATION,
    val scrollSpeed: Float = TouchpadBallistics.DEFAULT_SCROLL_SPEED,
    val tapToClick: Boolean = true,
    val threeFingerMiddleClick: Boolean = false,
    val naturalScroll: Boolean = true,
    val momentum: Boolean = true,
    /** Set false when a mode supplies its own one-finger aiming. Multi-finger gestures still work. */
    val relativePointer: Boolean = true,
)

data class TouchpadContact(val id: Long, val xDp: Float, val yDp: Float)

enum class TouchpadHaptic { Soft, Tick, Click, Heavy, Double }

enum class TouchpadAction {
    SwitchStart, SwitchNext, SwitchPrevious, SwitchEnd, SwitchCancel,
    WorkspaceNext, WorkspacePrevious, Overview, OverviewClose, ShowDesktop, Search, Notifications,
}

enum class TouchpadButton { Left, Right, Middle }
enum class TouchpadButtonState { Down, Up, Click }

sealed interface TouchpadSingleFinger {
    data class Start(val contact: TouchpadContact) : TouchpadSingleFinger
    data class Move(val contact: TouchpadContact, val dxDp: Float, val dyDp: Float) : TouchpadSingleFinger
    data object End : TouchpadSingleFinger
    data object Cancel : TouchpadSingleFinger
}

sealed interface TouchpadGestureEvent {
    data class Move(val dx: Float, val dy: Float) : TouchpadGestureEvent
    data class Button(val button: TouchpadButton, val state: TouchpadButtonState) : TouchpadGestureEvent
    data class Scroll(val dxNotches: Float, val dyNotches: Float) : TouchpadGestureEvent
    data class Pinch(val active: Boolean) : TouchpadGestureEvent
    data class Zoom(val dyNotches: Float) : TouchpadGestureEvent
    data class Action(val action: TouchpadAction) : TouchpadGestureEvent
    data class Haptic(val haptic: TouchpadHaptic) : TouchpadGestureEvent
    data class SingleFinger(val event: TouchpadSingleFinger) : TouchpadGestureEvent
    data object LeftTap : TouchpadGestureEvent
    data object Stolen : TouchpadGestureEvent
}

/** Feed snapshots of all pressed contacts in dp and use one monotonic clock for frames and timers. */
class TouchpadGestureEngine(
    private val options: TouchpadGestureOptions = TouchpadGestureOptions(),
    private val emit: (TouchpadGestureEvent) -> Unit,
) {
    private val ballistics = TouchpadBallistics(options.pointerSpeed, options.acceleration)
    private var gesture: Gesture? = null
    private var drag: Drag? = null
    private var graceDeadline: Long? = null
    private var lastTap: Tap? = null
    private var shown: Shown? = null
    private var momentum: Momentum? = null
    private var timeMs = 0L

    /** Wake [advanceTo] at this time even if no pointer events arrive. */
    val nextDeadlineMs: Long?
        get() = gesture?.holdDeadline ?: graceDeadline ?: momentum?.let { it.lastMs + FRAME_MS }

    fun advanceTo(timestampMs: Long) {
        timeMs = timestampMs.coerceAtLeast(timeMs)
        gesture?.let { current ->
            if (current.holdDeadline?.let { timeMs >= it } == true) {
                current.holdDeadline = null
                if (current.travel < HOLD_SLOP_DP) {
                    current.holdArmed = true
                    haptic(TouchpadHaptic.Tick)
                }
            }
        }
        if (graceDeadline?.let { timeMs >= it } == true) endDrag()
        momentum?.let { glide ->
            if (timeMs - glide.lastMs >= FRAME_MS) {
                val dt = (timeMs - glide.lastMs).coerceAtMost(40L)
                glide.lastMs = timeMs
                val decay = 0.9965f.pow(dt.toFloat())
                glide.vx *= decay
                glide.vy *= decay
                scroll(glide.vx * dt, glide.vy * dt)
                if (hypot(glide.vx, glide.vy) < 0.03f) momentum = null
            }
        }
    }

    fun onPointers(timestampMs: Long, contacts: List<TouchpadContact>) {
        advanceTo(timestampMs)
        val current = gesture
        if (current == null) {
            if (contacts.isNotEmpty()) begin(contacts)
            return
        }
        if (contacts.isEmpty()) {
            finish(current)
            return
        }

        val previousCount = current.contacts.size
        val joined = contacts.any { it.id !in current.contacts }
        if (joined || contacts.size != previousCount) {
            val pressedIds = contacts.mapTo(HashSet()) { it.id }
            current.contacts.keys.retainAll(pressedIds)
            for (contact in contacts) {
                val tracked = current.contacts.getOrPut(contact.id) { TrackedContact(contact) }
                tracked.update(contact)
                current.travel = maxOf(current.travel, tracked.travel)
            }
            if (joined) join(current)
            current.rebase()
            current.lastMoveMs = timeMs
            return
        }

        var moved = false
        for (contact in contacts) {
            val tracked = current.contacts.getValue(contact.id)
            moved = moved || tracked.point != contact
            tracked.update(contact)
            current.travel = maxOf(current.travel, tracked.travel)
        }
        if (!moved) return

        val (cx, cy, spread) = current.geometry()
        val dx = cx - current.cx
        val dy = cy - current.cy
        val ds = spread - current.spread
        current.cx = cx
        current.cy = cy
        current.spread = spread
        if (current.kind == Kind.One) {
            moveOne(current, contacts.single(), dx, dy)
        } else if (contacts.size >= 2) {
            moveMulti(current, dx, dy, ds)
        }
        current.lastMoveMs = timeMs
    }

    /** Drops pending timers and releases any pressed button, pinch, or switcher. */
    fun cancel(stolenBySystem: Boolean = false) {
        val current = gesture
        gesture = null
        momentum = null
        lastTap = null
        if (current != null) {
            if (current.kind == Kind.One) single(TouchpadSingleFinger.Cancel)
            closeClassification(current, cancelled = true)
        }
        endDrag()
        if (stolenBySystem && current != null && current.maxFingers >= 3) {
            emit(TouchpadGestureEvent.Stolen)
        }
    }

    private fun begin(contacts: List<TouchpadContact>) {
        momentum = null
        val repositioning = drag == Drag.Hold && graceDeadline != null
        graceDeadline = null
        val current = Gesture(contacts, timeMs)
        gesture = current
        val tap = lastTap
        lastTap = null
        if (current.kind != Kind.One) return

        val p = contacts.single()
        single(TouchpadSingleFinger.Start(p))
        ballistics.reset()
        if (!options.relativePointer || repositioning) return
        if (options.tapToClick && tap != null && timeMs - tap.timeMs < DOUBLE_TAP_MS &&
            hypot(p.xDp - tap.point.xDp, p.yDp - tap.point.yDp) < DOUBLE_TAP_DISTANCE_DP
        ) {
            startDrag(Drag.Tap)
        } else {
            current.holdDeadline = timeMs + HOLD_MS
        }
    }

    private fun join(current: Gesture) {
        val count = current.contacts.size
        current.maxFingers = maxOf(current.maxFingers, count)
        current.lastJoinMs = timeMs
        if (current.kind == Kind.One && count > 1) {
            single(TouchpadSingleFinger.Cancel)
            current.kind = Kind.Multi
        } else if (current.kind != Kind.One && current.kind != Kind.Multi && !current.fired &&
            (count > current.classifiedCount || timeMs - current.classifiedMs < JOIN_MS)
        ) {
            closeClassification(current, cancelled = true)
            current.kind = Kind.Multi
            current.scrollSamples.clear()
        }
        if (current.kind != Kind.One) {
            if (drag == Drag.Tap) endDrag()
            current.holdDeadline = null
            current.holdArmed = false
        }
    }

    private fun moveOne(current: Gesture, point: TouchpadContact, dx: Float, dy: Float) {
        if (current.travel > HOLD_SLOP_DP) current.holdDeadline = null
        if (current.holdArmed && drag == null) {
            current.holdArmed = false
            startDrag(Drag.Hold)
        }
        if (drag == Drag.Tap && !current.dragMoved && current.travel > HOLD_SLOP_DP) {
            current.dragMoved = true
            haptic(TouchpadHaptic.Tick)
        }
        single(TouchpadSingleFinger.Move(point, dx, dy))
        if (options.relativePointer && (dx != 0f || dy != 0f)) {
            val gain = ballistics.gainDp(hypot(dx, dy), timeMs - current.lastMoveMs)
            emit(TouchpadGestureEvent.Move(dx * gain, dy * gain))
        }
    }

    private fun moveMulti(current: Gesture, dx: Float, dy: Float, ds: Float) {
        when (current.kind) {
            Kind.Multi -> classify(current)
            Kind.Scroll -> {
                if (current.axis == null) {
                    current.absX += abs(dx)
                    current.absY += abs(dy)
                    if (current.absX + current.absY > MULTI_START_DP) {
                        current.axis = when {
                            current.absY > current.absX * AXIS_RATIO -> Axis.Vertical
                            current.absX > current.absY * AXIS_RATIO -> Axis.Horizontal
                            else -> Axis.Both
                        }
                    }
                }
                val sx = if (current.axis == Axis.Vertical) 0f else dx
                val sy = if (current.axis == Axis.Horizontal) 0f else dy
                scroll(sx, sy)
                current.scrollSamples.addLast(ScrollSample(timeMs, sx, sy))
                if (current.scrollSamples.size > 20) current.scrollSamples.removeFirst()
            }
            Kind.Pinch -> {
                if (ds == 0f) return
                emit(TouchpadGestureEvent.Zoom(TouchpadBallistics.pinchNotches(ds)))
                current.stepTravel += ds
                while (abs(current.stepTravel) >= PINCH_TICK_DP) {
                    current.stepTravel -= sign(current.stepTravel) * PINCH_TICK_DP
                    haptic(TouchpadHaptic.Soft)
                }
            }
            Kind.Swipe -> if (current.switching) stepSwitcher(current, dx)
            Kind.One -> Unit
        }
    }

    private fun classify(current: Gesture) {
        if (timeMs - current.lastJoinMs < SETTLE_MS) return
        val count = current.contacts.size
        val mx = current.cx - current.startX
        val my = current.cy - current.startY
        val centroidTravel = hypot(mx, my)
        val spreadTravel = abs(current.spread - current.startSpread)
        when {
            count == 2 && maxOf(centroidTravel, spreadTravel) > MULTI_START_DP -> {
                current.kind = if (spreadTravel > 1.1f * centroidTravel) Kind.Pinch else Kind.Scroll
                current.axis = null
                current.absX = 0f
                current.absY = 0f
                current.stepTravel = 0f
                if (current.kind == Kind.Pinch) {
                    emit(TouchpadGestureEvent.Pinch(true))
                    haptic(TouchpadHaptic.Tick)
                }
            }
            count >= 3 && centroidTravel > SWIPE_START_DP -> {
                current.kind = Kind.Swipe
                startSwipe(current, count, mx, my)
            }
            else -> return
        }
        current.classifiedCount = count
        current.classifiedMs = timeMs
    }

    private fun startSwipe(current: Gesture, count: Int, dx: Float, dy: Float) {
        val horizontal = abs(dx) > abs(dy)
        if (horizontal && count == 3) {
            current.switching = true
            current.stepTravel = dx - sign(dx) * SWIPE_START_DP
            action(TouchpadAction.SwitchStart)
            switchStep(dx)
            haptic(TouchpadHaptic.Click)
            shown = null
            return
        }

        val next = when {
            horizontal -> {
                shown = null
                if (dx < 0f) TouchpadAction.WorkspaceNext else TouchpadAction.WorkspacePrevious
            }
            dy < 0f && shown == Shown.Desktop -> {
                shown = null
                TouchpadAction.ShowDesktop
            }
            dy < 0f -> {
                shown = Shown.Overview
                TouchpadAction.Overview
            }
            shown == Shown.Overview -> {
                shown = null
                TouchpadAction.OverviewClose
            }
            else -> {
                shown = Shown.Desktop
                TouchpadAction.ShowDesktop
            }
        }
        action(next)
        haptic(TouchpadHaptic.Heavy)
        current.fired = true
    }

    private fun stepSwitcher(current: Gesture, dx: Float) {
        current.stepTravel += dx
        while (abs(current.stepTravel) > SWITCH_STEP_DP) {
            switchStep(current.stepTravel)
            current.stepTravel -= sign(current.stepTravel) * SWITCH_STEP_DP
            haptic(TouchpadHaptic.Tick)
        }
    }

    private fun finish(current: Gesture) {
        gesture = null
        val multi = current.maxFingers > 1
        val tap = if (multi) {
            current.kind == Kind.Multi && timeMs - current.lastJoinMs < MULTI_TAP_MS &&
                current.travel < MULTI_TAP_DISTANCE_DP
        } else {
            timeMs - current.startedMs < TAP_MS && current.travel < TAP_DISTANCE_DP
        }
        if (tap) shown = null
        closeClassification(current, cancelled = false)

        when {
            current.kind == Kind.One -> {
                single(TouchpadSingleFinger.End)
                when (drag) {
                    Drag.Tap -> {
                        endDrag()
                        if (tap && !current.dragMoved) {
                            haptic(TouchpadHaptic.Click)
                            emit(TouchpadGestureEvent.LeftTap)
                        }
                    }
                    Drag.Hold -> graceDeadline = timeMs + REPOSITION_MS
                    null -> if (tap && options.tapToClick) {
                        click(TouchpadButton.Left)
                        emit(TouchpadGestureEvent.LeftTap)
                        lastTap = Tap(timeMs, current.contacts.values.single().point)
                    }
                }
            }
            drag != null -> endDrag()
            tap -> when (current.maxFingers) {
                2 -> click(TouchpadButton.Right)
                3 -> if (options.threeFingerMiddleClick) click(TouchpadButton.Middle) else tapAction(TouchpadAction.Search)
                else -> tapAction(TouchpadAction.Notifications)
            }
            current.kind == Kind.Scroll -> startMomentum(current.scrollSamples)
        }
    }

    private fun closeClassification(current: Gesture, cancelled: Boolean) {
        if (current.kind == Kind.Pinch) emit(TouchpadGestureEvent.Pinch(false))
        if (current.switching) {
            action(if (cancelled) TouchpadAction.SwitchCancel else TouchpadAction.SwitchEnd)
            current.switching = false
        }
    }

    private fun startMomentum(samples: ArrayDeque<ScrollSample>) {
        if (!options.momentum) return
        val recent = samples.filter { timeMs - it.timeMs < 90L }
        if (recent.size < 2) return
        val dt = (recent.last().timeMs - recent.first().timeMs).coerceAtLeast(1L)
        var dx = 0f
        var dy = 0f
        for (i in 1 until recent.size) {
            dx += recent[i].dx
            dy += recent[i].dy
        }
        val vx = dx / dt
        val vy = dy / dt
        if (hypot(vx, vy) >= 0.35f) momentum = Momentum(timeMs, vx, vy)
    }

    private fun scroll(dxDp: Float, dyDp: Float) {
        if (dxDp == 0f && dyDp == 0f) return
        val (dx, dy) = TouchpadBallistics.scrollNotches(dxDp, dyDp, options.scrollSpeed, options.naturalScroll)
        emit(TouchpadGestureEvent.Scroll(dx, dy))
    }

    private fun startDrag(kind: Drag) {
        drag = kind
        emit(TouchpadGestureEvent.Button(TouchpadButton.Left, TouchpadButtonState.Down))
        if (kind == Drag.Hold) haptic(TouchpadHaptic.Heavy)
    }

    private fun endDrag() {
        graceDeadline = null
        if (drag == null) return
        drag = null
        emit(TouchpadGestureEvent.Button(TouchpadButton.Left, TouchpadButtonState.Up))
    }

    private fun click(button: TouchpadButton) {
        emit(TouchpadGestureEvent.Button(button, TouchpadButtonState.Click))
        haptic(if (button == TouchpadButton.Left) TouchpadHaptic.Click else TouchpadHaptic.Double)
    }

    private fun switchStep(dx: Float) = action(if (dx > 0f) TouchpadAction.SwitchNext else TouchpadAction.SwitchPrevious)

    private fun tapAction(action: TouchpadAction) {
        action(action)
        haptic(TouchpadHaptic.Click)
    }

    private fun action(action: TouchpadAction) = emit(TouchpadGestureEvent.Action(action))
    private fun haptic(haptic: TouchpadHaptic) = emit(TouchpadGestureEvent.Haptic(haptic))
    private fun single(event: TouchpadSingleFinger) = emit(TouchpadGestureEvent.SingleFinger(event))

    private enum class Kind { One, Multi, Scroll, Pinch, Swipe }
    private enum class Drag { Tap, Hold }
    private enum class Axis { Horizontal, Vertical, Both }
    private enum class Shown { Overview, Desktop }
    private data class Tap(val timeMs: Long, val point: TouchpadContact)
    private data class ScrollSample(val timeMs: Long, val dx: Float, val dy: Float)
    private data class Momentum(var lastMs: Long, var vx: Float, var vy: Float)
    private data class Geometry(val x: Float, val y: Float, val spread: Float)

    private class TrackedContact(private val start: TouchpadContact) {
        var point = start
            private set
        var travel = 0f
            private set

        fun update(contact: TouchpadContact) {
            point = contact
            travel = maxOf(travel, hypot(contact.xDp - start.xDp, contact.yDp - start.yDp))
        }
    }

    private class Gesture(pressed: List<TouchpadContact>, val startedMs: Long) {
        val contacts = pressed.associateTo(LinkedHashMap()) { it.id to TrackedContact(it) }
        var maxFingers = contacts.size
        var kind = if (maxFingers == 1) Kind.One else Kind.Multi
        var lastJoinMs = startedMs
        var lastMoveMs = startedMs
        var travel = 0f
        var cx = 0f
        var cy = 0f
        var spread = 0f
        var startX = 0f
        var startY = 0f
        var startSpread = 0f
        var holdDeadline: Long? = null
        var holdArmed = false
        var dragMoved = false
        var classifiedCount = 0
        var classifiedMs = 0L
        var switching = false
        var fired = false
        var stepTravel = 0f
        var axis: Axis? = null
        var absX = 0f
        var absY = 0f
        val scrollSamples = ArrayDeque<ScrollSample>()

        init {
            rebase()
        }

        fun rebase() {
            val (x, y, s) = geometry()
            cx = x
            cy = y
            spread = s
            startX = x
            startY = y
            startSpread = s
        }

        fun geometry(): Geometry {
            var x = 0f
            var y = 0f
            for (contact in contacts.values) {
                x += contact.point.xDp
                y += contact.point.yDp
            }
            x /= contacts.size
            y /= contacts.size
            var radius = 0f
            if (contacts.size > 1) {
                for (contact in contacts.values) {
                    radius += hypot(contact.point.xDp - x, contact.point.yDp - y)
                }
            }
            return Geometry(x, y, 2f * radius / contacts.size)
        }
    }

    private companion object {
        const val TAP_MS = 300L
        const val TAP_DISTANCE_DP = 14f
        const val DOUBLE_TAP_MS = 320L
        const val DOUBLE_TAP_DISTANCE_DP = 70f
        const val HOLD_MS = 450L
        const val HOLD_SLOP_DP = 7f
        const val REPOSITION_MS = 380L
        const val MULTI_TAP_MS = 420L
        const val MULTI_TAP_DISTANCE_DP = 22f
        const val SETTLE_MS = 70L
        const val JOIN_MS = 220L
        const val MULTI_START_DP = 12f
        const val AXIS_RATIO = 1.7f
        const val PINCH_TICK_DP = 40f
        const val SWIPE_START_DP = 28f
        const val SWITCH_STEP_DP = 46f
        const val FRAME_MS = 16L
    }
}

/**
 * Compose adapter for the relative pad and other remote modes. Touch UI positions
 * are in pixels. [onSingleFinger] reports dp for custom aiming, with
 * [TouchpadGestureOptions.relativePointer] disabled. In custom aiming modes,
 * a supplied [onTap] sends the left click. In relative mode, it observes the click.
 *
 * [onPinch] brackets [onZoom] so a caller can hold Ctrl. Zoom deltas use NectarLink's
 * down-positive wheel convention. [onGestureStolen] reports interrupted 3+ finger input.
 */
internal suspend fun PointerInputScope.touchpadGestures(
    pcId: String,
    core: Core,
    sensitivity: Float,
    onStatus: (Core.RemoteAccess) -> Unit,
    onTouch: (Offset?) -> Unit = {},
    onDragging: (Boolean) -> Unit = {},
    onTap: (() -> Unit)? = null,
    options: TouchpadGestureOptions = TouchpadGestureOptions(),
    onAction: (TouchpadAction) -> Unit = {},
    onPinch: (Boolean) -> Unit = {},
    onZoom: (Float) -> Unit = {},
    onHaptic: (TouchpadHaptic) -> Unit = {},
    onGestureStolen: () -> Unit = {},
    onSingleFinger: (TouchpadSingleFinger) -> Unit = {},
    usePreciseMove: Boolean = false,
) {
    val engine = TouchpadGestureEngine(options) { event ->
        when (event) {
            is TouchpadGestureEvent.Move -> if (usePreciseMove) {
                core.remoteMovePrecise(pcId, event.dx * sensitivity, event.dy * sensitivity)
            } else {
                core.remoteMove(pcId, event.dx * sensitivity, event.dy * sensitivity)
            }
            is TouchpadGestureEvent.Button -> {
                val customLeftClick = !options.relativePointer && onTap != null &&
                    event.button == TouchpadButton.Left && event.state == TouchpadButtonState.Click
                if (!customLeftClick) {
                    core.remoteButton(pcId, event.button.name.lowercase(), event.state.name.lowercase(), onStatus)
                }
                if (event.state != TouchpadButtonState.Click) onDragging(event.state == TouchpadButtonState.Down)
            }
            is TouchpadGestureEvent.Scroll -> core.remoteScroll(pcId, event.dxNotches, event.dyNotches, fast = true)
            is TouchpadGestureEvent.Pinch -> onPinch(event.active)
            is TouchpadGestureEvent.Zoom -> onZoom(event.dyNotches)
            is TouchpadGestureEvent.Action -> onAction(event.action)
            is TouchpadGestureEvent.Haptic -> onHaptic(event.haptic)
            is TouchpadGestureEvent.SingleFinger -> onSingleFinger(event.event)
            TouchpadGestureEvent.LeftTap -> onTap?.invoke()
            TouchpadGestureEvent.Stolen -> onGestureStolen()
        }
    }

    val pixelsPerDp = density
    try {
        awaitPointerEventScope {
            var ignoreUntilLift = false
            var tracking = false
            while (true) {
                val deadline = engine.nextDeadlineMs
                val event = if (deadline == null) {
                    awaitPointerEvent()
                } else {
                    val now = SystemClock.uptimeMillis()
                    if (deadline <= now) {
                        engine.advanceTo(now)
                        continue
                    }
                    withTimeoutOrNull(deadline - now) { awaitPointerEvent() }
                } ?: continue

                val changes = event.changes
                if (changes.isEmpty()) continue
                val pressed = changes.filter { it.pressed }
                if (changes.any { it.isConsumed && (it.pressed || it.previousPressed) }) {
                    if (tracking) {
                        // Compose translates OS cancellation to synthetic, consumed releases.
                        val systemCancel = event.motionEvent == null && pressed.isEmpty() &&
                            changes.all { !it.previousPressed || it.isConsumed }
                        engine.cancel(stolenBySystem = systemCancel)
                        onTouch(null)
                        tracking = false
                    }
                    ignoreUntilLift = pressed.isNotEmpty()
                } else if (!ignoreUntilLift && (tracking || pressed.any { !it.previousPressed })) {
                    val contacts = pressed.map {
                        TouchpadContact(it.id.value, it.position.x / pixelsPerDp, it.position.y / pixelsPerDp)
                    }
                    onTouch(pressed.firstOrNull()?.position)
                    engine.onPointers(changes.maxOf { it.uptimeMillis }, contacts)
                    changes.forEach { it.consume() }
                    tracking = pressed.isNotEmpty()
                }
                if (pressed.isEmpty()) ignoreUntilLift = false
            }
        }
    } finally {
        engine.cancel()
        onTouch(null)
    }
}
