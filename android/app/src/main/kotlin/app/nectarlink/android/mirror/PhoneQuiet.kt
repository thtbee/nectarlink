// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.Context
import android.media.AudioManager
import android.os.SystemClock
import androidx.core.content.edit
import app.nectarlink.android.audio.PhonePlaybackPolicy
import kotlin.math.sqrt

/**
 * Keeps the phone quiet while its sound plays on the PC on its own (#94).
 *
 * Android's playback capture runs at full volume whatever the media volume is (a
 * loopback mix is fixed volume), so turning the media volume to 0 silences the
 * speaker and not the PC. Muting the stream would silence both. Elevated captures
 * through the remote submix, which takes the sound off the speaker by itself.
 *
 * One [Session] owns the change at a time. When another app plays while the capture
 * stays silent (RMS under [SILENT_RMS] for [SILENT_MS], an app that refuses capture),
 * the session gives the volume back so that app is heard on the phone.
 *
 * The volume to give back is kept on disk, so it comes back even if the app stopped
 * while the phone was quiet ([end] at startup).
 */
object PhoneQuiet {
    private const val PREFS = "phone_quiet"
    private const val KEY_VOLUME = "volume"
    private const val SILENT_RMS = 0.0005
    private const val SILENT_MS = 2_500L
    private var owner: Session? = null
    private var legacy: Session? = null

    /** Existing mirror caller. Leaves a previously restored session alone until [end]. */
    @Synchronized
    fun start(context: Context) {
        if (legacy != null) return
        val playback = PhonePlaybackPolicy.forContext(context)
        legacy = begin(context) { playback?.phonePlaybackActive?.value == true }
    }

    /** Existing mirror teardown and process startup recovery. */
    @Synchronized
    fun end(context: Context) {
        val session = legacy
        legacy = null
        if (session != null) session.close()
        else if (owner == null) restoreVolume(context)
    }

    @Synchronized
    fun begin(context: Context, phonePlaybackActive: () -> Boolean): Session {
        val app = context.applicationContext
        val session = Session(app, phonePlaybackActive)
        val audio = app.getSystemService(AudioManager::class.java) ?: return session
        val prefs = prefs(app)
        if (owner != null || audio.isVolumeFixed || prefs.contains(KEY_VOLUME)) return session
        val volume = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        if (volume == 0) return session
        prefs.edit(commit = true) { putInt(KEY_VOLUME, volume) }
        if (runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0) }.isSuccess) {
            owner = session
        } else {
            prefs.edit(commit = true) { remove(KEY_VOLUME) }
        }
        return session
    }

    internal fun observeLegacyPcm(pcm: ByteArray) {
        val session = synchronized(this) { legacy }
        session?.observePcm(pcm)
    }

    class Session internal constructor(
        private val context: Context,
        private val phonePlaybackActive: () -> Boolean,
    ) : AutoCloseable {
        private var silenceSince: Long? = null

        fun observePcm(pcm: ByteArray, nowMs: Long = SystemClock.elapsedRealtime()) {
            if (pcm.size < 2 || synchronized(PhoneQuiet) { owner !== this }) return
            val silentDuringPlayback = phonePlaybackActive() && rms(pcm) < SILENT_RMS
            synchronized(PhoneQuiet) {
                if (owner !== this) return
                if (!silentDuringPlayback) {
                    silenceSince = null
                } else {
                    val since = silenceSince ?: nowMs.also { silenceSince = it }
                    if (nowMs - since >= SILENT_MS) close()
                }
            }
        }

        override fun close() {
            synchronized(PhoneQuiet) {
                if (owner !== this) return
                restoreVolume(context)
                owner = null
            }
        }
    }

    private fun restoreVolume(context: Context) {
        val prefs = prefs(context)
        val volume = prefs.getInt(KEY_VOLUME, -1)
        if (volume < 0) return
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        if (audio.getStreamVolume(AudioManager.STREAM_MUSIC) != 0 ||
            runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, volume, 0) }.isSuccess
        ) {
            prefs.edit(commit = true) { remove(KEY_VOLUME) }
        }
    }

    private fun rms(pcm: ByteArray): Double {
        var squares = 0.0
        for (i in 0 until pcm.size - 1 step 2) {
            val sample = ((pcm[i].toInt() and 0xff) or (pcm[i + 1].toInt() shl 8)).toShort() / 32768.0
            squares += sample * sample
        }
        return sqrt(squares / (pcm.size / 2))
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
