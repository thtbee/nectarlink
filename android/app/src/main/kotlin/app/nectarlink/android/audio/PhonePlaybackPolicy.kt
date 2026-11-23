// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.AudioPlaybackConfiguration
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class PlaybackPolicy(val activeGain: Float) {
    PAUSE(0f), MUTE(0f), LOWER(0.15f), OFF(1f);

    fun gain(phonePlaybackActive: Boolean): Float = if (phonePlaybackActive) activeGain else 1f
}

/** Mirrors the playback decision without depending on Android playback configuration objects. */
internal class PlaybackDebounce {
    var active = false
        private set
    private var candidate = false
    private var changedAtMs = 0L

    fun update(otherPlayback: Boolean, nowMs: Long): Long? {
        if (candidate != otherPlayback) {
            candidate = otherPlayback
            changedAtMs = nowMs
        }
        if (candidate == active) return null
        val remaining = (if (candidate) 150L else 1_500L) - (nowMs - changedAtMs)
        if (remaining > 0) return remaining
        active = candidate
        return null
    }
}

internal data class PlaybackObservation(
    val relevant: Boolean,
    val couldBeSpeaker: Boolean,
    val uid: Int?,
    val playerId: Int?,
)

internal fun hasOtherPlayback(
    observed: List<PlaybackObservation>,
    ownUid: Int,
    speakerMayBePlaying: Boolean,
    beforeSpeaker: Set<Int>,
): Boolean {
    val ambiguousPlayers = mutableSetOf<Int>()
    for (player in observed) {
        if (!player.relevant || player.uid == ownUid) continue
        if (player.uid != null || !speakerMayBePlaying || !player.couldBeSpeaker ||
            player.playerId in beforeSpeaker
        ) return true
        // One anonymous media player could be our AudioTrack, even when its ID
        // first appeared after play(). Two distinct IDs require another player.
        player.playerId?.let { ambiguousPlayers += it }
    }
    return ambiguousPlayers.size > 1
}

/** Tracks phone playback for ducking, pausing, and the silent-capture watchdog. */
class PhonePlaybackPolicy(private val audio: AudioManager) {
    private val handler = Handler(Looper.getMainLooper())
    private val active = MutableStateFlow(false)
    val phonePlaybackActive: StateFlow<Boolean> = active.asStateFlow()

    private var debounce = PlaybackDebounce()
    private var registered = false
    private var speakerMayBePlaying = false
    private var beforeSpeaker = emptySet<Int>()
    private val refresh = Runnable { refresh() }
    private val callback = object : AudioManager.AudioPlaybackCallback() {
        override fun onPlaybackConfigChanged(configs: MutableList<AudioPlaybackConfiguration>) {
            // Read the current snapshot. A callback queued before play() or release()
            // can be delivered in the opposite track lifecycle state.
            refresh()
        }
    }

    @Synchronized
    fun start() {
        if (registered) return
        registered = runCatching { audio.registerAudioPlaybackCallback(callback, handler) }.isSuccess
        refresh()
    }

    @Synchronized
    fun stop() {
        if (registered) runCatching { audio.unregisterAudioPlaybackCallback(callback) }
        registered = false
        handler.removeCallbacks(refresh)
        debounce = PlaybackDebounce()
        active.value = false
    }

    @Synchronized
    internal fun beforeTrackStart() {
        val ownUid = Process.myUid()
        beforeSpeaker = observations().filter { it.uid != ownUid }.mapNotNull { it.playerId }.toSet()
        speakerMayBePlaying = true
        refresh()
    }

    @Synchronized
    internal fun afterTrackStart() {
        refresh()
    }

    @Synchronized
    internal fun afterTrackRelease() {
        speakerMayBePlaying = false
        beforeSpeaker = emptySet()
        refresh()
    }

    @Synchronized
    private fun refresh() {
        if (!registered) return
        handler.removeCallbacks(refresh)
        val other = hasOtherPlayback(observations(), Process.myUid(), speakerMayBePlaying, beforeSpeaker)
        debounce.update(other, SystemClock.elapsedRealtime())?.let { handler.postDelayed(refresh, it) }
        active.value = debounce.active
    }

    private fun observations(): List<PlaybackObservation> = runCatching {
        // Android returns anonymous active configurations to unprivileged apps.
        // PIID and UID are hidden APIs. Treat missing/negative debug fields as unknown.
        audio.activePlaybackConfigurations.mapNotNull { config ->
            val description = config.toString()
            val state = STATE.find(description)?.groupValues?.get(1)
            if (state != null && !state.equals("started", ignoreCase = true)) return@mapNotNull null
            val usage = config.audioAttributes.usage
            val couldBeSpeaker = usage == AudioAttributes.USAGE_MEDIA || usage == AudioAttributes.USAGE_UNKNOWN
            PlaybackObservation(
                relevant = couldBeSpeaker || usage == AudioAttributes.USAGE_GAME ||
                    usage == AudioAttributes.USAGE_VOICE_COMMUNICATION || usage == AudioAttributes.USAGE_ASSISTANT,
                couldBeSpeaker = couldBeSpeaker,
                uid = UID.nonnegativeInt(description),
                playerId = PIID.nonnegativeInt(description),
            )
        }
    }.getOrDefault(emptyList())

    private fun Regex.nonnegativeInt(value: String): Int? =
        find(value)?.groupValues?.get(1)?.toIntOrNull()?.takeIf { it >= 0 }

    companion object {
        private var shared: PhonePlaybackPolicy? = null

        @Synchronized
        fun forContext(context: Context): PhonePlaybackPolicy? {
            shared?.let { return it }
            val manager = context.applicationContext.getSystemService(AudioManager::class.java) ?: return null
            return PhonePlaybackPolicy(manager).also { policy ->
                shared = policy
                policy.start()
            }
        }

        private val PIID = Regex("""\bpiid:\s*(-?\d+)""")
        private val UID = Regex("""\bu/pid:\s*(-?\d+)/""")
        private val STATE = Regex("""\bstate:\s*(\w+)""")
    }
}
