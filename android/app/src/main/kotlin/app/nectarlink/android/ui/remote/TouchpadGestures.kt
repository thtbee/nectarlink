// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.positionChange
import app.nectarlink.android.core.Core
import kotlin.math.abs
import kotlin.math.hypot

/** Movement in reference-density pixels, so a swipe moves the pointer the same on every phone. */
internal const val TOUCHPAD_REFERENCE_DENSITY = 2.625f

/**
 * A laptop touchpad over this area: one finger moves the PC's pointer, a tap
 * clicks, tap then drag holds the button, two fingers scroll and a two-finger
 * tap right-clicks. [onTouch] follows the finger (null once it lifts),
 * [onDragging] says when the button is held, and [onTap] runs after a click.
 */
internal suspend fun PointerInputScope.touchpadGestures(
    pcId: String,
    core: Core,
    sensitivity: Float,
    onStatus: (Core.RemoteAccess) -> Unit,
    onTouch: (Offset?) -> Unit = {},
    onDragging: (Boolean) -> Unit = {},
    onTap: () -> Unit = {},
) {
    val tapSlop = viewConfiguration.touchSlop
    val toReference = TOUCHPAD_REFERENCE_DENSITY / density
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
        val ballistics = TouchpadBallistics()

        onTouch(firstDown.position)

        while (true) {
            val event = awaitPointerEvent()
            val changes = event.changes
            var pressedCount = 0
            var firstPressed: PointerInputChange? = null
            for (i in 0 until changes.size) {
                val c = changes[i]
                if (c.pressed) {
                    if (firstPressed == null) firstPressed = c
                    pressedCount++
                }
            }
            if (pressedCount == 0 || firstPressed == null) break

            if (pressedCount > maxPointers) {
                maxPointers = pressedCount
                if (dragStarted) {
                    core.remoteButton(pcId, "left", "up", onStatus)
                    dragStarted = false
                    onDragging(false)
                }
                isDragHold = false
            }

            if (pressedCount == 1 && maxPointers == 1) {
                val change = firstPressed
                onTouch(change.position)
                val delta = change.positionChange()
                val dist = hypot(delta.x, delta.y)
                totalMoved += dist

                if (isDragHold && !dragStarted && totalMoved > tapSlop * 0.6f) {
                    dragStarted = true
                    onDragging(true)
                    core.remoteButton(pcId, "left", "down", onStatus)
                }

                if (dist > 0f) {
                    val dtMs = change.uptimeMillis - change.previousUptimeMillis
                    val gain = ballistics.gain(dist * toReference, dtMs)
                    core.remoteMove(
                        pcId,
                        delta.x * toReference * gain * sensitivity,
                        delta.y * toReference * gain * sensitivity,
                    )
                    change.consume()
                }
            } else if (pressedCount >= 2) {
                onTouch(firstPressed.position)
                var avgDx = 0f
                var avgDy = 0f
                for (i in 0 until changes.size) {
                    val c = changes[i]
                    if (c.pressed) {
                        val d = c.positionChange()
                        avgDx += d.x
                        avgDy += d.y
                        c.consume()
                    }
                }
                avgDx /= pressedCount
                avgDy /= pressedCount
                totalMoved += hypot(avgDx, avgDy)

                scrollAccumX += avgDx * toReference
                scrollAccumY += avgDy * toReference
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

        onTouch(null)
        val elapsed = System.currentTimeMillis() - downTime
        if (dragStarted) {
            core.remoteButton(pcId, "left", "up", onStatus)
            onDragging(false)
            lastSingleTapTime = 0L
        } else if (elapsed <= tapTimeoutMs && totalMoved <= tapSlop * 1.5f) {
            if (maxPointers == 1) {
                lastSingleTapTime = System.currentTimeMillis()
                core.remoteButton(pcId, "left", "click", onStatus)
                onTap()
            } else if (maxPointers == 2) {
                lastSingleTapTime = 0L
                core.remoteButton(pcId, "right", "click", onStatus)
            }
        } else {
            lastSingleTapTime = 0L
        }
    }
}
