// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.ComponentName
import android.content.Context
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.TouchPhase
import kotlin.math.abs
import kotlin.math.hypot

/**
 * The PC's mouse and keyboard on this phone while its screen is mirrored
 * (Assist level, docs/protocol/mirror.md): an accessibility service, which
 * the user turns on once. Clicks become taps, holds long presses, drags
 * swipes (played when the button comes up: Android plays a gesture whole),
 * the wheel scrolls, and typing goes into the focused text field.
 */
class InputService : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())

    /** The finger's way so far, as fractions of the screen, and when it went down. */
    private val stroke = mutableListOf<Pair<Float, Float>>()
    private var downAt = 0L
    private var longPressFired = false

    private val longPressRunnable = Runnable {
        if (stroke.isEmpty() || longPressFired) return@Runnable
        val (w, h) = screen()
        val points = stroke.map { (px, py) -> clampScreenPoint(px, py, w, h) }
        if (isStationaryTapOrHold(points, w)) {
            longPressFired = true
            val start = points.first()
            gesture(Path().apply { moveTo(start.first, start.second) }, LONG_PRESS_DURATION_MS)
        }
    }

    // Wheel notches are gathered briefly into one swipe and queued while a gesture is in flight.
    private var scrollX = 0f
    private var scrollY = 0f
    private var scrollDx = 0f
    private var scrollDy = 0f
    private var gestureInFlight = false
    private val flushScroll = Runnable { flushScroll() }
    private val clearGestureInFlight = Runnable { onGestureFinished() }

    override fun onServiceConnected() {
        instance = this
        (application as NectarlinkApplication).core.refreshNotificationAccess()
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        main.removeCallbacksAndMessages(null)
        gestureInFlight = false
        instance = null
        (application as NectarlinkApplication).core.refreshNotificationAccess()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        gestureInFlight = false
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

    override fun onInterrupt() {}

    private fun handle(input: MirrorInputEvent) {
        when (input) {
            is MirrorInputEvent.Touch -> touch(input.action, input.x, input.y)
            is MirrorInputEvent.Scroll -> scroll(input.x, input.y, input.dx, input.dy)
            is MirrorInputEvent.Key -> key(input.key)
            is MirrorInputEvent.Text -> type(input.text)
            is MirrorInputEvent.Gamepad -> {
                app.nectarlink.android.elevated.Elevated.sendGamepad(
                    input.buttons.toInt(),
                    input.lx.toInt(),
                    input.ly.toInt(),
                    input.rx.toInt(),
                    input.ry.toInt(),
                    input.lt.toInt(),
                    input.rt.toInt(),
                )
            }
        }
    }

    // ---- Touch ----

    private fun touch(action: TouchPhase, x: Float, y: Float) {
        when (action) {
            TouchPhase.DOWN -> {
                main.removeCallbacks(longPressRunnable)
                stroke.clear()
                stroke.add(x to y)
                downAt = SystemClock.uptimeMillis()
                longPressFired = false
                main.postDelayed(longPressRunnable, LONG_PRESS_MS)
            }
            TouchPhase.MOVE -> if (stroke.isNotEmpty()) {
                stroke.add(x to y)
                if (!longPressFired) {
                    val (w, h) = screen()
                    val start = stroke.first().let { clampScreenPoint(it.first, it.second, w, h) }
                    val cur = clampScreenPoint(x, y, w, h)
                    if (hypot(cur.first - start.first, cur.second - start.second) >= TAP_SLOP_FRACTION * w) {
                        main.removeCallbacks(longPressRunnable)
                    }
                }
            }
            TouchPhase.UP -> {
                main.removeCallbacks(longPressRunnable)
                if (stroke.isEmpty()) return
                stroke.add(x to y)
                val held = SystemClock.uptimeMillis() - downAt
                val (w, h) = screen()
                val points = stroke.map { (px, py) -> clampScreenPoint(px, py, w, h) }
                stroke.clear()
                val wasLongPressFired = longPressFired
                longPressFired = false
                val start = points.first()
                if (isStationaryTapOrHold(points, w)) {
                    if (wasLongPressFired) return
                    val path = Path().apply { moveTo(start.first, start.second) }
                    val duration = if (held >= LONG_PRESS_MS) LONG_PRESS_DURATION_MS else TAP_MS
                    gesture(path, duration)
                } else {
                    val filtered = filterDragPoints(points)
                    val path = Path().apply {
                        moveTo(filtered.first().first, filtered.first().second)
                        filtered.drop(1).forEach { (px, py) -> lineTo(px, py) }
                    }
                    gesture(path, swipeDurationMs(held))
                }
            }
        }
    }

    private fun scroll(x: Float, y: Float, dx: Float, dy: Float) {
        scrollX = x
        scrollY = y
        scrollDx += dx
        scrollDy += dy
        main.removeCallbacks(flushScroll)
        if (!gestureInFlight) {
            main.postDelayed(flushScroll, SCROLL_GATHER_MS)
        }
    }

    private fun flushScroll() {
        main.removeCallbacks(flushScroll)
        if (gestureInFlight) return
        val (w, h) = screen()
        if (w < 2f || h < 2f) {
            scrollDx = 0f
            scrollDy = 0f
            return
        }
        val (rawX, rawY) = clampScreenPoint(scrollX, scrollY, w, h)
        val x = rawX.coerceIn(1f, (w - 1f).coerceAtLeast(1f))
        val y = rawY.coerceIn(1f, (h - 1f).coerceAtLeast(1f))
        // Wheel down shows what's below: the finger moves up.
        val dx = clampScrollDelta(x, -scrollDx * SCROLL_STEP_FRACTION * w, w)
        val dy = clampScrollDelta(y, -scrollDy * SCROLL_STEP_FRACTION * h, h)
        scrollDx = 0f
        scrollDy = 0f
        if (abs(dx) < 1f && abs(dy) < 1f) return
        gesture(Path().apply { moveTo(x, y); lineTo(x + dx, y + dy) }, SCROLL_MS)
    }

    private fun gesture(path: Path, durationMs: Long) {
        val clampedDuration = durationMs.coerceAtLeast(1L)
        val description = GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(path, 0, clampedDuration))
            .build()
        gestureInFlight = true
        main.removeCallbacks(clearGestureInFlight)
        main.postDelayed(clearGestureInFlight, maxOf(GESTURE_TIMEOUT_MS, clampedDuration + 150L))
        val dispatched = dispatchGesture(
            description,
            object : GestureResultCallback() {
                override fun onCompleted(gestureDescription: GestureDescription?) {
                    onGestureFinished()
                }

                override fun onCancelled(gestureDescription: GestureDescription?) {
                    onGestureFinished()
                }
            },
            main,
        )
        if (!dispatched) {
            Log.i(TAG, "a gesture wasn't played")
            onGestureFinished()
        }
    }

    private fun onGestureFinished() {
        main.removeCallbacks(clearGestureInFlight)
        gestureInFlight = false
        if (scrollDx != 0f || scrollDy != 0f) {
            flushScroll()
        }
    }

    /** The screen's size in pixels, as it's turned now. */
    private fun screen(): Pair<Float, Float> {
        val windows = getSystemService(WindowManager::class.java)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windows.maximumWindowMetrics.bounds.let { it.width().toFloat() to it.height().toFloat() }
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windows.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels.toFloat() to metrics.heightPixels.toFloat()
        }
    }

    // ---- Keys and text ----

    private fun key(key: String) {
        when (key) {
            "back" -> performGlobalAction(GLOBAL_ACTION_BACK)
            "home" -> performGlobalAction(GLOBAL_ACTION_HOME)
            "recents" -> performGlobalAction(GLOBAL_ACTION_RECENTS)
            "notifications" -> performGlobalAction(GLOBAL_ACTION_NOTIFICATIONS)
            "lock" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                performGlobalAction(GLOBAL_ACTION_LOCK_SCREEN)
            }
            "enter" -> focused()?.let { node ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
                }
            }
            "backspace" -> edit { text, start, end ->
                if (start != end) Edit(text.removeRange(start, end), start)
                else if (start > 0) {
                    val prev = text.offsetByCodePoints(start, -1)
                    Edit(text.removeRange(prev, start), prev)
                } else null
            }
            "delete" -> edit { text, start, end ->
                if (start != end) Edit(text.removeRange(start, end), start)
                else if (end < text.length) {
                    val next = text.offsetByCodePoints(end, 1)
                    Edit(text.removeRange(end, next), start)
                } else null
            }
            "left" -> edit { text, start, _ -> Edit(text, if (start > 0) text.offsetByCodePoints(start, -1) else 0) }
            "right" -> edit { text, _, end -> Edit(text, if (end < text.length) text.offsetByCodePoints(end, 1) else text.length) }
            "up" -> edit { text, start, _ ->
                val prevNewline = text.lastIndexOf('\n', (start - 1).coerceAtLeast(0))
                Edit(text, if (prevNewline >= 0) prevNewline else 0)
            }
            "down" -> edit { text, _, end ->
                val nextNewline = text.indexOf('\n', end)
                Edit(text, if (nextNewline >= 0) (nextNewline + 1).coerceAtMost(text.length) else text.length)
            }
            "tab" -> focused()?.focusSearch(android.view.View.FOCUS_FORWARD)?.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            "copy" -> focused()?.performAction(AccessibilityNodeInfo.ACTION_COPY)
            "cut" -> focused()?.let { node ->
                pendingText = null
                node.performAction(AccessibilityNodeInfo.ACTION_CUT)
            }
            "select_all" -> focused()?.takeIf { isNodeEditable(it) }?.let { node ->
                runCatching { node.refresh() }
                val length = if (node.isShowingHintText) 0 else node.text?.length ?: 0
                pendingText = null
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_SELECTION,
                    android.os.Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, length)
                    },
                )
            }
        }
    }

    private fun type(typed: String) = edit { text, start, end -> Edit(text.replaceRange(start, end, typed), start + typed.length) }

    private data class Edit(val text: String, val cursor: Int)

    private var pendingText: String? = null
    private var pendingCursor: Int = 0
    private var pendingAt: Long = 0L

    /** Changes the focused text field: its text and selection in, the new text and cursor out. */
    private fun edit(change: (String, Int, Int) -> Edit?) {
        val node = focused() ?: return
        if (!isNodeEditable(node)) return
        runCatching { node.refresh() }
        val showingHint = node.isShowingHintText
        val rawText = if (showingHint) "" else node.text?.toString().orEmpty()
        val now = SystemClock.uptimeMillis()
        val cachedPending = pendingText
        val usePending = cachedPending != null && (now - pendingAt) < 350L && rawText != cachedPending
        val text = if (usePending) cachedPending.orEmpty() else rawText
        val start = if (usePending || (rawText == cachedPending && node.textSelectionStart <= 0 && pendingCursor > 0)) {
            pendingCursor.coerceIn(0, text.length)
        } else {
            node.textSelectionStart.takeIf { it in 0..text.length } ?: text.length
        }
        val end = if (usePending || (rawText == cachedPending && node.textSelectionEnd <= 0 && pendingCursor > 0)) {
            start
        } else {
            node.textSelectionEnd.takeIf { it in start..text.length } ?: start
        }
        val result = change(text, start, end) ?: return
        pendingText = result.text
        pendingCursor = result.cursor
        pendingAt = now
        if (result.text != text || usePending) {
            node.performAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, result.text) },
            )
        }
        node.performAction(
            AccessibilityNodeInfo.ACTION_SET_SELECTION,
            Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, result.cursor)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, result.cursor)
            },
        )
    }

    private fun isNodeEditable(node: AccessibilityNodeInfo): Boolean =
        node.isEditable ||
            node.className == "android.widget.EditText" ||
            node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT }

    /**
     * Presses the Send button by the focused box on [display] (an app window's
     * own display), or on the phone's screen when it's null. Logs what it
     * found, never what's typed, so a report can say where it stopped (#220).
     */
    private fun pressAction(display: Int?): Boolean {
        val field = (if (display == null) focused() else focusedOn(display))
        if (field == null) {
            Log.i(TAG, "send: no text box on display ${display ?: 0}")
            return false
        }
        // Chat boxes take the keyboard's action and do nothing with it, and
        // they don't all say they're multi-line (Messenger's, Telegram's and
        // WhatsApp's don't), so press a Send button by the box when there is
        // one. Without one, a multi-line box gets a plain Enter for its new
        // line, and any other box the keyboard's action.
        val pressed = SendButton.press(field, field.window?.root ?: rootInActiveWindow)
        Log.i(TAG, "send: display ${display ?: 0}, multi-line ${field.isMultiLine}, Send button pressed $pressed")
        if (pressed) return true
        if (field.isMultiLine || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        return field.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
    }

    /** The text box with focus on [display], from that display's own windows. */
    private fun focusedOn(display: Int): AccessibilityNodeInfo? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        val roots = runCatching { windowsOnAllDisplays.get(display) }.getOrNull().orEmpty().mapNotNull { it.root }
        for (root in roots) {
            root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { isNodeEditable(it) }?.let { return it }
        }
        for (root in roots) {
            findEditable(root, requireFocused = true)?.let { return it }
        }
        return null
    }

    private fun focused(): AccessibilityNodeInfo? {
        findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { isNodeEditable(it) }?.let { return it }
        findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY)?.takeIf { isNodeEditable(it) }?.let { return it }
        val roots = buildList {
            rootInActiveWindow?.let { add(it) }
            runCatching { windows }.getOrNull()?.forEach { w -> w.root?.let { add(it) } }
        }
        for (root in roots) {
            findEditable(root, requireFocused = true)?.let { return it }
        }
        for (root in roots) {
            findEditable(root, requireFocused = false)?.let { return it }
        }
        return findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
    }

    private fun findEditable(node: AccessibilityNodeInfo?, requireFocused: Boolean): AccessibilityNodeInfo? {
        if (node == null) return null
        if (isNodeEditable(node) && (!requireFocused || node.isFocused)) return node
        for (i in 0 until node.childCount) {
            findEditable(node.getChild(i), requireFocused)?.let { return it }
        }
        return null
    }

    companion object {
        private const val TAG = "InputService"
        internal const val TAP_MS = 40L
        internal const val LONG_PRESS_MS = 520L
        internal const val LONG_PRESS_DURATION_MS = 550L
        internal const val SWIPE_MIN_MS = 80L
        internal const val SWIPE_MAX_MS = 600L
        internal const val GESTURE_TIMEOUT_MS = 700L
        internal const val TAP_SLOP_FRACTION = 0.015f
        internal const val DRAG_MIN_STEP_PX = 1.5f
        internal const val SCROLL_STEP_FRACTION = 0.12f
        internal const val SCROLL_MS = 75L
        internal const val SCROLL_GATHER_MS = 24L

        @Volatile private var instance: InputService? = null

        /** Whether the user turned it on (and Android has it running). */
        val running: Boolean get() = instance != null

        /** The PC's input; ignored while the service is off. */
        /** Presses the focused field's action, like Send. False if that isn't possible here. */
        /** Presses Send on [display] (an app window's), or on the phone's screen when null. */
        fun pressSend(display: Int? = null): Boolean = instance?.pressAction(display) == true

        fun handle(input: MirrorInputEvent) {
            val service = instance ?: return
            service.main.post { service.handle(input) }
        }

        /** Whether it's turned on in Settings (it may not be running yet). */
        fun enabled(context: Context): Boolean {
            val enabled = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES).orEmpty()
            val me = ComponentName(context, InputService::class.java)
            return enabled.split(':').any { ComponentName.unflattenFromString(it) == me }
        }

        /** Android's accessibility settings, where the user turns it on. */
        fun settingsIntent(): android.content.Intent =
            android.content.Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}

