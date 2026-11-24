// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import app.nectarlink.android.NectarlinkApplication
import kotlin.math.roundToInt

/**
 * The phone's volume keys while its sound plays on a PC on its own (#577).
 *
 * A remote-playback media session, the way a cast app does it: while it's the
 * active one, Android sends the volume keys and its volume panel to it, and
 * each step sets how loud the PC plays the phone's sound (`mirror.volume`).
 * The PC's slider moves it back ([fromPc]).
 *
 * Another app's session can take the keys back (one that starts playing after
 * this). Then they turn the phone's own media volume up from the 0 that
 * [PhoneQuiet] set: that counts as a step up on the PC, the phone goes quiet
 * again and this session takes the keys back.
 */
object PcSoundVolume {
    private const val TAG = "PcSoundVolume"
    private const val PREFS = "pc_sound_volume"
    /** Steps in the phone's volume panel, 5% each. */
    private const val STEPS = 20

    private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    private const val EXTRA_STREAM = "android.media.EXTRA_VOLUME_STREAM_TYPE"
    private const val EXTRA_VALUE = "android.media.EXTRA_VOLUME_STREAM_VALUE"

    private val main = Handler(Looper.getMainLooper())
    private var session: MediaSession? = null
    private var provider: VolumeProvider? = null
    private var receiver: BroadcastReceiver? = null
    private var pcId: String? = null
    private var context: Context? = null

    /** The phone's sound started playing on [pc]: its volume keys go there. */
    fun start(context: Context, pc: String) {
        val app = context.applicationContext
        main.post { startNow(app, pc) }
    }

    /** Its sound is back on the phone. */
    fun stop() {
        main.post { stopNow() }
    }

    /** [pc] set how loud it plays the phone's sound (its slider). */
    fun fromPc(context: Context, pc: String, level: Int) {
        val app = context.applicationContext
        main.post {
            prefs(app).edit { putInt(pc, level.coerceIn(0, 100)) }
            if (pcId == pc) provider?.currentVolume = toSteps(level)
        }
    }

    private fun startNow(app: Context, pc: String) {
        stopNow()
        context = app
        pcId = pc
        val volume = object : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, STEPS, toSteps(prefs(app).getInt(pc, 100))) {
            override fun onSetVolumeTo(volume: Int) = set(volume)

            override fun onAdjustVolume(direction: Int) {
                when (direction) {
                    AudioManager.ADJUST_RAISE, AudioManager.ADJUST_LOWER -> set(currentVolume + direction)
                    AudioManager.ADJUST_MUTE -> set(0)
                }
            }
        }
        provider = volume
        session = runCatching {
            MediaSession(app, TAG).apply {
                setPlaybackToRemote(volume)
                playing(this)
                isActive = true
            }
        }.onFailure { Log.w(TAG, "no volume session", it) }.getOrNull()

        val keysWentLocal = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.getIntExtra(EXTRA_STREAM, -1) != AudioManager.STREAM_MUSIC) return
                if (intent.getIntExtra(EXTRA_VALUE, 0) <= 0 || !PhoneQuiet.quiet(context)) return
                val audio = context.getSystemService(AudioManager::class.java) ?: return
                runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0) }
                provider?.let { set(it.currentVolume + 1) }
                session?.let { takeKeysBack(it) }
            }
        }
        receiver = keysWentLocal
        ContextCompat.registerReceiver(app, keysWentLocal, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    private fun stopNow() {
        receiver?.let { r -> context?.let { runCatching { it.unregisterReceiver(r) } } }
        receiver = null
        session?.let { runCatching { it.release() } }
        session = null
        provider = null
        pcId = null
        context = null
    }

    private fun set(steps: Int) {
        val volume = provider ?: return
        val pc = pcId ?: return
        val app = context ?: return
        val clamped = steps.coerceIn(0, STEPS)
        volume.currentVolume = clamped
        val level = clamped * 100 / STEPS
        prefs(app).edit { putInt(pc, level) }
        (app as NectarlinkApplication).core.mirrorVolume(pc, level)
    }

    private fun playing(session: MediaSession) {
        session.setPlaybackState(
            PlaybackState.Builder()
                .setState(PlaybackState.STATE_PLAYING, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f)
                .build(),
        )
    }

    /** Playing again, so this is the most recent session and gets the keys. */
    private fun takeKeysBack(session: MediaSession) {
        session.setPlaybackState(
            PlaybackState.Builder()
                .setState(PlaybackState.STATE_PAUSED, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 0f)
                .build(),
        )
        playing(session)
    }

    private fun toSteps(level: Int): Int = (level.coerceIn(0, 100) * STEPS / 100f).roundToInt()

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
