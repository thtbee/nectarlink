// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import kotlin.math.hypot

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