internal fun maxTravelledFromStart(points: List<Pair<Float, Float>>): Float {
    val start = points.firstOrNull() ?: return 0f
    var maxDist = 0f
    for (i in 1 until points.size) {
        val pt = points[i]
        val d = hypot(pt.first - start.first, pt.second - start.second)
        if (d > maxDist) maxDist = d
    }
    return maxDist
}

internal fun isStationaryTapOrHold(points: List<Pair<Float, Float>>, screenWidthPx: Float): Boolean =
    maxTravelledFromStart(points) < InputService.TAP_SLOP_FRACTION * screenWidthPx

internal fun filterDragPoints(
    points: List<Pair<Float, Float>>,
    minStepPx: Float = InputService.DRAG_MIN_STEP_PX,
): List<Pair<Float, Float>> {
    if (points.isEmpty()) return emptyList()
    val result = ArrayList<Pair<Float, Float>>(points.size)
    var prev = points.first()
    result.add(prev)
    for (i in 1 until points.size) {
        val pt = points[i]
        if (hypot(pt.first - prev.first, pt.second - prev.second) >= minStepPx) {
            result.add(pt)
            prev = pt
        }
    }
    return result
}

internal fun swipeDurationMs(heldMs: Long): Long =
    heldMs.coerceIn(InputService.SWIPE_MIN_MS, InputService.SWIPE_MAX_MS)

