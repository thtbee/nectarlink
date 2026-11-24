// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.Context
import android.media.AudioManager
import android.util.Log
import androidx.core.content.edit

/**
 * Keeps the phone quiet while its sound plays on the PC on its own (#94).
 *
 * Android's playback capture runs at full volume whatever the media volume is
 * (a loopback mix is fixed volume, `SwAudioOutputDescriptor::isFixedVolume`),
 * so turning the media volume to 0 silences the speaker and not the PC.
 * Muting the stream instead would silence both. Elevated captures through the
 * remote submix, which takes the sound off the speaker by itself, so it
 * doesn't need this.
 *
 * The volume to give back is kept on disk, so it comes back even if the app
 * stopped while the phone was quiet ([end] at startup).
 */
object PhoneQuiet {
    private const val TAG = "PhoneQuiet"
    private const val PREFS = "phone_quiet"
    private const val KEY_VOLUME = "volume"

    /** Turns the media volume to 0, remembering what it was. */
    fun start(context: Context) {
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        if (audio.isVolumeFixed) return
        val prefs = prefs(context)
        if (prefs.contains(KEY_VOLUME)) return
        val volume = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        if (volume == 0) return
        prefs.edit(commit = true) { putInt(KEY_VOLUME, volume) }
        runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0) }.onFailure {
            Log.w(TAG, "can't turn the media volume down", it)
            prefs.edit(commit = true) { remove(KEY_VOLUME) }
        }
    }

    /**
     * Gives the media volume back, unless the user turned it up meanwhile
     * (then theirs stays). Nothing to do if [start] didn't turn it down.
     */
    fun end(context: Context) {
        val prefs = prefs(context)
        val volume = prefs.getInt(KEY_VOLUME, -1)
        if (volume < 0) return
        prefs.edit(commit = true) { remove(KEY_VOLUME) }
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        if (audio.getStreamVolume(AudioManager.STREAM_MUSIC) == 0) {
            runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, volume, 0) }
                .onFailure { Log.w(TAG, "can't give the media volume back", it) }
        }
    }

    /** Whether [start] turned the media volume down and it's still to give back. */
    fun quiet(context: Context): Boolean = prefs(context).contains(KEY_VOLUME)

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
