// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import kotlin.math.hypot
import kotlin.math.pow
import kotlin.math.sqrt

/**
 * Maps phone gyroscope angular velocity (`Sensor.TYPE_GYROSCOPE`, in rad/s)
 * to relative PC pointer motion `(dx, dy)` in logical pixels, with a radial
 * dead zone against hand tremor and exponential smoothing.
 */
class AirMouseFilter(
    private val deadZoneRadPerSec: Float = DEFAULT_DEAD_ZONE_RAD_PER_SEC,
    private val smoothingAlpha: Float = DEFAULT_SMOOTHING_ALPHA,
    private val basePixelsPerRad: Float = DEFAULT_PIXELS_PER_RAD,
) {
    var smoothX: Float = 0f
        private set
    var smoothY: Float = 0f
        private set
    private var lastTimestampNs: Long = 0L

    /**
     * Processes a gyroscope sample at [timestampNs] and returns the relative
     * pointer step `(dx, dy)` in logical pixels.
     */
    fun onSample(
        wx: Float,
        wy: Float,
        wz: Float,
        timestampNs: Long,
        sensitivity: Float,
    ): Pair<Float, Float> {
        val dtSec = if (lastTimestampNs == 0L || timestampNs <= lastTimestampNs) {
            DEFAULT_DT_SEC
        } else {
            ((timestampNs - lastTimestampNs) * 1e-9f).coerceIn(MIN_DT_SEC, MAX_DT_SEC)
        }
        lastTimestampNs = timestampNs
        return step(wx, wy, wz, dtSec, sensitivity)
    }

    /**
     * Pure step with an explicit time delta [dtSec] (in seconds).
     *
     * Axis mapping:
     * - Horizontal (`dx` > 0 moves right): panning right rotates around -Z
     *   (when held flat) and -Y (when held upright), so `rawX = -(wz + wy)`.
     * - Vertical (`dy` > 0 moves down): tilting down rotates around -X, so
     *   `rawY = -wx`.
     */
    fun step(
        wx: Float,
        wy: Float,
        wz: Float,
        dtSec: Float,
        sensitivity: Float = 1.0f,
    ): Pair<Float, Float> {
        if (!wx.isFinite() || !wy.isFinite() || !wz.isFinite() || !dtSec.isFinite() || !sensitivity.isFinite()) {
            return 0f to 0f
        }
        val clampedDt = dtSec.coerceIn(MIN_DT_SEC, MAX_DT_SEC)
        val clampedSens = sensitivity.coerceIn(0.1f, 5.0f)

        val rawX = -(wz + wy)
        val rawY = -wx
        val (dzX, dzY) = applyDeadZone(rawX, rawY, deadZoneRadPerSec)

        val alpha = smoothingAlpha.coerceIn(0.05f, 1.0f)
        smoothX += alpha * (dzX - smoothX)
        smoothY += alpha * (dzY - smoothY)

        if (dzX == 0f && dzY == 0f && hypot(smoothX, smoothY) < SNAP_ZERO_EPSILON) {
            smoothX = 0f
            smoothY = 0f
            return 0f to 0f
        }

        val scale = clampedDt * basePixelsPerRad * clampedSens
        val dx = (smoothX * scale).coerceIn(-MAX_MOVE_DELTA, MAX_MOVE_DELTA)
        val dy = (smoothY * scale).coerceIn(-MAX_MOVE_DELTA, MAX_MOVE_DELTA)
        return dx to dy
    }

    /** Clears smoothed velocity and timestamp state when the pad is released. */
    fun reset() {
        smoothX = 0f
        smoothY = 0f
        lastTimestampNs = 0L
    }

    companion object {
        const val DEFAULT_DEAD_ZONE_RAD_PER_SEC = 0.04f
        const val DEFAULT_SMOOTHING_ALPHA = 0.35f
        const val DEFAULT_PIXELS_PER_RAD = 1800f
        const val DEFAULT_DT_SEC = 0.02f
        const val MIN_DT_SEC = 0.001f
        const val MAX_DT_SEC = 0.1f
        const val MAX_MOVE_DELTA = 4000f
        private const val SNAP_ZERO_EPSILON = 0.005f

        /**
         * Applies a continuous radial dead zone of [deadZone] rad/s so hand
         * tremor produces zero output while motion above the threshold ramps
         * smoothly from zero.
         */
        fun applyDeadZone(
            rawX: Float,
            rawY: Float,
            deadZone: Float = DEFAULT_DEAD_ZONE_RAD_PER_SEC,
        ): Pair<Float, Float> {
            val mag = hypot(rawX, rawY)
            if (mag <= deadZone || mag == 0f) return 0f to 0f
            val factor = (mag - deadZone) / mag
            return (rawX * factor) to (rawY * factor)
        }
    }
}