internal fun clampScreenPoint(normX: Float, normY: Float, screenW: Float, screenH: Float): Pair<Float, Float> {
    val safeW = screenW.takeIf { it.isFinite() && it > 0f } ?: 1f
    val safeH = screenH.takeIf { it.isFinite() && it > 0f } ?: 1f
    val maxW = (safeW - 1f).coerceAtLeast(0f)
    val maxH = (safeH - 1f).coerceAtLeast(0f)
    val nx = if (normX.isFinite()) normX.coerceIn(0f, 1f) else 0f
    val ny = if (normY.isFinite()) normY.coerceIn(0f, 1f) else 0f
    return (nx * safeW).coerceIn(0f, maxW) to (ny * safeH).coerceIn(0f, maxH)
}

internal fun clampScrollDelta(pos: Float, rawDelta: Float, screenExtent: Float): Float {
    val safeExtent = screenExtent.takeIf { it.isFinite() && it >= 2f } ?: return 0f
    val maxExtent = (safeExtent - 1f).coerceAtLeast(1f)
    val clampedPos = if (pos.isFinite()) pos.coerceIn(1f, maxExtent) else 1f
    val minDelta = (-clampedPos + 1f).coerceAtMost(0f)
    val maxDelta = (maxExtent - clampedPos).coerceAtLeast(0f)
    val safeRaw = if (rawDelta.isFinite()) rawDelta else 0f
    return safeRaw.coerceIn(minDelta, maxDelta)
}

