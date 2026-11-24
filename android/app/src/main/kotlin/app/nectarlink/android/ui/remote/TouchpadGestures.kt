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

/** How far (reference px) three or four fingers travel before a swipe counts. */
private const val SWIPE_PX = 60f
/** How far three fingers travel sideways to move the app switcher one app on. */
private const val SWITCHER_STEP_PX = 90f
/** How much two fingers spread or close (reference px) for one zoom step. */
private const val ZOOM_STEP_PX = 40f

/**
 * A laptop touchpad over this area: one finger moves the PC's pointer, a tap
 * clicks, tap then drag holds the button, two fingers scroll and a two-finger
 * tap right-clicks. With [gestures] (the PC offers `input.gestures`) it does
 * what a Windows precision touchpad does too: two fingers pinch to zoom; three
 * fingers up open Task View, down show the desktop, sideways go through the
 * app switcher (the app picked comes up when they lift) and a three-finger tap
 * opens Search; four fingers up and down do the same, sideways switch virtual
 * desktops, and a four-finger tap opens notifications. [onTouch] follows the
 * finger (null once it lifts), [onDragging] says when the button is held, and
 * [onTap] runs after a click.
 */
internal suspend fun PointerInputScope.touchpadGestures(
    pcId: String,
    core: Core,
    sensitivity: Float,
    onStatus: (Core.RemoteAccess) -> Unit,
    onTouch: (Offset?) -> Unit = {},
    onDragging: (Boolean) -> Unit = {},
    onTap: () -> Unit = {},
    gestures: Boolean = false,
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
        // Two fingers: scroll, or pinch once they spread or close.
        var pinching = false
        var twoDecided = false
        var spreadAtStart = -1f
        var lastSpread = -1f
        var zoomAccum = 0f
        var twoMoved = 0f
        // Three or four fingers: one swipe each, or the app switcher.
        var multiDx = 0f
        var multiDy = 0f
        var swiped = false
        var switcherOpen = false
        var switcherAccum = 0f
        val gesture = { name: String -> core.remoteGesture(pcId, name, onStatus) }

        onTouch(firstDown.position)

        // A drag holds the PC's left button: if the gesture is cancelled midway (the
        // screen goes away, the sensitivity changes), let go of it, or it stays held.
        try {
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
                    var cx = 0f
                    var cy = 0f
                    for (i in 0 until changes.size) {
                        val c = changes[i]
                        if (c.pressed) {
                            val d = c.positionChange()
                            avgDx += d.x
                            avgDy += d.y
                            cx += c.position.x
                            cy += c.position.y
                            c.consume()
                        }
                    }
                    avgDx /= pressedCount
                    avgDy /= pressedCount
                    cx /= pressedCount
                    cy /= pressedCount
                    totalMoved += hypot(avgDx, avgDy)

                    if (gestures && maxPointers >= 3) {
                        // Three or four fingers never scroll.
                        multiDx += avgDx * toReference
                        multiDy += avgDy * toReference
                        if (switcherOpen) {
                            switcherAccum += avgDx * toReference
                            while (switcherAccum >= SWITCHER_STEP_PX) {
                                gesture("switcher_next")
                                switcherAccum -= SWITCHER_STEP_PX
                            }
                            while (switcherAccum <= -SWITCHER_STEP_PX) {
                                gesture("switcher_previous")
                                switcherAccum += SWITCHER_STEP_PX
                            }
                        } else if (!swiped) {
                            if (abs(multiDy) >= SWIPE_PX && abs(multiDy) > abs(multiDx)) {
                                gesture(if (multiDy < 0) "task_view" else "show_desktop")
                                swiped = true
                            } else if (abs(multiDx) >= SWIPE_PX) {
                                if (maxPointers >= 4) {
                                    // The desktops follow the fingers, as on a touchpad.
                                    gesture(if (multiDx < 0) "desktop_next" else "desktop_previous")
                                    swiped = true
                                } else {
                                    gesture(if (multiDx > 0) "switcher_next" else "switcher_previous")
                                    switcherOpen = true
                                    switcherAccum = 0f
                                }
                            }
                        }
                        continue
                    }

                    if (gestures && pressedCount == 2) {
                        var spread = 0f
                        for (i in 0 until changes.size) {
                            val c = changes[i]
                            if (c.pressed) spread += hypot(c.position.x - cx, c.position.y - cy)
                        }
                        spread = spread / pressedCount * toReference
                        if (spreadAtStart < 0f) {
                            spreadAtStart = spread
                            lastSpread = spread
                        }
                        twoMoved += hypot(avgDx, avgDy) * toReference
                        if (!twoDecided) {
                            if (abs(spread - spreadAtStart) > tapSlop * toReference * 1.5f) {
                                pinching = true
                                twoDecided = true
                            } else if (twoMoved > tapSlop * toReference) {
                                twoDecided = true
                            }
                        }
                        if (pinching) {
                            zoomAccum += spread - lastSpread
                            lastSpread = spread
                            while (zoomAccum >= ZOOM_STEP_PX) {
                                gesture("zoom_in")
                                zoomAccum -= ZOOM_STEP_PX
                            }
                            while (zoomAccum <= -ZOOM_STEP_PX) {
                                gesture("zoom_out")
                                zoomAccum += ZOOM_STEP_PX
                            }
                            continue
                        }
                        lastSpread = spread
                        if (!twoDecided) continue
                    }

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
        } catch (e: kotlinx.coroutines.CancellationException) {
            if (dragStarted) {
                core.remoteButton(pcId, "left", "up", onStatus)
                onDragging(false)
            }
            if (switcherOpen) gesture("switcher_done")
            onTouch(null)
            throw e
        }

        onTouch(null)
        if (switcherOpen) gesture("switcher_done")
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
            } else if (gestures && maxPointers == 3) {
                lastSingleTapTime = 0L
                gesture("search")
            } else if (gestures && maxPointers >= 4) {
                lastSingleTapTime = 0L
                gesture("notifications")
            }
        } else {
            lastSingleTapTime = 0L
        }
    }
}