/**
 * Formats consecutive dictated utterances for typing into a PC field:
 * strips control characters, trims surrounding whitespace, and prepends a
 * single space between consecutive non-empty utterances.
 */
class UtteranceJoiner {
    var hasTypedUtterance: Boolean = false
        private set

    fun next(raw: String): String {
        val cleaned = sanitizeUtterance(raw)
        if (cleaned.isEmpty()) return ""
        val out = if (hasTypedUtterance) " $cleaned" else cleaned
        hasTypedUtterance = true
        return out
    }

    fun reset() {
        hasTypedUtterance = false
    }

    companion object {
        const val MAX_REMOTE_TEXT_BYTES = 256

        /**
         * Replaces control characters (newlines, tabs, etc.) with spaces and
         * collapses whitespace runs so the result is valid for `remote.input`
         * (`type = "text"`).
         */
        fun sanitizeUtterance(raw: String): String {
            if (raw.isEmpty()) return ""
            val sb = StringBuilder(raw.length)
            var lastWasSpace = true
            for (ch in raw) {
                if (ch.isISOControl() || ch == '\u0000' || ch.isWhitespace()) {
                    if (!lastWasSpace) {
                        sb.append(' ')
                        lastWasSpace = true
                    }
                } else {
                    sb.append(ch)
                    lastWasSpace = false
                }
            }
            if (sb.isNotEmpty() && sb[sb.length - 1] == ' ') {
                sb.setLength(sb.length - 1)
            }
            return sb.toString()
        }

        /**
         * Splits [text] on Unicode code-point boundaries into chunks of at most
         * [maxBytes] UTF-8 bytes (`docs/protocol/remote.md` §3: 1–256 bytes),
         * dropping control characters.
         */
        fun chunkUtf8(text: String, maxBytes: Int = MAX_REMOTE_TEXT_BYTES): List<String> {
            require(maxBytes >= 4) { "maxBytes must fit at least one UTF-8 code point" }
            val filtered = text.filter { !it.isISOControl() && it != '\u0000' }
            if (filtered.isEmpty()) return emptyList()
            val chunks = mutableListOf<String>()
            var start = 0
            var currentBytes = 0
            var i = 0
            while (i < filtered.length) {
                val codePoint = Character.codePointAt(filtered, i)
                val charCount = Character.charCount(codePoint)
                val utf8Bytes = when {
                    codePoint <= 0x7F -> 1
                    codePoint <= 0x7FF -> 2
                    codePoint <= 0xFFFF -> 3
                    else -> 4
                }
                if (currentBytes + utf8Bytes > maxBytes && i > start) {
                    chunks.add(filtered.substring(start, i))
                    start = i
                    currentBytes = 0
                }
                currentBytes += utf8Bytes
                i += charCount
            }
            if (start < filtered.length) {
                chunks.add(filtered.substring(start, filtered.length))
            }
            return chunks
        }
    }
}

