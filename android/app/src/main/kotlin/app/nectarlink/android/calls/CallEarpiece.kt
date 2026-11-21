// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.content.Context
import android.media.AudioManager
import android.os.Build
import android.util.Log
import androidx.core.content.edit

/**
 * Keeps the phone's earpiece (or speaker) quiet while a call's sound is on a
 * PC (#133).
 *
 * Android's call audio interception copies the other party's voice for the
 * PC rather than moving it, so the phone kept playing the call too. The copy
 * is read from the telephony receive device, which the call volume
 * shouldn't change, so turning the call volume to its lowest quiets the
 * phone and not the PC (worth checking on each new phone). Android keeps a
 * floor for call volume (usually 1), so it's very quiet, not silent.
 *
 * The volume to give back is kept on disk, so it comes back even if the app
 * stopped while the call was on the PC ([end] at startup).
 */
object CallEarpiece {
    private const val TAG = "CallEarpiece"
    private const val PREFS = "call_earpiece"
    private const val KEY_VOLUME = "volume"

    /** Turns the call volume to its lowest, remembering what it was. */
    fun quiet(context: Context) {
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        if (audio.isVolumeFixed) return
        val prefs = prefs(context)
        if (prefs.contains(KEY_VOLUME)) return
        val lowest = lowest(audio)
        val volume = audio.getStreamVolume(AudioManager.STREAM_VOICE_CALL)
        if (volume <= lowest) return
        prefs.edit(commit = true) { putInt(KEY_VOLUME, volume) }
        runCatching { audio.setStreamVolume(AudioManager.STREAM_VOICE_CALL, lowest, 0) }.onFailure {
            Log.w(TAG, "can't turn the call volume down", it)
            prefs.edit(commit = true) { remove(KEY_VOLUME) }
        }
    }

    /**
     * Gives the call volume back, unless the user changed it meanwhile (then
     * theirs stays). Nothing to do if [quiet] didn't turn it down.
     */
    fun end(context: Context) {
        val prefs = prefs(context)
        val volume = prefs.getInt(KEY_VOLUME, -1)
        if (volume < 0) return
        prefs.edit(commit = true) { remove(KEY_VOLUME) }
        val audio = context.getSystemService(AudioManager::class.java) ?: return
        if (audio.getStreamVolume(AudioManager.STREAM_VOICE_CALL) == lowest(audio)) {
            runCatching { audio.setStreamVolume(AudioManager.STREAM_VOICE_CALL, volume, 0) }
                .onFailure { Log.w(TAG, "can't give the call volume back", it) }
        }
    }

    /** The lowest call volume; before Android 9 it can't be asked, and is 1. */
    private fun lowest(audio: AudioManager): Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) audio.getStreamMinVolume(AudioManager.STREAM_VOICE_CALL) else 1

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
