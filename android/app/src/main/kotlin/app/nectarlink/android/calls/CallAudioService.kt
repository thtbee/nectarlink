// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.elevated.CallAudio
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.ElevatedCallAudioStream
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorAudioConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * The call in progress's sound on a PC (`call.audio`, docs/protocol/calls.md §2.6):
 * the Elevated helper's [CallAudio] connection carries the other party to the
 * PC and the PC's microphone into the call. Runs in the foreground with an
 * ongoing notification saying where the sound went, whose Back to phone
 * button gives it back. Ends with the call, when the PC stops, or on that tap.
 */
class CallAudioService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val pcId = intent?.getStringExtra(EXTRA_PC)
        when (intent?.action) {
            ACTION_START -> {
                // Started with startForegroundService: in the foreground before anything else.
                createChannel(this)
                val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                } else {
                    0
                }
                runCatching { ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(pcId.orEmpty()), type) }
                val socket = synchronized(lock) { active?.takeIf { it.pcId == pcId && !it.started } }
                if (socket == null) stopSelf() else carry(socket.also { it.started = true })
            }
            ACTION_BACK -> {
                // The phone's user took the sound back: the PC hears it.
                val current = synchronized(lock) { active }
                end(current)
                if (current != null) core().callAudioStop(current.pcId)
            }
            else -> if (synchronized(lock) { active } == null) stopSelf()
        }
        return START_NOT_STICKY
    }

    private fun carry(call: Active) {
        scope.launch {
            try {
                val stream = core().callAudioOpen(call.pcId) ?: run {
                    core().callAudioStop(call.pcId)
                    return@launch
                }
                try {
                    if (stream.send(VideoPacketKind.CONFIG, 0u, mirrorAudioConfig(CallAudio.RATE.toUInt(), 1u)) ==
                        MirrorSendResult.CLOSED
                    ) {
                        return@launch
                    }
                    // The PC plays the call now; the phone's copy goes quiet (#133).
                    CallEarpiece.quiet(this@CallAudioService)
                    val input = call.socket.inputStream
                    val chunk = ByteArray(CallAudio.CHUNK_BYTES)
                    var samples = 0L
                    while (true) {
                        val read = input.read(chunk)
                        if (read <= 0) break
                        val timeUs = samples * 1_000_000L / CallAudio.RATE
                        samples += read / 2
                        if (stream.send(VideoPacketKind.FRAME, timeUs.toULong(), chunk.copyOf(read)) ==
                            MirrorSendResult.CLOSED
                        ) {
                            break
                        }
                    }
                } finally {
                    stream.end()
                    stream.close()
                }
                // The call ended (or the helper went): the PC stops too.
                if (synchronized(lock) { active === call }) core().callAudioStop(call.pcId)
            } catch (e: Exception) {
                Log.w(TAG, "call sound failed", e)
                core().callAudioStop(call.pcId)
            } finally {
                end(call)
            }
        }
    }

    /** Closes [call]'s connection (the helper gives the sound back) and, if it's the last, stops. */
    private fun end(call: Active?) {
        val last = synchronized(lock) {
            if (call != null && active === call) active = null
            active == null
        }
        call?.let { runCatching { it.socket.close() } }
        if (last) {
            CallEarpiece.end(this)
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun core() = (application as NectarlinkApplication).core

    private fun notification(pcId: String): Notification {
        val pcName = core().state.value.nameOf(pcId).orEmpty().ifEmpty { getString(R.string.your_pc) }
        val back = PendingIntent.getService(
            this,
            1,
            Intent(this, CallAudioService::class.java).setAction(ACTION_BACK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.call_audio_notification_title, pcName))
            .setContentText(getString(R.string.call_audio_notification_text))
            .setOngoing(true)
            // Pops up once (the channel makes no sound), then stays while the sound is on the PC.
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .addAction(0, getString(R.string.call_audio_back), back)
            .build()
    }

    override fun onDestroy() {
        end(synchronized(lock) { active })
        scope.cancel()
        super.onDestroy()
    }

    /** A call's sound carried to [pcId] over the helper's [socket]. */
    private class Active(val pcId: String, val socket: ElevatedCallAudioStream) {
        @Volatile var started = false
    }

    companion object {
        private const val TAG = "CallAudioService"
        /** High importance without sound or vibration, so it pops up quietly. */
        private const val CHANNEL = "call_audio_on_pc"
        private const val NOTIFICATION_ID = 21
        private const val ACTION_START = "app.nectarlink.android.calls.CALL_AUDIO_START"
        private const val ACTION_BACK = "app.nectarlink.android.calls.CALL_AUDIO_BACK"
        private const val EXTRA_PC = "pc"

        private val lock = Any()
        private var active: Active? = null

        /**
         * A PC asked for the call's sound: opens the helper's side first, so a
         * phone that can't reach it says no, then starts carrying it. Blocks
         * briefly (called off the main thread).
         */
        fun start(context: Context, pcId: String): Boolean {
            val socket = Elevated.openCallAudio() ?: return false
            val previous = synchronized(lock) { active.also { active = Active(pcId, socket) } }
            previous?.let { runCatching { it.socket.close() } }
            val intent = Intent(context, CallAudioService::class.java).setAction(ACTION_START).putExtra(EXTRA_PC, pcId)
            return runCatching { ContextCompat.startForegroundService(context, intent) }
                .onFailure { Log.w(TAG, "can't start the call sound service", it) }
                .isSuccess
        }

        /** [pcId] stopped taking the call's sound: the phone has it again. */
        fun stop(pcId: String) {
            val call = synchronized(lock) { active?.takeIf { it.pcId == pcId }?.also { active = null } }
            call?.let { runCatching { it.socket.close() } }
        }

        /**
         * The PC's microphone, for the call. Telecom's mute silences only the phone's own
         * microphone, not what's played into the call, so a muted call drops it here.
         */
        fun uplink(pcId: String, pcm: ByteArray) {
            if (CallCompanion.controls()?.muted == true) return
            val call = synchronized(lock) { active?.takeIf { it.pcId == pcId } } ?: return
            runCatching {
                val out = call.socket.outputStream
                synchronized(call) { out.write(pcm) }
            }
        }

        private fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_call_audio), NotificationManager.IMPORTANCE_HIGH)
                    .apply {
                        setShowBadge(false)
                        setSound(null, null)
                        enableVibration(false)
                    },
            )
        }
    }
}