/** Drift's velocity-dependent gain, with distances measured in dp. */
class TouchpadBallistics(
    pointerSpeed: Float = DEFAULT_POINTER_SPEED,
    acceleration: Float = DEFAULT_ACCELERATION,
) {
    private val pointerSpeed = pointerSpeed.coerceIn(MIN_POINTER_SPEED, MAX_POINTER_SPEED)
    private val acceleration = acceleration.coerceIn(MIN_ACCELERATION, MAX_ACCELERATION)
    private var speed = 0f

    /** The multiplier for [distance] in the original touchpad's reference pixels. */
    fun gain(distance: Float, dtMs: Long): Float = gainDp(distance / TOUCHPAD_REFERENCE_DENSITY, dtMs)

    /** The multiplier for [distanceDp] over [dtMs] milliseconds. */
    fun gainDp(distanceDp: Float, dtMs: Long): Float {
        if (distanceDp > 0f) {
            val instant = distanceDp / dtMs.coerceAtLeast(4L)
            speed = speed * 0.55f + instant * 0.45f
        }
        val t = ((speed - 0.1f) / 1.4f).coerceIn(0f, 1f)
        return pointerSpeed * (0.72f + (0.28f + acceleration * 3.2f) * t * t * (3f - 2f * t))
    }

    fun reset() {
        speed = 0f
    }

    companion object {
        const val DEFAULT_POINTER_SPEED = 1.6f
        const val MIN_POINTER_SPEED = 0.4f
        const val MAX_POINTER_SPEED = 4.0f
        const val DEFAULT_ACCELERATION = 0.65f
        const val MIN_ACCELERATION = 0f
        const val MAX_ACCELERATION = 1f
        const val DEFAULT_SCROLL_SPEED = 1f
        const val MIN_SCROLL_SPEED = 0.3f
        const val MAX_SCROLL_SPEED = 3f
        const val SCROLL_GAIN = 1.45f
        const val SCROLL_DP_PER_NOTCH = 100f
        const val PINCH_NOTCHES_PER_DP = 0.024f

        /** Converts finger travel in dp to NectarLink's right/down-positive wheel notches. */
        fun scrollNotches(
            dxDp: Float,
            dyDp: Float,
            scrollSpeed: Float = DEFAULT_SCROLL_SPEED,
            naturalScroll: Boolean = true,
        ): Pair<Float, Float> {
            val direction = if (naturalScroll) -1f else 1f
            val scale = direction * scrollSpeed.coerceIn(MIN_SCROLL_SPEED, MAX_SCROLL_SPEED) *
                SCROLL_GAIN / SCROLL_DP_PER_NOTCH
            return dxDp * scale to dyDp * scale
        }

        /** Increasing spread scrolls up with Ctrl held, zooming in. */
        fun pinchNotches(spreadChangeDp: Float): Float = -spreadChangeDp * PINCH_NOTCHES_PER_DP
    }
}

internal const val TOUCHPAD_REFERENCE_DENSITY = 2.625f

/** Screen-right expressed in the device's unrotated sensor coordinate system. */
enum class DisplayRotation(internal val rightX: Float, internal val rightY: Float) {
    Rotation0(1f, 0f),
    Rotation90(0f, -1f),
    Rotation180(-1f, 0f),
    Rotation270(0f, 1f),
    ;

    companion object {
        /** Matches the values returned by Android's Display.getRotation(). */
        fun fromSurfaceRotation(rotation: Int): DisplayRotation = entries.getOrElse(rotation) { Rotation0 }
    }
}

/** Air aiming by world yaw and screen-relative pitch, independent of handset roll. */
object AirMouseProjection {
    const val DEAD_ZONE_RAD_PER_SEC = 0.02f
    const val DEFAULT_SPEED = 1f
    const val MIN_SPEED = 0.3f
    const val MAX_SPEED = 3f

    /** Returns yaw and pitch rates in rad/s. Gravity and gyro share device axes. */
    fun rates(
        wx: Float,
        wy: Float,
        wz: Float,
        gx: Float,
        gy: Float,
        gz: Float,
        rotation: DisplayRotation = DisplayRotation.Rotation0,
    ): Pair<Float, Float> {
        val gravity = sqrt(gx * gx + gy * gy + gz * gz)
        if (!gravity.isFinite() || gravity < 1f || !wx.isFinite() || !wy.isFinite() || !wz.isFinite()) {
            return 0f to 0f
        }
        val ux = gx / gravity
        val uy = gy / gravity
        val uz = gz / gravity
        val yaw = wx * ux + wy * uy + wz * uz

        val ax = rotation.rightX
        val ay = rotation.rightY
        val au = ax * ux + ay * uy
        val hx = ax - au * ux
        val hy = ay - au * uy
        val hz = -au * uz
        val horizontal = sqrt(hx * hx + hy * hy + hz * hz)
        val pitch = if (horizontal > 0.2f) (wx * hx + wy * hy + wz * hz) / horizontal else wx * ax + wy * ay
        return yaw to pitch
    }

    /** Converts the integrated angles in one frame to pointer pixels. */
    fun pointerDelta(
        yawRadians: Float,
        pitchRadians: Float,
        dtSec: Float,
        airSpeed: Float = DEFAULT_SPEED,
    ): Pair<Float, Float> {
        if (!dtSec.isFinite() || dtSec <= 0f) return 0f to 0f
        val rate = hypot(yawRadians, pitchRadians) / dtSec
        if (!rate.isFinite() || rate <= DEAD_ZONE_RAD_PER_SEC) return 0f to 0f
        val gain = 1150f * airSpeed.coerceIn(MIN_SPEED, MAX_SPEED) * (0.55f + 1.1f * (rate / 2.2f).coerceAtMost(1f))
        return -yawRadians * gain to -pitchRadians * gain
    }
}

