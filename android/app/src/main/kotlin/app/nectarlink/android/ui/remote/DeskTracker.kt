// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.SystemClock
import kotlin.math.abs
import kotlin.math.exp
import kotlin.math.hypot
import kotlin.math.sign
import kotlin.math.sqrt

enum class DeskStatus { Ready, Tracking, Lifted, Tilted }

/** Sensor motion since the last drain, in screen axes (metres and radians). */
data class DeskMotion(
    val xMetres: Double,
    val yMetres: Double,
    val twistRadians: Double,
    val status: DeskStatus,
) {
    /** Pointer pixels from a frame's motion, with positive Y heading down. */
    fun pointerDelta(
        dtSec: Double,
        deskSpeed: Float = DeskTracker.DEFAULT_SPEED,
        deskTwist: Boolean = true,
    ): Pair<Float, Float> {
        val speed = deskSpeed.coerceIn(DeskTracker.MIN_SPEED, DeskTracker.MAX_SPEED)
        val velocity = hypot(xMetres, yMetres) / dtSec.coerceAtLeast(0.001)
        val t = ((velocity - 0.04) / 0.5).coerceIn(0.0, 1.0)
        val gain = 21000 * speed * (0.6 + 0.9 * t * t * (3.0 - 2.0 * t))
        val twistPixels = if (deskTwist) -650 * speed * twistRadians else 0.0
        return (xMetres * gain + twistPixels).toFloat() to (-yMetres * gain).toFloat()
    }
}

/** Integrates Drift's desk strokes. Feed and drain from the same thread. */
class DeskSensorEngine(
    sensitivity: Float = DeskTracker.DEFAULT_SENSITIVITY,
    rotation: DisplayRotation = DisplayRotation.Rotation0,
) {
    var sensitivity = sensitivity.coerceIn(0f, 1f)
        set(value) {
            field = value.coerceIn(0f, 1f)
        }
    var rotation = rotation
    var status = DeskStatus.Ready
        private set

    private val x = StrokeAxis()
    private val y = StrokeAxis()
    private var twist = 0.0
    private var gyroMagnitude = 0f
    private var quietSamples = 0
    private var lifted = false
    private var flat = true
    private var lastAccelNs: Long? = null
    private var lastGyroNs: Long? = null

    /** Clears motion and timestamps while retaining the learned resting bias. */
    fun reset() {
        x.reset()
        y.reset()
        twist = 0.0
        quietSamples = 0
        lifted = false
        status = DeskStatus.Ready
        lastAccelNs = null
        lastGyroNs = null
    }

    fun onGravity(gx: Float, gy: Float, gz: Float) {
        val magnitude = sqrt(gx * gx + gy * gy + gz * gz)
        flat = magnitude > 1f && gz / magnitude > FLAT_COS
    }

    fun onGyro(wx: Float, wy: Float, wz: Float, timestampNs: Long) {
        gyroMagnitude = sqrt(wx * wx + wy * wy + wz * wz)
        val dt = secondsSince(lastGyroNs, timestampNs)
        lastGyroNs = timestampNs
        if (dt > 0.0 && dt < MAX_SAMPLE_SECONDS && flat && !lifted && abs(wz) > TWIST_DEAD_ZONE) {
            twist += wz * dt
        }
    }

    fun onAcceleration(ax: Float, ay: Float, az: Float, timestampNs: Long) {
        val dt = secondsSince(lastAccelNs, timestampNs)
        lastAccelNs = timestampNs
        if (dt <= 0.0 || dt > MAX_SAMPLE_SECONDS) return

        x.filter(ax)
        y.filter(ay)
        if (!flat) {
            stopStroke()
            status = DeskStatus.Tilted
            return
        }
        if (abs(az) > LIFT_ACCEL) {
            lifted = true
            stopStroke()
        }

        val stillAccel = 0.16f - 0.10f * sensitivity
        val quiet = hypot(x.correctedAccel, y.correctedAccel) < stillAccel &&
            gyroMagnitude < STILL_GYRO && abs(az) < 0.6
        quietSamples = if (quiet) (quietSamples + 1).coerceAtMost(STILL_SAMPLES) else 0
        if (quietSamples >= STILL_SAMPLES) {
            stopStroke()
            lifted = false
            status = DeskStatus.Ready
            if (abs(x.filteredAccel) < 0.3 && abs(y.filteredAccel) < 0.3) {
                x.learnBias()
                y.learnBias()
            }
            return
        }
        if (lifted) {
            status = DeskStatus.Lifted
            return
        }
        status = DeskStatus.Tracking

        val deadband = stillAccel * 0.6f
        val leak = exp(-dt / LEAK_SECONDS)
        x.integrate(dt, deadband, leak)
        y.integrate(dt, deadband, leak)
    }

    fun readMotion(): DeskMotion {
        val dx = x.drain()
        val dy = y.drain()
        val rx = rotation.rightX
        val ry = rotation.rightY
        val motion = DeskMotion(dx * rx + dy * ry, -dx * ry + dy * rx, twist, status)
        twist = 0.0
        return motion
    }

    private fun stopStroke() {
        x.stop()
        y.stop()
    }

    private class StrokeAxis {
        var filteredAccel = 0f
            private set
        private var bias = 0.0
        private var velocity = 0.0
        private var displacement = 0.0
        private var direction = 0

        val correctedAccel: Double
            get() = filteredAccel - bias

        fun filter(sample: Float) {
            filteredAccel = filteredAccel * 0.5f + sample * 0.5f
        }

        fun learnBias() {
            bias += correctedAccel * 0.03
        }

        fun integrate(dt: Double, deadband: Float, leak: Double) {
            val corrected = correctedAccel
            val accel = (abs(corrected) - deadband).coerceAtLeast(0.0) * sign(corrected)
            velocity = (velocity + accel * dt) * leak

            val nextDirection = if (velocity > 0) 1 else -1
            val threshold = if (direction == 0) START_VELOCITY else REVERSE_VELOCITY
            if (direction != nextDirection && abs(velocity) > threshold) direction = nextDirection
            if (direction == nextDirection) displacement += velocity * dt
        }

        fun stop() {
            velocity = 0.0
            direction = 0
        }

        fun reset() {
            stop()
            filteredAccel = 0f
            displacement = 0.0
        }

        fun drain(): Double = displacement.also { displacement = 0.0 }
    }

    private companion object {
        const val STILL_GYRO = 0.07f
        const val STILL_SAMPLES = 16
        const val LEAK_SECONDS = 0.45
        const val START_VELOCITY = 0.015
        const val REVERSE_VELOCITY = 0.07
        const val LIFT_ACCEL = 2.2f
        const val FLAT_COS = 0.86f
        const val TWIST_DEAD_ZONE = 0.03
        const val MAX_SAMPLE_SECONDS = 0.1

        fun secondsSince(previous: Long?, timestamp: Long): Double =
            if (previous == null) 0.0 else (timestamp - previous) / 1e9
    }
}

