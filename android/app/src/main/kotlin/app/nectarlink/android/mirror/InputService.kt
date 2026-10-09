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

    // Wheel notches are gathered briefly into one swipe.
    private var scrollX = 0f
    private var scrollY = 0f
    private var scrollDx = 0f
    private var scrollDy = 0f
    private val flushScroll = Runnable { scroll() }

    override fun onServiceConnected() {
        instance = this
        (application as NectarlinkApplication).core.refreshNotificationAccess()
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        instance = null
        (application as NectarlinkApplication).core.refreshNotificationAccess()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

    override fun onInterrupt() {}

    private fun handle(input: MirrorInputEvent) {
        when (input) {
            is MirrorInputEvent.Touch -> touch(input.action, input.x, input.y)
            is MirrorInputEvent.Scroll -> {
                scrollX = input.x
                scrollY = input.y
                scrollDx += input.dx
                scrollDy += input.dy
                main.removeCallbacks(flushScroll)
                main.postDelayed(flushScroll, SCROLL_GATHER_MS)
            }
            is MirrorInputEvent.Key -> key(input.key)
            is MirrorInputEvent.Text -> type(input.text)
        }
    }

    // ---- Touch ----

    private fun touch(action: TouchPhase, x: Float, y: Float) {
        when (action) {
            TouchPhase.DOWN -> {
                stroke.clear()
                stroke += x to y
                downAt = SystemClock.uptimeMillis()
            }
            TouchPhase.MOVE -> if (stroke.isNotEmpty()) stroke += x to y
            TouchPhase.UP -> {
                if (stroke.isEmpty()) return
                stroke += x to y
                val held = SystemClock.uptimeMillis() - downAt
                val (w, h) = screen()
                val points = stroke.map { (px, py) -> px * w to py * h }
                stroke.clear()
                val travelled = points.zipWithNext { a, b -> hypot(b.first - a.first, b.second - a.second) }.sum()
                val path = Path().apply {
                    moveTo(points.first().first, points.first().second)
                    points.drop(1).forEach { (px, py) -> lineTo(px, py) }
                }
                val duration = when {
                    // Barely moved: a tap, or a long press when held.
                    travelled < TAP_SLOP_FRACTION * w -> if (held >= LONG_PRESS_MS) held.coerceAtMost(MAX_GESTURE_MS) else TAP_MS
                    else -> held.coerceIn(MIN_SWIPE_MS, MAX_GESTURE_MS)
                }
                gesture(path, duration)
            }
        }
    }

    private fun scroll() {
        val (w, h) = screen()
        val x = scrollX * w
        val y = scrollY * h
        // Wheel down shows what's below: the finger moves up.
        val dx = (-scrollDx * SCROLL_STEP_FRACTION * w).coerceIn(-x + 1, w - x - 1)
        val dy = (-scrollDy * SCROLL_STEP_FRACTION * h).coerceIn(-y + 1, h - y - 1)
        scrollDx = 0f
        scrollDy = 0f
        if (abs(dx) < 1 && abs(dy) < 1) return
        gesture(Path().apply { moveTo(x, y); lineTo(x + dx, y + dy) }, SCROLL_MS)
    }

    private fun gesture(path: Path, durationMs: Long) {
        val description = GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(path, 0, durationMs.coerceAtLeast(1)))
            .build()
        if (!dispatchGesture(description, null, null)) Log.i(TAG, "a gesture wasn't played")
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
        val showingHint = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && node.isShowingHintText
        val rawText = if (showingHint) "" else node.text?.toString().orEmpty()
        val now = SystemClock.uptimeMillis()
        val usePending = pendingText != null && (now - pendingAt) < 350L && rawText != pendingText
        val text = if (usePending) pendingText!! else rawText
        val start = if (usePending || (rawText == pendingText && node.textSelectionStart <= 0 && pendingCursor > 0)) {
            pendingCursor.coerceIn(0, text.length)
        } else {
            node.textSelectionStart.takeIf { it in 0..text.length } ?: text.length
        }
        val end = if (usePending || (rawText == pendingText && node.textSelectionEnd <= 0 && pendingCursor > 0)) {
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
        private const val TAP_MS = 40L
        private const val LONG_PRESS_MS = 450L
        private const val MIN_SWIPE_MS = 80L
        private const val MAX_GESTURE_MS = 3000L
        private const val TAP_SLOP_FRACTION = 0.015f
        private const val SCROLL_STEP_FRACTION = 0.12f
        private const val SCROLL_MS = 120L
        private const val SCROLL_GATHER_MS = 40L

        @Volatile private var instance: InputService? = null

        /** Whether the user turned it on (and Android has it running). */
        val running: Boolean get() = instance != null

        /** The PC's input; ignored while the service is off. */
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