object StickCurve {
    const val RADIUS_DP = 64f
    const val DEAD_ZONE = 0.1f
    const val DEFAULT_SPEED = 1f
    const val MIN_SPEED = 0.3f
    const val MAX_SPEED = 3f

    /** Pointer velocity in px/s for a thumb displacement in dp. */
    fun velocity(dxDp: Float, dyDp: Float, stickSpeed: Float = DEFAULT_SPEED): Pair<Float, Float> {
        val distance = hypot(dxDp, dyDp)
        if (!distance.isFinite() || distance <= RADIUS_DP * DEAD_ZONE) return 0f to 0f
        val magnitude = (distance / RADIUS_DP).coerceAtMost(1f)
        val travel = (magnitude - DEAD_ZONE) / (1f - DEAD_ZONE)
        val speed = 2300f * stickSpeed.coerceIn(MIN_SPEED, MAX_SPEED) * travel.pow(1.8f)
        return dxDp / distance * speed to dyDp / distance * speed
    }
}

data class MonitorRect(
    val name: String,
    val x: Int,
    val y: Int,
    val width: Int,
    val height: Int,
    val primary: Boolean = false,
)

data class ScreenMapRect(val x: Float, val y: Float, val width: Float, val height: Float)

/** Aspect-fits the virtual desktop in a pad measured in dp, including gaps between monitors. */
class ScreenMapTransform(
    monitors: List<MonitorRect>,
    padWidthDp: Float,
    padHeightDp: Float,
) {
    val monitors = monitors.filter { it.width > 0 && it.height > 0 }
        .ifEmpty { listOf(MonitorRect("Display", 0, 0, 1920, 1080, primary = true)) }
    val virtualDesktop: ScreenMapRect
    val mapBounds: ScreenMapRect
    private val scale: Float

    init {
        val minX = this.monitors.minOf { it.x }
        val minY = this.monitors.minOf { it.y }
        val width = (this.monitors.maxOf { it.x.toLong() + it.width } - minX).toFloat()
        val height = (this.monitors.maxOf { it.y.toLong() + it.height } - minY).toFloat()
        virtualDesktop = ScreenMapRect(minX.toFloat(), minY.toFloat(), width, height)

        val padWidth = padWidthDp.coerceAtLeast(0f)
        val padHeight = padHeightDp.coerceAtLeast(0f)
        scale = minOf(
            (padWidth - 2f * INSET_DP).coerceAtLeast(0f) / width,
            (padHeight - 2f * INSET_DP).coerceAtLeast(0f) / height,
        )
        val mapWidth = width * scale
        val mapHeight = height * scale
        mapBounds = ScreenMapRect((padWidth - mapWidth) / 2f, (padHeight - mapHeight) / 2f, mapWidth, mapHeight)
    }

    fun monitorBounds(monitor: MonitorRect): ScreenMapRect = ScreenMapRect(
        mapBounds.x + (monitor.x - virtualDesktop.x) * scale,
        mapBounds.y + (monitor.y - virtualDesktop.y) * scale,
        monitor.width * scale,
        monitor.height * scale,
    )

    /** Pad point in dp to virtual desktop fractions in [0, 1], clamped at the edges. */
    fun normalizedAt(xDp: Float, yDp: Float): Pair<Float, Float> {
        if (scale == 0f) return 0.5f to 0.5f
        return ((xDp - mapBounds.x) / mapBounds.width).coerceIn(0f, 1f) to
            ((yDp - mapBounds.y) / mapBounds.height).coerceIn(0f, 1f)
    }

    fun padPosition(normalizedX: Float, normalizedY: Float): Pair<Float, Float> =
        (mapBounds.x + normalizedX.coerceIn(0f, 1f) * mapBounds.width) to
            (mapBounds.y + normalizedY.coerceIn(0f, 1f) * mapBounds.height)

    /** Looks up a monitor at a pad point in dp. Gaps and the outer inset return null. */
    fun hitTest(xDp: Float, yDp: Float): MonitorRect? = monitors.firstOrNull { monitor ->
        val rect = monitorBounds(monitor)
        xDp >= rect.x && xDp < rect.x + rect.width && yDp >= rect.y && yDp < rect.y + rect.height
    }

    companion object {
        const val INSET_DP = 16f
    }
}
