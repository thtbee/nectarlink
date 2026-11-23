// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mic

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AudioEffect
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.audio.AudioRouting
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.roundToInt
import kotlin.math.sqrt

/** Streams 5 ms mono PCM blocks over the existing microphone service protocol. */
class MicService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val core get() = (application as NectarlinkApplication).core
    private var capture: Capture? = null
    private var desiredPc: String? = null
    private var deviceWatch: AutoCloseable? = null
    @Volatile private var deviceRevision = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val pcId = intent.getStringExtra(EXTRA_PC)
                if (pcId == null || !hasPermission(this)) {
                    if (capture == null) stopAfterForeground()
                } else {
                    val active = capture
                    val replace = intent.getBooleanExtra(EXTRA_REPLACE, false)
                    val target = if (active == null || replace) pcId else desiredPc ?: active.pcId
                    createChannel(this)
                    if (!promote(target)) {
                        if (active == null) stopSelf(startId)
                        return START_NOT_STICKY
                    }
                    desiredPc = target
                    if (active == null) {
                        startCapture(target)
                    } else if (active.pcId != target && active.running) {
                        active.stop()
                        core.micStop(active.pcId)
                    }
                }
            }
            ACTION_STOP -> {
                val requestedPc = intent.getStringExtra(EXTRA_PC)
                if (requestedPc == null || requestedPc == desiredPc) {
                    val activePc = capture?.pcId
                    desiredPc = null
                    capture?.stop()
                    if (intent.getBooleanExtra(EXTRA_FROM_USER, false) && activePc != null) core.micStop(activePc)
                    if (activePc == null) stopSelf(startId)
                }
            }
            else -> if (capture == null) stopAfterForeground()
        }
        return START_NOT_STICKY
    }

    private fun promote(pcId: String): Boolean {
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        } else 0
        return runCatching { ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(pcId), type) }.isSuccess
    }

    private fun stopAfterForeground() {
        createChannel(this)
        promote("")
        stopSelf()
    }

    private fun startCapture(pcId: String) {
        val session = Capture(pcId)
        capture = session
        _streaming.value = pcId
        AudioRouting.updateMicStreaming(pcId)
        if (deviceWatch == null) {
            deviceWatch = watchInputs(this) {
                deviceRevision++
                AudioRouting.updateMicInputs(it)
            }
        }

        scope.launch {
            var failed = false
            try {
                withContext(Dispatchers.IO) {
                    val stream = core.micOpen(pcId)
                    if (stream == null) {
                        failed = true
                        return@withContext
                    }
                    try {
                        failed = streamMicrophone(session, stream)
                    } finally {
                        runCatching { stream.end() }
                        stream.close()
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                failed = true
            } finally {
                session.stop()
                if (capture === session) {
                    val next = desiredPc?.takeUnless { it == pcId }
                    capture = null
                    if (next != null) {
                        startCapture(next)
                    } else {
                        desiredPc = null
                        _streaming.value = null
                        AudioRouting.updateMicStreaming(null)
                        if (failed) core.micStop(pcId)
                        stopSelf()
                    }
                }
            }
        }
    }

    private fun streamMicrophone(session: Capture, stream: MirrorStream): Boolean {
        val manager = getSystemService(AudioManager::class.java) ?: return true
        runCatching { Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO) }
        var preference = AudioRouting.state.value.mic.input
        var failedDevice: Int? = null
        var stalledDevice: Int? = null
        var revision = deviceRevision
        val clock = StreamClock()
        while (session.running) {
            val selected = AudioRouting.state.value.mic.input
            if (preference != selected) {
                preference = selected
                failedDevice = null
                stalledDevice = null
            }
            val devices = inputDevices(manager)
            if (failedDevice != null && devices.none { it.id == failedDevice }) failedDevice = null
            val input = preferredDevice(devices, preference, failedDevice)
            revision = deviceRevision
            val result = recordDevice(session, manager, input, stream, clock) {
                val latest = AudioRouting.state.value.mic.input
                if (latest != preference) return@recordDevice true
                val latestRevision = deviceRevision
                if (latestRevision == revision) return@recordDevice false
                revision = latestRevision
                val available = inputDevices(manager)
                if (failedDevice != null && available.none { it.id == failedDevice }) failedDevice = null
                preferredDevice(available, preference, failedDevice)?.id != input?.id
            }
            when (result) {
                CaptureResult.ENDED -> return false
                CaptureResult.INPUT_CHANGED -> Unit
                CaptureResult.STALLED -> {
                    if (!session.running) return false
                    val external = input?.takeIf { inputType(it) != AudioRouting.MicInputType.BUILTIN } ?: return true
                    if (stalledDevice == external.id) failedDevice = external.id
                    stalledDevice = external.id
                }
            }
        }
        return false
    }

    private fun recordDevice(
        session: Capture,
        manager: AudioManager,
        input: AudioDeviceInfo?,
        stream: MirrorStream,
        clock: StreamClock,
        inputChanged: () -> Boolean,
    ): CaptureResult {
        val bluetooth = input != null && inputType(input) == AudioRouting.MicInputType.BLUETOOTH
        try {
            if (bluetooth && !session.routeBluetooth(manager, input)) return CaptureResult.STALLED
            if (!session.running) return CaptureResult.ENDED
            val recording = openRecorder(session, input) ?: return CaptureResult.STALLED
            try {
                val recorder = recording.recorder
                val rate = recording.rate
                val samples = ShortArray(rate / 200)
                var filled = 0
                var peak = 0f
                var lastMeter = 0L
                val startedAt = SystemClock.elapsedRealtime()
                var lastRead = startedAt
                if (clock.rate != rate) {
                    if (stream.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(rate.toUInt(), 1u)) == MirrorSendResult.CLOSED) {
                        return CaptureResult.ENDED
                    }
                    clock.rate = rate
                }

                while (session.running) {
                    if (inputChanged()) return CaptureResult.INPUT_CHANGED
                    val count = recorder.read(samples, filled, samples.size - filled, AudioRecord.READ_NON_BLOCKING)
                    val now = SystemClock.elapsedRealtime()
                    if (count > 0) {
                        lastRead = now
                        filled += count
                    }
                    if (now - lastRead >= STALL_MS || count == AudioRecord.ERROR_DEAD_OBJECT) return CaptureResult.STALLED
                    if (bluetooth && now - startedAt >= STALL_MS &&
                        recorder.routedDevice?.let(::inputType) != AudioRouting.MicInputType.BLUETOOTH
                    ) return CaptureResult.STALLED
                    if (filled < samples.size) {
                        if (count <= 0) SystemClock.sleep(4)
                        continue
                    }

                    val mic = AudioRouting.state.value.mic
                    val pcm = ByteArray(samples.size * 2)
                    var squares = 0.0
                    for (i in samples.indices) {
                        val sample = samples[i].toInt()
                        squares += sample.toDouble() * sample
                        if (!mic.muted) {
                            val amplified = (sample * mic.gain).roundToInt().coerceIn(-32768, 32767)
                            pcm[i * 2] = amplified.toByte()
                            pcm[i * 2 + 1] = (amplified shr 8).toByte()
                        }
                    }
                    peak = maxOf(sqrt(squares / samples.size).toFloat() / 32768f, peak * 0.86f)
                    if (now - lastMeter >= METER_MS && session.running) {
                        AudioRouting.updateMicPeak(peak)
                        lastMeter = now
                    }
                    if (!session.running || stream.send(VideoPacketKind.FRAME, clock.timeUs, pcm) == MirrorSendResult.CLOSED) {
                        return CaptureResult.ENDED
                    }
                    clock.timeUs += 5_000u
                    filled = 0
                }
                return CaptureResult.ENDED
            } finally {
                recording.effects.forEach { effect -> runCatching { effect.release() } }
                session.closeRecorder(recording.recorder)
            }
        } finally {
            session.clearCommunicationRoute()
        }
    }

    @SuppressLint("MissingPermission")
    private fun openRecorder(session: Capture, input: AudioDeviceInfo?): Recording? {
        val mask = AudioFormat.CHANNEL_IN_MONO
        val encoding = AudioFormat.ENCODING_PCM_16BIT
        for (rate in MicPcm.RATES) {
            if (!session.running) return null
            val min = AudioRecord.getMinBufferSize(rate, mask, encoding)
            if (min <= 0) continue
            val recorder = runCatching {
                AudioRecord(
                    MediaRecorder.AudioSource.VOICE_COMMUNICATION,
                    rate,
                    mask,
                    encoding,
                    maxOf(min, rate / 200 * 2 * 8),
                )
            }.getOrNull() ?: continue
            if (recorder.state != AudioRecord.STATE_INITIALIZED) {
                recorder.release()
                continue
            }
            input?.let { runCatching { recorder.setPreferredDevice(it) } }
            val id = recorder.audioSessionId
            val effects = listOfNotNull(
                enableEffect { if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(id) else null },
                enableEffect { if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(id) else null },
                enableEffect { if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(id) else null },
            )
            if (session.startRecorder(recorder)) return Recording(rate, recorder, effects)
            effects.forEach { effect -> runCatching { effect.release() } }
            session.closeRecorder(recorder)
        }
        return null
    }

    private fun enableEffect(create: () -> AudioEffect?): AudioEffect? = runCatching {
        create()?.also { effect -> runCatching { effect.enabled = true } }
    }.getOrNull()

    private fun notification(pcId: String): Notification {
        val pcName = core.state.value.nameOf(pcId).orEmpty().ifEmpty { getString(R.string.your_pc) }
        val stop = PendingIntent.getService(
            this, 1, Intent(this, MicService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_FROM_USER, true),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.mic_notification_title, pcName))
            .setContentText(getString(R.string.mic_notification_text))
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    override fun onDestroy() {
        capture?.stop()
        capture = null
        desiredPc = null
        deviceWatch?.close()
        scope.cancel()
        _streaming.value = null
        AudioRouting.updateMicStreaming(null)
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    private enum class CaptureResult { ENDED, INPUT_CHANGED, STALLED }
    private class StreamClock(var rate: Int = 0, var timeUs: ULong = 0u)
    private data class Recording(val rate: Int, val recorder: AudioRecord, val effects: List<AudioEffect>)

    private class Capture(val pcId: String) {
        private val lock = Any()
        @Volatile var running = true
            private set
        private var recorder: AudioRecord? = null
        private var communicationRoute: CommunicationRoute? = null

        fun startRecorder(record: AudioRecord): Boolean = synchronized(lock) {
            if (!running) return false
            recorder = record
            runCatching {
                record.startRecording()
                record.recordingState == AudioRecord.RECORDSTATE_RECORDING
            }.getOrDefault(false)
        }

        fun closeRecorder(record: AudioRecord) = synchronized(lock) {
            if (recorder === record) recorder = null
            runCatching { record.stop() }
            record.release()
        }

        fun routeBluetooth(manager: AudioManager, input: AudioDeviceInfo): Boolean {
            val route = synchronized(lock) {
                if (!running) return false
                val next = CommunicationRoute(manager)
                communicationRoute = next
                if (!next.start(input)) return false
                next
            }
            val until = SystemClock.elapsedRealtime() + 4_000
            while (running && SystemClock.elapsedRealtime() < until) {
                if (route.connected()) return true
                SystemClock.sleep(50)
            }
            return false
        }

        fun clearCommunicationRoute() = synchronized(lock) {
            communicationRoute?.close()
            communicationRoute = null
        }

        fun stop() = synchronized(lock) {
            running = false
            runCatching { recorder?.stop() }
            clearCommunicationRoute()
        }
    }

    @Suppress("DEPRECATION")
    private class CommunicationRoute(private val audio: AudioManager) {
        private val previousMode = audio.mode
        private var ownMode = false
        private var communicationId: Int? = null
        private var startedSco = false

        @SuppressLint("MissingPermission")
        fun start(input: AudioDeviceInfo): Boolean = runCatching {
            if (previousMode != AudioManager.MODE_IN_COMMUNICATION) {
                audio.mode = AudioManager.MODE_IN_COMMUNICATION
                ownMode = true
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                val devices = audio.availableCommunicationDevices.filter {
                    inputType(it) == AudioRouting.MicInputType.BLUETOOTH
                }
                val output = devices.firstOrNull { it.address == input.address } ?: devices.singleOrNull()
                    ?: return false
                if (!audio.setCommunicationDevice(output)) return false
                communicationId = output.id
            } else if (!audio.isBluetoothScoOn) {
                audio.startBluetoothSco()
                startedSco = true
                audio.isBluetoothScoOn = true
            }
            true
        }.getOrDefault(false)

        fun connected(): Boolean = runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                audio.communicationDevice?.id == communicationId
            } else {
                audio.isBluetoothScoOn
            }
        }.getOrDefault(false)

        fun close() {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                if (communicationId != null) runCatching { audio.clearCommunicationDevice() }
            } else if (startedSco) {
                runCatching { audio.isBluetoothScoOn = false }
                runCatching { audio.stopBluetoothSco() }
            }
            if (ownMode) runCatching {
                if (audio.mode == AudioManager.MODE_IN_COMMUNICATION) audio.mode = previousMode
            }
        }
    }

    companion object {
        private const val CHANNEL = "mic"
        private const val NOTIFICATION_ID = 9
        private const val STALL_MS = 3_000L
        private const val METER_MS = 50L
        private const val DEVICE_DEBOUNCE_MS = 1_500L

        private const val ACTION_START = "app.nectarlink.android.mic.START"
        private const val ACTION_STOP = "app.nectarlink.android.mic.STOP"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_FROM_USER = "from_user"
        private const val EXTRA_REPLACE = "replace"

        private val _streaming = MutableStateFlow<String?>(null)
        val streaming: StateFlow<String?> = _streaming.asStateFlow()

        fun hasPermission(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

        fun hasBluetoothPermission(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
                ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

        /** Starts streaming from a foreground activity after microphone permission was granted. */
        fun start(context: Context, pcId: String) {
            if (_streaming.value == null) startForeground(context, pcId, replace = false)
        }

        /** Switches targets from the foreground, keeping at most one recorder alive. */
        fun switchTarget(context: Context, pcId: String): Boolean = startForeground(context, pcId, replace = true)

        private fun startForeground(context: Context, pcId: String, replace: Boolean): Boolean {
            if (!hasPermission(context)) return false
            val intent = Intent(context, MicService::class.java).setAction(ACTION_START)
                .putExtra(EXTRA_PC, pcId).putExtra(EXTRA_REPLACE, replace)
            return runCatching { ContextCompat.startForegroundService(context, intent) }.isSuccess
        }

        const val TOGGLE_NAME = "mic"

        fun stop(context: Context, pcId: String) {
            if (_streaming.value != pcId) return
            context.startService(Intent(context, MicService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_PC, pcId))
        }

        fun stopFromUser(context: Context) {
            if (_streaming.value == null) return
            context.startService(
                Intent(context, MicService::class.java).setAction(ACTION_STOP).putExtra(EXTRA_FROM_USER, true),
            )
        }

        /** Observes available inputs. Call from the UI thread and close with its lifecycle. */
        fun watchInputs(context: Context, onChanged: (List<AudioRouting.MicInputDevice>) -> Unit): AutoCloseable {
            val audio = context.getSystemService(AudioManager::class.java) ?: return AutoCloseable {}
            val main = Handler(Looper.getMainLooper())
            val refresh = Runnable {
                onChanged(inputDevices(audio).mapNotNull { device ->
                    val type = inputType(device) ?: return@mapNotNull null
                    val name = device.productName?.toString()?.takeIf { it.isNotBlank() } ?: when (type) {
                        AudioRouting.MicInputType.BUILTIN -> context.getString(R.string.audio_mixer_builtin_mic)
                        AudioRouting.MicInputType.WIRED_USB -> context.getString(R.string.audio_mixer_wired_usb)
                        AudioRouting.MicInputType.BLUETOOTH -> context.getString(R.string.audio_mixer_bluetooth_mic)
                        AudioRouting.MicInputType.AUTO -> context.getString(R.string.audio_mixer_automatic)
                    }
                    AudioRouting.MicInputDevice(AudioRouting.MicInput(type, device.id), name)
                })
            }
            val callback = object : AudioDeviceCallback() {
                override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = refreshLater()
                override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = refreshLater()
                private fun refreshLater() {
                    main.removeCallbacks(refresh)
                    main.postDelayed(refresh, DEVICE_DEBOUNCE_MS)
                }
            }
            audio.registerAudioDeviceCallback(callback, main)
            refresh.run()
            return AutoCloseable {
                audio.unregisterAudioDeviceCallback(callback)
                main.removeCallbacks(refresh)
            }
        }

        private fun inputDevices(audio: AudioManager): List<AudioDeviceInfo> =
            runCatching { audio.getDevices(AudioManager.GET_DEVICES_INPUTS).filter { inputType(it) != null } }
                .getOrDefault(emptyList())

        private fun preferredDevice(
            devices: List<AudioDeviceInfo>,
            preference: AudioRouting.MicInput,
            failedDevice: Int?,
        ): AudioDeviceInfo? {
            if (preference.type != AudioRouting.MicInputType.AUTO) {
                devices.firstOrNull {
                    it.id != failedDevice && (preference.deviceId == null || it.id == preference.deviceId) &&
                        inputType(it) == preference.type
                }?.let { return it }
            }
            return devices.firstOrNull { inputType(it) == AudioRouting.MicInputType.BUILTIN }
        }

        private fun inputType(device: AudioDeviceInfo): AudioRouting.MicInputType? = when (device.type) {
            AudioDeviceInfo.TYPE_BUILTIN_MIC -> AudioRouting.MicInputType.BUILTIN
            AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_DEVICE,
            AudioDeviceInfo.TYPE_USB_ACCESSORY -> AudioRouting.MicInputType.WIRED_USB
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO, AudioDeviceInfo.TYPE_BLE_HEADSET -> AudioRouting.MicInputType.BLUETOOTH
            else -> null
        }

        private fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_mic), NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }
}