/** Android sensor registration and frame drains. Use from the main thread. */
class DeskTracker(
    private val sensors: SensorManager,
    private val engine: DeskSensorEngine = DeskSensorEngine(),
) : SensorEventListener {
    private val linear = sensors.getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION)
    private val gyro = sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    private val gravity = sensors.getDefaultSensor(Sensor.TYPE_GRAVITY)
    private var lastReadNs = 0L

    val isAvailable: Boolean get() = linear != null && gyro != null && gravity != null
    var isRunning = false
        private set
    var sensitivity: Float
        get() = engine.sensitivity
        set(value) { engine.sensitivity = value }
    var rotation: DisplayRotation
        get() = engine.rotation
        set(value) { engine.rotation = value }
    var deskSpeed = DEFAULT_SPEED
    var deskTwist = true

    fun start(): Boolean {
        if (isRunning) return true
        if (!isAvailable) return false
        engine.reset()
        lastReadNs = SystemClock.elapsedRealtimeNanos()
        isRunning = sensors.registerListener(this, gravity, GRAVITY_PERIOD_US) &&
            sensors.registerListener(this, gyro, MOTION_PERIOD_US) &&
            sensors.registerListener(this, linear, MOTION_PERIOD_US)
        if (!isRunning) sensors.unregisterListener(this)
        return isRunning
    }

    fun stop() {
        if (isRunning) sensors.unregisterListener(this)
        isRunning = false
        engine.reset()
    }

    /** Drain once per UI frame. The timestamp uses Android's elapsed realtime clock. */
    fun readFrame(timestampNs: Long = SystemClock.elapsedRealtimeNanos()): DeskFrame {
        val motion = engine.readMotion()
        val (dx, dy) = motion.pointerDelta((timestampNs - lastReadNs) / 1e9, deskSpeed, deskTwist)
        lastReadNs = timestampNs
        return DeskFrame(dx, dy, motion.status)
    }

    override fun onSensorChanged(event: SensorEvent) {
        if (!isRunning || event.values.size < 3) return
        val (x, y, z) = event.values
        when (event.sensor.type) {
            Sensor.TYPE_GRAVITY -> engine.onGravity(x, y, z)
            Sensor.TYPE_GYROSCOPE -> engine.onGyro(x, y, z, event.timestamp)
            Sensor.TYPE_LINEAR_ACCELERATION -> engine.onAcceleration(x, y, z, event.timestamp)
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    companion object {
        const val DEFAULT_SPEED = 1f
        const val MIN_SPEED = 0.3f
        const val MAX_SPEED = 3f
        const val DEFAULT_SENSITIVITY = 0.5f
        const val MOTION_PERIOD_US = 5_000
        const val GRAVITY_PERIOD_US = 10_000
    }
}

data class DeskFrame(val dx: Float, val dy: Float, val status: DeskStatus)
