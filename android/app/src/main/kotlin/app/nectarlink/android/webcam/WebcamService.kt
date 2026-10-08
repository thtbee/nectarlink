// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.camera.core.Preview
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class WebcamSession(
    val active: Boolean = false,
    val connecting: Boolean = false,
    val pcId: String? = null,
    val width: Int = 1280,
    val height: Int = 720,
    val fps: Int = 30,
    val camera: String = "back",
    val zoomRatio: Float = 1f,
    val minZoomRatio: Float = 1f,
    val maxZoomRatio: Float = 4f,
    val hasFlash: Boolean = false,
    val torchOn: Boolean = false,
    val supports4k: Boolean = CameraEncoder.supports4kEncoder(),
)

/**
 * Streams the phone's camera as H.264 to a paired PC (`docs/protocol/webcam.md`).
 * Runs as a `camera` foreground service with an ongoing notification and a
 * Stop action so streaming continues if the user switches apps or locks the
 * screen.
 */
class WebcamService : LifecycleService() {
    private val main = Handler(Looper.getMainLooper())
    private var encoder: CameraEncoder? = null
    private var pcId: String? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_STOP -> {
                stopStreaming()
                return START_NOT_STICKY
            }
            ACTION_START -> {
                val targetPc = intent.getStringExtra(EXTRA_PC)
                if (targetPc == null || !hasPermission(this)) {
                    stopStreaming()
                    return START_NOT_STICKY
                }
                val height = intent.getIntExtra(EXTRA_HEIGHT, _session.value.height)
                val fps = intent.getIntExtra(EXTRA_FPS, 30).coerceIn(15, 60)
                val camera = intent.getStringExtra(EXTRA_CAMERA) ?: _session.value.camera
                if (encoder != null && pcId == targetPc) {
                    encoder?.setCamera(camera)
                    encoder?.setResolutionHeight(height)
                    return START_NOT_STICKY
                }
                startStreamingSession(targetPc, height, fps, camera)
            }
        }
        return START_NOT_STICKY
    }

    private fun startStreamingSession(targetPc: String, height: Int, fps: Int, camera: String) {
        encoder?.stop()
        encoder = null
        pcId = targetPc
        current = this
        WebcamRequests.dismiss(this, targetPc)

        val supports4k = CameraEncoder.supports4kEncoder()
        val normHeight = CameraEncoder.normalizeHeight(height, supports4k)
        val normWidth = CameraEncoder.widthForHeight(normHeight)
        val normCamera = if (camera == "front") "front" else "back"
        val hasFlash = CameraEncoder.cameraHasFlash(this, normCamera)
        val maxZoom = CameraEncoder.cameraMaxZoom(this, normCamera)
        val initialZoom = _session.value.zoomRatio.coerceIn(1f, maxZoom)

        _session.value = WebcamSession(
            active = true,
            connecting = true,
            pcId = targetPc,
            width = normWidth,
            height = normHeight,
            fps = fps,
            camera = normCamera,
            zoomRatio = initialZoom,
            maxZoomRatio = maxZoom,
            hasFlash = hasFlash,
            supports4k = supports4k,
        )

        createChannel(this)
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        } else {
            0
        }
        val startedFg = runCatching {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                buildNotification(targetPc, normWidth, normHeight, normCamera),
                type,
            )
        }.onFailure { e ->
            Log.w(TAG, "can't enter camera foreground service", e)
        }.isSuccess
        if (!startedFg) {
            stopStreaming()
            return
        }

        val core = (application as NectarlinkApplication).core
        core.clearWebcamRequest(targetPc)

        lifecycleScope.launch(Dispatchers.IO) {
            val stream = core.webcamOpenOrMessage(targetPc)
            if (stream == null) {
                main.post { stopStreaming() }
                return@launch
            }
            main.post {
                if (pcId != targetPc || current !== this@WebcamService) {
                    runCatching { stream.end() }
                    return@post
                }
                val camEncoder = CameraEncoder(
                    context = this@WebcamService,
                    lifecycleOwner = this@WebcamService,
                    stream = stream,
                    initialHeight = normHeight,
                    fps = fps,
                    initialCamera = normCamera,
                    initialZoomRatio = _session.value.zoomRatio,
                    onStateChanged = { live ->
                        _session.update { s ->
                            s.copy(
                                active = true,
                                connecting = false,
                                width = live.width,
                                height = live.height,
                                fps = live.fps,
                                camera = live.camera,
                                zoomRatio = live.zoomRatio,
                                minZoomRatio = live.minZoomRatio,
                                maxZoomRatio = live.maxZoomRatio,
                                hasFlash = live.hasFlash,
                                torchOn = live.torchOn,
                                supports4k = live.supports4k,
                            )
                        }
                        updateNotification()
                    },
                    onEnded = {
                        if (current === this@WebcamService) {
                            stopStreaming()
                        }
                    },
                )
                camEncoder.setUiSurfaceProvider(uiSurfaceProvider)
                encoder = camEncoder
                camEncoder.start()
            }
        }
    }

    private fun updateNotification() {
        val s = _session.value
        val targetPc = s.pcId ?: return
        val nm = getSystemService(NotificationManager::class.java) ?: return
        nm.notify(NOTIFICATION_ID, buildNotification(targetPc, s.width, s.height, s.camera))
    }

    private fun buildNotification(targetPc: String, width: Int, height: Int, camera: String): Notification {
        val core = (application as NectarlinkApplication).core
        val pcName = core.state.value.nameOf(targetPc).orEmpty().ifEmpty { getString(R.string.your_pc) }

        val openIntent = Intent(this, MainActivity::class.java).apply {
            putExtra("webcam_pc", targetPc)
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val openPending = PendingIntent.getActivity(
            this,
            10,
            openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stopPending = PendingIntent.getService(
            this,
            11,
            Intent(this, WebcamService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val camLabel = getString(
            if (camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
        ).lowercase()

        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.webcam_notification_streaming, pcName))
            .setContentText(getString(R.string.webcam_notification_detail, width, height, camLabel))
            .setContentIntent(openPending)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stopPending)
            .build()
    }

    private fun stopStreaming() {
        val runningEncoder = encoder
        encoder = null
        pcId = null
        if (current === this) current = null
        runningEncoder?.stop()
        _session.update {
            it.copy(
                active = false,
                connecting = false,
                pcId = null,
                torchOn = false,
            )
        }
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        stopStreaming()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "WebcamService"
        private const val CHANNEL = "webcam"
        private const val NOTIFICATION_ID = 8

        private const val ACTION_START = "app.nectarlink.android.webcam.START"
        private const val ACTION_STOP = "app.nectarlink.android.webcam.STOP"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_HEIGHT = "height"
        private const val EXTRA_FPS = "fps"
        private const val EXTRA_CAMERA = "camera"

        @Volatile private var current: WebcamService? = null
        @Volatile private var uiSurfaceProvider: Preview.SurfaceProvider? = null

        private val _session = MutableStateFlow(WebcamSession())
        val session: StateFlow<WebcamSession> = _session.asStateFlow()

        fun hasPermission(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED &&
                context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)

        fun start(
            context: Context,
            pcId: String,
            height: Int = _session.value.height,
            fps: Int = 30,
            camera: String = _session.value.camera,
        ) {
            val intent = Intent(context, WebcamService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_PC, pcId)
                .putExtra(EXTRA_HEIGHT, height)
                .putExtra(EXTRA_FPS, fps)
                .putExtra(EXTRA_CAMERA, camera)
            ContextCompat.startForegroundService(context, intent)
        }

        fun stop(context: Context) {
            val srv = current
            if (srv != null) {
                srv.main.post { srv.stopStreaming() }
            } else {
                runCatching { context.stopService(Intent(context, WebcamService::class.java)) }
            }
        }

        fun stopForPc(context: Context, pcId: String) {
            val srv = current ?: return
            srv.main.post {
                if (srv.pcId == pcId) srv.stopStreaming()
            }
        }

        fun keyframe(pcId: String) {
            current?.takeIf { it.pcId == pcId }?.encoder?.requestKeyframe()
        }

        fun refreshCapabilities(context: Context) {
            val cam = _session.value.camera
            val flash = CameraEncoder.cameraHasFlash(context, cam)
            val maxZoom = CameraEncoder.cameraMaxZoom(context, cam)
            _session.update {
                it.copy(
                    hasFlash = flash,
                    maxZoomRatio = maxZoom,
                    zoomRatio = it.zoomRatio.coerceIn(1f, maxZoom),
                )
            }
        }

        fun setCamera(camera: String, context: Context? = null) {
            val norm = if (camera == "front") "front" else "back"
            val ctx = context ?: current
            val flash = ctx?.let { CameraEncoder.cameraHasFlash(it, norm) } ?: false
            val maxZoom = ctx?.let { CameraEncoder.cameraMaxZoom(it, norm) } ?: 4f
            _session.update {
                it.copy(
                    camera = norm,
                    hasFlash = flash,
                    maxZoomRatio = maxZoom,
                    zoomRatio = it.zoomRatio.coerceIn(1f, maxZoom),
                    torchOn = false,
                )
            }
            current?.encoder?.setCamera(norm)
        }

        fun setResolutionHeight(height: Int) {
            val norm = CameraEncoder.normalizeHeight(height, _session.value.supports4k)
            _session.update {
                it.copy(width = CameraEncoder.widthForHeight(norm), height = norm)
            }
            current?.encoder?.setResolutionHeight(norm)
        }

        fun setZoomRatio(ratio: Float) {
            _session.update { it.copy(zoomRatio = ratio) }
            current?.encoder?.setZoomRatio(ratio)
        }

        fun setTorch(on: Boolean) {
            _session.update { it.copy(torchOn = on) }
            current?.encoder?.setTorch(on)
        }

        fun attachUiPreview(provider: Preview.SurfaceProvider?) {
            uiSurfaceProvider = provider
            current?.encoder?.setUiSurfaceProvider(provider)
        }

        fun createChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java) ?: return
            val channel = NotificationChannel(
                CHANNEL,
                context.getString(R.string.channel_webcam),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                setShowBadge(false)
            }
            nm.createNotificationChannel(channel)
        }
    }
}