internal object MicPcm {
    val RATES = intArrayOf(48_000, 16_000)

    fun blockBytes(rate: Int): Int = rate / 200 * 2

    fun peak(pcm: ByteArray, length: Int = pcm.size): Float {
        var maxSample = 0
        var i = 0
        val limit = length - 1
        while (i < limit) {
            val sample = ((pcm[i].toInt() and 0xff) or (pcm[i + 1].toInt() shl 8)).toShort().toInt()
            val absVal = if (sample < 0) -sample else sample
            if (absVal > maxSample) maxSample = absVal
            i += 2
        }
        return (maxSample / 32768f).coerceIn(0f, 1f)
    }

    fun applyGain(pcm: ByteArray, length: Int, gain: Float, muted: Boolean): Float {
        if (muted) {
            val preMutePeak = peak(pcm, length)
            pcm.fill(0, 0, length.coerceAtMost(pcm.size))
            return preMutePeak
        }
        var maxSample = 0
        var i = 0
        val limit = length - 1
        while (i < limit) {
            val sample = ((pcm[i].toInt() and 0xff) or (pcm[i + 1].toInt() shl 8)).toShort().toInt()
            val amplified = (sample * gain).roundToInt().coerceIn(-32768, 32767)
            pcm[i] = amplified.toByte()
            pcm[i + 1] = (amplified shr 8).toByte()
            val absVal = if (amplified < 0) -amplified else amplified
            if (absVal > maxSample) maxSample = absVal
            i += 2
        }
        return (maxSample / 32768f).coerceIn(0f, 1f)
    }
}

internal class MicStallWatch(
    private val startupGraceMs: Long = 5_000L,
    private val stallMs: Long = 3_000L,
) {
    enum class Action { NONE, RETRY, FALL_BACK }

    private var stalls = 0
    private var lastSeenReadMs = 0L

    fun check(nowMs: Long, startedAtMs: Long, lastReadMs: Long, builtin: Boolean): Action {
        if (lastReadMs > lastSeenReadMs) {
            lastSeenReadMs = lastReadMs
            stalls = 0
        }
        val stalled = if (lastReadMs <= 0L) {
            nowMs - startedAtMs >= startupGraceMs
        } else {
            nowMs - lastReadMs >= stallMs
        }
        if (!stalled) return Action.NONE
        stalls++
        return if (builtin || stalls <= 1) Action.RETRY else Action.FALL_BACK
    }
}
