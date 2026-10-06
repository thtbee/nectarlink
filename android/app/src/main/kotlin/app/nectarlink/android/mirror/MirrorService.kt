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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

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
    private var sound: SoundCapture? = null
    private var pcId: String? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSharing()
            return START_NOT_STICKY
        }
        val pc = intent?.getStringExtra(EXTRA_PC)
        val consent = intent?.let { IntentCompat.getParcelableExtra(it, EXTRA_CONSENT, Intent::class.java) }
        if (pc == null || consent == null || projection != null) {
            if (projection == null) stopSelf()
            return START_NOT_STICKY
        }
        val core = (application as NectarlinkApplication).core
        val name = core.state.value.nameOf(pc).orEmpty()
        createChannel(this)
        // The screen capture needs the service in the foreground first.
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(name), type)
        val manager = getSystemService(MediaProjectionManager::class.java)
        val granted = runCatching {
            manager.getMediaProjection(intent.getIntExtra(EXTRA_RESULT, 0), consent)
        }.getOrNull()
        if (granted == null) {
            Log.w(TAG, "no screen capture")
            stopSharing()
            return START_NOT_STICKY
        }
        projection = granted
        pcId = pc
        current = this
        granted.registerCallback(object : MediaProjection.Callback() {
            // The user stopped it (from the status bar, or by locking).
            override fun onStop() = main.post { stopSharing() }.let {}
        }, main)
        val maxSize = intent.getIntExtra(EXTRA_MAX_SIZE, 1920)
        val fps = intent.getIntExtra(EXTRA_FPS, 60)
        val bitrate = intent.getIntExtra(EXTRA_BITRATE, 8_000_000)
        val audio = intent.getBooleanExtra(EXTRA_AUDIO, false)
        scope.launch {
            val stream = runCatching { core.mirrorOpen(pc) }.getOrElse {
                Log.w(TAG, "can't stream to the PC", it)
                main.post { stopSharing() }
                return@launch
            }
            main.post {
                val running = projection
                if (running == null) {
                    stream.end()
                    return@post
                }
                encoder = ScreenEncoder(this@MirrorService, running, stream, maxSize, fps, bitrate) {
                    main.post { stopSharing() }
                }.also { it.start() }
            }
            if (audio && SoundCapture.canCapture(this@MirrorService)) startSound(core, pc)
        }
        return START_NOT_STICKY
    }

    /** The sound, on a stream of its own; the screen goes on without it if it can't. */
    private suspend fun startSound(core: app.nectarlink.android.core.Core, pc: String) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return
        val stream = runCatching { core.mirrorOpenAudio(pc) }.getOrElse {
            Log.w(TAG, "can't send the sound", it)
            return
        }
        main.post {
            val running = projection
            if (running == null || pcId != pc || Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
                stream.end()
                return@post
            }
            sound = SoundCapture(running, stream).also { it.start() }
        }
    }

    private fun notification(pcName: String): Notification {
        val stop = PendingIntent.getService(
            this, 0, Intent(this, MirrorService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.mirror_sharing, pcName.ifEmpty { getString(R.string.your_pc) }))
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    private fun stopSharing() {
        encoder?.stop()
        encoder = null
        sound?.stop()
        sound = null
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
        private const val CHANNEL = "mirroring"
        private const val NOTIFICATION_ID = 4
        private const val ACTION_STOP = "app.nectarlink.android.mirror.STOP"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_CONSENT = "consent"
        private const val EXTRA_RESULT = "result"
        private const val EXTRA_MAX_SIZE = "maxSize"
        private const val EXTRA_FPS = "fps"
        private const val EXTRA_BITRATE = "bitrate"
        private const val EXTRA_AUDIO = "audio"

        /** The service sharing the screen now, if any. */
        @Volatile private var current: MirrorService? = null

        fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_mirroring), NotificationManager.IMPORTANCE_LOW),
            )
        }

        /** Starts sharing with `request`'s PC, with the user's consent (from the capture prompt). */
        fun start(context: Context, request: MirrorRequest, result: Int, consent: Intent) {
            val intent = Intent(context, MirrorService::class.java)
                .putExtra(EXTRA_PC, request.pcId)
                .putExtra(EXTRA_CONSENT, consent)
                .putExtra(EXTRA_RESULT, result)
                .putExtra(EXTRA_MAX_SIZE, request.maxSize)
                .putExtra(EXTRA_FPS, request.fps)
                .putExtra(EXTRA_BITRATE, request.bitrate)
                .putExtra(EXTRA_AUDIO, request.audio)
            ContextCompat.startForegroundService(context, intent)
        }

        /** The PC stopped watching. */
        fun stop(pcId: String) {
            val service = current ?: return
            service.main.post { if (service.pcId == pcId) service.stopSharing() }
        }

        /** The PC needs a keyframe. */
        fun keyframe(pcId: String) {
            current?.takeIf { it.pcId == pcId }?.encoder?.requestKeyframe()
        }
    }
}
