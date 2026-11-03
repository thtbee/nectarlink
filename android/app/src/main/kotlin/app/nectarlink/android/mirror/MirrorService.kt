// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.elevated.AppDisplay
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

@androidx.compose.runtime.Immutable
data class MirrorSession(
    val active: Boolean = false,
    val pcId: String? = null,
    val audio: Boolean = false,
    val stayAwake: Boolean = false,
    val screenOff: Boolean = false,
    /** Only the sound is shared, no picture. */
    val soundOnly: Boolean = false,
)

/**
 * Shares the screen with one PC while the user agreed: holds the screen
 * capture (Android wants a foreground service for it, with its own
 * notification and a Stop button) and runs the encoder.
 */
class MirrorService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val main = Handler(Looper.getMainLooper())
    private var projection: MediaProjection? = null
    private var encoder: ScreenEncoder? = null
    @Volatile private var elevatedSocket: java.net.Socket? = null
    /** Opening the helper's screen stream (off the main thread), before [elevatedSocket] is set. */
    @Volatile private var elevatedStarting = false
    private var elevatedDisplayListener: android.hardware.display.DisplayManager.DisplayListener? = null
    private var sound: SoundCapture? = null
    private var pcId: String? = null
    private var stayAwake: Boolean = false
    private var screenOff: Boolean = false
    private var soundOnly: Boolean = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSharing()
            return START_NOT_STICKY
        }
        val pc = intent?.getStringExtra(EXTRA_PC)
        val isElevated = intent?.getBooleanExtra(EXTRA_ELEVATED, false) == true
        val consent = intent?.let { IntentCompat.getParcelableExtra(it, EXTRA_CONSENT, Intent::class.java) }
        if (pc == null || (!isElevated && consent == null) || projection != null || elevatedSocket != null || elevatedStarting) {
            if (projection == null && elevatedSocket == null && !elevatedStarting) {
                createChannel(this)
                val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    if (isElevated) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
                } else {
                    0
                }
                runCatching { ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(""), type) }
                stopSharing()
            }
            return START_NOT_STICKY
        }
        val core = (application as NectarlinkApplication).core
        val name = core.state.value.nameOf(pc).orEmpty()
        createChannel(this)
        val maxSize = intent.getIntExtra(EXTRA_MAX_SIZE, 1920)
        val fps = intent.getIntExtra(EXTRA_FPS, 60)
        val bitrate = intent.getIntExtra(EXTRA_BITRATE, 8_000_000)
        val audio = intent.getBooleanExtra(EXTRA_AUDIO, false)
        stayAwake = intent.getBooleanExtra(EXTRA_STAY_AWAKE, false)
        screenOff = intent.getBooleanExtra(EXTRA_SCREEN_OFF, false)
        soundOnly = intent.getBooleanExtra(EXTRA_SOUND_ONLY, false)

        if (isElevated) {
            val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else 0
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(name), type)
            pcId = pc
            current = this
            _session.value = MirrorSession(
                active = true,
                pcId = pc,
                audio = audio,
                stayAwake = stayAwake,
                screenOff = screenOff,
                soundOnly = soundOnly,
            )
            if (soundOnly) {
                scope.launch { startSound(core, pc) }
                return START_NOT_STICKY
            }
            val fallbackRequest = MirrorRequest(pc, maxSize, fps, bitrate, audio, stayAwake, screenOff)
            elevatedStarting = true
            startElevatedStream(core, pc, name, fallbackRequest)
            return START_NOT_STICKY
        }

        // The screen capture needs the service in the foreground first.
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(name), type)
        val manager = getSystemService(MediaProjectionManager::class.java)
        val granted = runCatching {
            manager.getMediaProjection(intent.getIntExtra(EXTRA_RESULT, 0), consent!!)
        }.getOrNull()
        if (granted == null) {
            Log.w(TAG, "no screen capture")
            stopSharing()
            return START_NOT_STICKY
        }
        projection = granted
        pcId = pc
        current = this
        _session.value = MirrorSession(
            active = true,
            pcId = pc,
            audio = audio,
            stayAwake = stayAwake,
            screenOff = screenOff,
            soundOnly = soundOnly,
        )
        granted.registerCallback(object : MediaProjection.Callback() {
            // The user stopped it (from the status bar, or by locking).
            override fun onStop() = main.post { stopSharing() }.let {}
        }, main)
        if (soundOnly) {
            scope.launch { startSound(core, pc) }
            return START_NOT_STICKY
        }
        scope.launch {
            val stream = runCatching { core.mirrorOpen(pc) }.getOrElse {
                Log.w(TAG, "can't stream to the PC", it)
                main.post { stopSharing() }
                return@launch
            }
            val preferredCodec = core.mirrorCodecs(pc).firstOrNull() ?: "h264"
            main.post {
                val running = projection
                if (running == null) {
                    stream.end()
                    return@post
                }
                encoder = ScreenEncoder(this@MirrorService, running, stream, maxSize, fps, bitrate, preferredCodec) {
                    main.post { stopSharing() }
                }.also { it.start() }
                applyPower()
            }
            if (audio && SoundCapture.canCapture(this@MirrorService)) startSound(core, pc)
        }
        return START_NOT_STICKY
    }

    private fun startElevatedStream(
        core: app.nectarlink.android.core.Core,
        pc: String,
        pcName: String,
        request: MirrorRequest,
    ) = scope.launch {
        // The helper's socket: off the main thread, where Android refuses
        // network calls (and the phone then asked for screen-capture consent).
        val (initW, initH, initDpi) = ScreenEncoder.computeScreenSize(this@MirrorService, request.maxSize)
        // With the screen off (phone in a pocket) there's nothing to show yet.
        app.nectarlink.android.elevated.Elevated.wakeScreen()
        // Straight off when it's meant to stay off, so it barely lights up.
        if (request.screenOff) app.nectarlink.android.elevated.Elevated.setScreenOff(true)
        val preferredCodec = core.mirrorCodecs(pc).firstOrNull() ?: "h264"
        val codec = ScreenEncoder.selectCodec(preferredCodec)
        val socket = app.nectarlink.android.elevated.Elevated.openScreen(
            initW,
            initH,
            initDpi,
            request.bitrate,
            request.fps,
            codec,
        )
        elevatedStarting = false
        if (!elevatedStarting && (pcId != pc || current !== this@MirrorService)) {
            runCatching { socket?.close() }
            return@launch
        }
        if (socket == null) {
            if (request.screenOff) app.nectarlink.android.elevated.Elevated.setScreenOff(false)
            main.post {
                if (pcId == pc && current === this@MirrorService) {
                    stopSharing()
                    MirrorRequests.show(this@MirrorService, request, pcName)
                }
            }
            return@launch
        }
        if (pcId != pc || current !== this@MirrorService) {
            runCatching { socket.close() }
            return@launch
        }
        elevatedSocket = socket
        main.post { watchDisplay(socket, request, initW, initH) }
        streamElevated(core, pc, pcName, request, socket)
    }

    /** Resizes the helper's picture when the phone's screen turns. */
    private fun watchDisplay(socket: java.net.Socket, request: MirrorRequest, initW: Int, initH: Int) {
        if (elevatedSocket !== socket) return
        var curW = initW
        var curH = initH
        val displays = getSystemService(android.hardware.display.DisplayManager::class.java)
        val listener = object : android.hardware.display.DisplayManager.DisplayListener {
            override fun onDisplayChanged(displayId: Int) {
                if (displayId != android.view.Display.DEFAULT_DISPLAY) return
                val (nw, nh, ndpi) = ScreenEncoder.computeScreenSize(this@MirrorService, request.maxSize)
                if (nw != curW || nh != curH) {
                    curW = nw
                    curH = nh
                    val sock = elevatedSocket ?: return
                    Thread {
                        runCatching {
                            synchronized(sock) {
                                sock.getOutputStream().apply {
                                    write("R $nw $nh $ndpi\n".toByteArray())
                                    flush()
                                }
                            }
                        }
                    }.start()
                }
            }
            override fun onDisplayAdded(displayId: Int) {}
            override fun onDisplayRemoved(displayId: Int) {}
        }
        elevatedDisplayListener = listener
        displays?.registerDisplayListener(listener, main)
        applyPower()
    }

    /** The helper's packets to the PC, until either side ends; asks for consent if none came. */
    private suspend fun streamElevated(
        core: app.nectarlink.android.core.Core,
        pc: String,
        pcName: String,
        request: MirrorRequest,
        socket: java.net.Socket,
    ) {
        val stream = runCatching { core.mirrorOpen(pc) }.getOrElse {
            Log.w(TAG, "can't stream to the PC", it)
            main.post { stopSharing() }
            return
        }
        if (request.audio && SoundCapture.canCapture(this@MirrorService)) startSound(core, pc)
        var gotConfig = false
        try {
            val input = java.io.DataInputStream(socket.getInputStream().buffered(1 shl 16))
            while (elevatedSocket === socket) {
                val length = input.readInt()
                if (length < 9 || length > 16 * 1024 * 1024) break
                val kind = input.readUnsignedByte()
                val time = input.readLong().toULong()
                val data = ByteArray(length - 9).also { input.readFully(it) }
                val result = when (kind) {
                    AppDisplay.KIND_CONFIG -> {
                        if (data.size != 16) throw java.io.IOException("Invalid screen configuration length")
                        val fields = java.nio.ByteBuffer.wrap(data)
                        val (w, h) = fields.int to fields.int
                        fields.int // display id
                        val activeCodec = when (fields.int) {
                            AppDisplay.CODEC_ID_H264 -> "h264"
                            AppDisplay.CODEC_ID_H265 -> "h265"
                            AppDisplay.CODEC_ID_AV1 -> "av1"
                            else -> throw java.io.IOException("Unsupported screen codec")
                        }
                        gotConfig = true
                        stream.send(
                            app.nectarlink.core.VideoPacketKind.CONFIG,
                            0u,
                            app.nectarlink.core.mirrorConfigCodec(activeCodec, w.toUInt(), h.toUInt(), 0u),
                        )
                    }
                    AppDisplay.KIND_KEYFRAME ->
                        stream.send(app.nectarlink.core.VideoPacketKind.KEYFRAME, time, data)
                    else ->
                        stream.send(app.nectarlink.core.VideoPacketKind.FRAME, time, data)
                }
                when (result) {
                    app.nectarlink.core.MirrorSendResult.CLOSED -> break
                    app.nectarlink.core.MirrorSendResult.NEED_KEYFRAME,
                    app.nectarlink.core.MirrorSendResult.DROPPED -> requestElevatedKeyframe()
                    app.nectarlink.core.MirrorSendResult.QUEUED -> {}
                }
            }
        } catch (_: java.io.EOFException) {
        } catch (e: java.io.IOException) {
            Log.d(TAG, "Elevated screen mirror ended", e)
        } finally {
            stream.end()
            val shouldFallback = !gotConfig && pcId == pc
            main.post {
                stopSharing()
                if (shouldFallback) {
                    MirrorRequests.show(this@MirrorService, request, pcName)
                }
            }
        }
    }

    private fun requestElevatedKeyframe() {
        val sock = elevatedSocket ?: return
        runCatching {
            synchronized(sock) {
                sock.getOutputStream().apply {
                    write("K\n".toByteArray())
                    flush()
                }
            }
        }
    }

    private fun applyPower() {
        val active = streaming(
            screen = encoder != null && projection != null,
            elevatedScreen = elevatedSocket != null,
            sound = sound != null,
        )
        StreamPower.set(this, StreamPower.SCREEN, active, stayAwake, screenOff)
    }

    /** The sound, on a stream of its own; the screen goes on without it if it can't. */
    private suspend fun startSound(core: app.nectarlink.android.core.Core, pc: String) {
        // Sharing only the sound, there's nothing left once it's gone.
        val endSharing = { if (soundOnly) main.post { if (pcId == pc) stopSharing() } }
        if (!SoundCapture.canCapture(this)) {
            endSharing()
            return
        }
        val stream = runCatching { core.mirrorOpenAudio(pc) }.getOrElse {
            Log.w(TAG, "can't send the sound", it)
            endSharing()
            return
        }
        main.post {
            val running = projection
            if ((running == null && !app.nectarlink.android.elevated.Elevated.running) || pcId != pc) {
                stream.end()
                endSharing()
                return@post
            }
            val capture = SoundCapture(running, stream) {
                // Some phones refuse the helper's capture. Screen-capture consent
                // still works. Not when this was stopped on purpose: `sound` is gone then.
                val current = sound
                if (soundOnly && running == null && current != null && !current.sentSound) askForSound(pc)
                endSharing()
            }
            sound = capture
            capture.start()
            // Sound alone also needs the phone awake and its Wi-Fi out of power save (#94).
            applyPower()
            // Only the sound, so it's heard on the PC instead of the phone (#94).
            if (soundOnly && sound?.usesPlaybackCapture == true) PhoneQuiet.start(this)
        }
    }

    private fun askForSound(pc: String) {
        val request = MirrorRequest(pc, 1280, 30, 4_000_000, audio = true, soundOnly = true)
        MirrorRequests.show(this, request, "")
    }

    private fun notification(pcName: String): Notification {
        val stop = PendingIntent.getService(
            this, 0, Intent(this, MirrorService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(
                getString(
                    if (soundOnly) R.string.mirror_sharing_sound else R.string.mirror_sharing,
                    pcName.ifEmpty { getString(R.string.your_pc) },
                ),
            )
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    private fun stopSharing() {
        stayAwake = false
        screenOff = false
        soundOnly = false
        elevatedStarting = false
        _session.value = MirrorSession()
        StreamPower.set(this, StreamPower.SCREEN, streaming = false)
        elevatedDisplayListener?.let {
            getSystemService(android.hardware.display.DisplayManager::class.java)?.unregisterDisplayListener(it)
        }
        elevatedDisplayListener = null
        elevatedSocket?.let { runCatching { it.close() } }
        elevatedSocket = null
        encoder?.stop()
        encoder = null
        sound?.stop()
        sound = null
        PhoneQuiet.end(this)
        projection?.let { runCatching { it.stop() } }
        projection = null
        pcId = null
        if (current === this) current = null
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        stopSharing()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "MirrorService"
        internal const val CHANNEL = "mirroring"
        private const val NOTIFICATION_ID = 4
        private const val ACTION_STOP = "app.nectarlink.android.mirror.STOP"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_ELEVATED = "elevated"
        private const val EXTRA_CONSENT = "consent"
        private const val EXTRA_RESULT = "result"
        private const val EXTRA_MAX_SIZE = "maxSize"
        private const val EXTRA_FPS = "fps"
        private const val EXTRA_BITRATE = "bitrate"
        private const val EXTRA_AUDIO = "audio"
        private const val EXTRA_STAY_AWAKE = "stayAwake"
        private const val EXTRA_SCREEN_OFF = "screenOff"
        private const val EXTRA_SOUND_ONLY = "soundOnly"

        /** The service sharing the screen now, if any. */
        @Volatile private var current: MirrorService? = null

        private val _session = MutableStateFlow(MirrorSession())
        val session: StateFlow<MirrorSession> = _session.asStateFlow()

        fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_mirroring), NotificationManager.IMPORTANCE_LOW),
            )
        }

        /** Starts sharing with `request`'s PC, with the user's consent (from the capture prompt). */
        fun start(context: Context, request: MirrorRequest, result: Int, consent: Intent) {
            if (current != null) return
            val intent = Intent(context, MirrorService::class.java)
                .putExtra(EXTRA_PC, request.pcId)
                .putExtra(EXTRA_CONSENT, consent)
                .putExtra(EXTRA_RESULT, result)
                .putExtra(EXTRA_MAX_SIZE, request.maxSize)
                .putExtra(EXTRA_FPS, request.fps)
                .putExtra(EXTRA_BITRATE, request.bitrate)
                .putExtra(EXTRA_AUDIO, request.audio)
                .putExtra(EXTRA_STAY_AWAKE, request.stayAwake)
                .putExtra(EXTRA_SCREEN_OFF, request.screenOff)
                .putExtra(EXTRA_SOUND_ONLY, request.soundOnly)
            ContextCompat.startForegroundService(context, intent)
        }

        /**
         * Starts full-screen sharing with `request`'s PC immediately through
         * the Elevated shell helper, without a `MediaProjection` consent prompt.
         */
        fun startElevated(context: Context, request: MirrorRequest): Boolean {
            if (!app.nectarlink.android.elevated.Elevated.running || current != null) return false
            val intent = Intent(context, MirrorService::class.java)
                .putExtra(EXTRA_PC, request.pcId)
                .putExtra(EXTRA_ELEVATED, true)
                .putExtra(EXTRA_MAX_SIZE, request.maxSize)
                .putExtra(EXTRA_FPS, request.fps)
                .putExtra(EXTRA_BITRATE, request.bitrate)
                .putExtra(EXTRA_AUDIO, request.audio)
                .putExtra(EXTRA_STAY_AWAKE, request.stayAwake)
                .putExtra(EXTRA_SCREEN_OFF, request.screenOff)
                .putExtra(EXTRA_SOUND_ONLY, request.soundOnly)
            return runCatching {
                ContextCompat.startForegroundService(context, intent)
                true
            }.getOrDefault(false)
        }

        /** The PC stopped watching. */
        fun stop(pcId: String) {
            val service = current ?: return
            service.main.post { if (service.pcId == pcId) service.stopSharing() }
        }

        /** Stops any active screen mirroring session from the UI. */
        fun stopCurrent() {
            val service = current ?: return
            service.main.post { service.stopSharing() }
        }

        /** The PC needs a keyframe. */
        fun keyframe(pcId: String) {
            val service = current?.takeIf { it.pcId == pcId } ?: return
            service.encoder?.requestKeyframe()
            service.requestElevatedKeyframe()
        }

        /** Updates keep-awake and screen-off state while mirroring to [pcId]. */
        /** Sends the sound to the PC, or stops so the phone plays it again (#178). */
        fun setSound(pcId: String, on: Boolean) {
            val service = current ?: return
            service.main.post {
                if (service.pcId != pcId || service.soundOnly) return@post
                if (on) {
                    if (service.sound == null) {
                        val core = (service.application as NectarlinkApplication).core
                        service.scope.launch { service.startSound(core, pcId) }
                    }
                } else {
                    service.sound?.stop()
                    service.sound = null
                    service.applyPower()
                }
            }
        }

        /**
         * Whether the phone is sending the PC anything, so it has to stay on the
         * network: the screen (captured, or through Elevated) or only its sound.
         */
        internal fun streaming(screen: Boolean, elevatedScreen: Boolean, sound: Boolean): Boolean =
            screen || elevatedScreen || sound

        fun setPower(pcId: String, stayAwake: Boolean, screenOff: Boolean) {
            val service = current ?: return
            service.main.post {
                if (service.pcId == pcId) {
                    service.stayAwake = stayAwake
                    service.screenOff = screenOff
                    _session.value = _session.value.copy(
                        stayAwake = stayAwake,
                        screenOff = screenOff,
                    )
                    service.applyPower()
                }
            }
        }
    }
}
