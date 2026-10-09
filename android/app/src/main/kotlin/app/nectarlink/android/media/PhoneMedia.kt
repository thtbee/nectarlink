// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.media

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.core.graphics.scale
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MediaFailure
import app.nectarlink.core.MediaPlayer
import java.io.ByteArrayOutputStream

/**
 * What plays on this phone, for paired PCs (docs/protocol/media.md): the
 * apps' media sessions, read with notification access, and the PCs'
 * commands for them.
 *
 * Runs on the main thread. Nectarlink's own session (a PC's media, see
 * [PcMedia]) is left out, so a PC never gets its own music back.
 */
class PhoneMedia(context: Context, private val onChange: (List<MediaPlayer>) -> Unit) {
    private val context = context.applicationContext
    private val manager = context.getSystemService(MediaSessionManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    /** Watched sessions, with their callbacks. */
    private val watched = mutableMapOf<MediaController, MediaController.Callback>()
    /** The players last sent, by ID, for commands. */
    @Volatile
    private var controllers: Map<String, MediaController> = emptyMap()
    private val art = ArtCache()
    private var running = false

    private val sessionsChanged = MediaSessionManager.OnActiveSessionsChangedListener { list ->
        watch(list.orEmpty())
    }
    private val publish = Runnable { publishNow() }

    /** Starts watching; needs notification access. */
    fun start() {
        if (running) return
        val component = NotificationListener.component(context)
        try {
            manager.addOnActiveSessionsChangedListener(sessionsChanged, component, main)
            watch(manager.getActiveSessions(component))
            running = true
        } catch (e: SecurityException) {
            Log.w(TAG, "no access to media sessions", e)
        }
    }

    fun stop() {
        if (!running) return
        running = false
        manager.removeOnActiveSessionsChangedListener(sessionsChanged)
        watched.forEach { (controller, callback) -> controller.unregisterCallback(callback) }
        watched.clear()
        controllers = emptyMap()
        main.removeCallbacks(publish)
        scheduled = false
        lastSent = null
        onChange(emptyList())
    }

    private fun watch(sessions: List<MediaController>) {
        val current = sessions.filter { it.packageName != context.packageName }
        watched.keys.filter { old -> current.none { it.sessionToken == old.sessionToken } }.forEach { gone ->
            watched.remove(gone)?.let(gone::unregisterCallback)
        }
        for (controller in current) {
            if (watched.keys.any { it.sessionToken == controller.sessionToken }) continue
            val callback = object : MediaController.Callback() {
                override fun onMetadataChanged(metadata: MediaMetadata?) = schedule()
                override fun onPlaybackStateChanged(state: PlaybackState?) = schedule()
                override fun onSessionDestroyed() = schedule()
            }
            controller.registerCallback(callback, main)
            watched[controller] = callback
        }
        schedule()
    }

    /** Whether a publish is on its way. */
    private var scheduled = false
    /** What was sent last, and when (for telling real changes from time passing). */
    private var lastSent: List<MediaPlayer>? = null
    private var lastSentAt = 0L

    /**
     * Changes come in bursts (metadata, then state, then artwork), so they're
     * sent together shortly after the first one. Not later: some players
     * report their state several times a second, and waiting for a quiet
     * moment would hold the update back for as long as they play.
     */
    private fun schedule() {
        if (scheduled) return
        scheduled = true
        main.postDelayed(publish, SETTLE_MS)
    }

    private fun publishNow() {
        scheduled = false
        val ids = mutableMapOf<String, MediaController>()
        val players = watched.keys
            .mapNotNull { controller -> read(controller)?.let { controller to it } }
            .sortedWith(
                compareByDescending<Pair<MediaController, Reading>> { it.second.playing }
                    .thenByDescending { it.second.updated },
            )
            .take(MAX_PLAYERS)
            .map { (controller, reading) ->
                // One app may have more than one session.
                var id = controller.packageName
                var n = 2
                while (id in ids) id = "${controller.packageName}#${n++}"
                ids[id] = controller
                reading.player.copy(id = id)
            }
        controllers = ids
        val now = SystemClock.elapsedRealtime()
        if (sameButLater(lastSent, lastSentAt, players, now)) return
        lastSent = players
        lastSentAt = now
        onChange(players)
    }

    private class Reading(val player: MediaPlayer, val playing: Boolean, val updated: Long)

    private fun read(controller: MediaController): Reading? {
        val metadata = controller.metadata ?: return null
        val title = metadata.text(MediaMetadata.METADATA_KEY_TITLE)
            ?: metadata.text(MediaMetadata.METADATA_KEY_DISPLAY_TITLE)
            ?: return null
        val state = controller.playbackState
        val playing = state?.state == PlaybackState.STATE_PLAYING ||
            state?.state == PlaybackState.STATE_BUFFERING
        val duration = metadata.getLong(MediaMetadata.METADATA_KEY_DURATION).takeIf { it > 0 }
        val position = state?.let { s ->
            val moved = if (playing && s.lastPositionUpdateTime > 0) {
                ((SystemClock.elapsedRealtime() - s.lastPositionUpdateTime) * s.playbackSpeed).toLong()
            } else {
                0L
            }
            (s.position + moved).coerceAtLeast(0).let { if (duration != null) it.coerceAtMost(duration) else it }
        }
        val bits = state?.actions ?: 0L
        val actions = buildList {
            if (bits and (PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PLAY_PAUSE) != 0L) add(MediaAction.PLAY)
            if (bits and (PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE) != 0L) add(MediaAction.PAUSE)
            if (bits and PlaybackState.ACTION_SKIP_TO_NEXT != 0L) add(MediaAction.NEXT)
            if (bits and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L) add(MediaAction.PREVIOUS)
            if (bits and PlaybackState.ACTION_SEEK_TO != 0L && duration != null) add(MediaAction.SEEK)
        }
        val picture = metadata.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: metadata.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: metadata.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)
        val track = listOf(controller.packageName, title, metadata.getString(MediaMetadata.METADATA_KEY_ARTIST),
            metadata.getString(MediaMetadata.METADATA_KEY_ALBUM)).joinToString("\u0001")
        val (artKey, artBytes) = picture?.let { art.get(track, it) } ?: (null to null)
        val player = MediaPlayer(
            id = controller.packageName,
            app = appName(controller.packageName),
            title = title,
            artist = metadata.text(MediaMetadata.METADATA_KEY_ARTIST)
                ?: metadata.text(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)
                ?: metadata.text(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE),
            album = metadata.text(MediaMetadata.METADATA_KEY_ALBUM),
            playing = playing,
            duration = duration?.toULong(),
            position = position?.toULong(),
            actions = actions,
            artKey = artKey,
            art = artBytes,
        )
        return Reading(player, playing, state?.lastPositionUpdateTime ?: 0)
    }

    private fun MediaMetadata.text(key: String): String? = getString(key)?.trim()?.takeIf { it.isNotEmpty() }

    private fun appName(packageName: String): String = runCatching {
        val pm = context.packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString()
    }.getOrDefault(packageName)

    /** A PC's command for one of the players last sent. */
    fun command(player: String, action: MediaAction, position: ULong?) {
        val controller = controllers[player] ?: throw MediaFailure.NotFound()
        val controls = controller.transportControls
        when (action) {
            MediaAction.PLAY -> controls.play()
            MediaAction.PAUSE -> controls.pause()
            MediaAction.NEXT -> controls.skipToNext()
            MediaAction.PREVIOUS -> controls.skipToPrevious()
            MediaAction.SEEK -> controls.seekTo(position?.toLong() ?: throw MediaFailure.Unsupported())
        }
    }

    /** Current playback position in seconds of the active media session, if any. */
    fun activePositionSeconds(): UInt? {
        val list = lastSent ?: return null
        val first = list.firstOrNull { it.playing } ?: list.firstOrNull() ?: return null
        val baseMs = first.position?.toLong() ?: return null
        val elapsedMs = if (first.playing && lastSentAt > 0) {
            (SystemClock.elapsedRealtime() - lastSentAt).coerceAtLeast(0L)
        } else {
            0L
        }
        val totalMs = (baseMs + elapsedMs).let { pos ->
            first.duration?.toLong()?.let { dur -> pos.coerceAtMost(dur) } ?: pos
        }
        val secs = (totalMs / 1000L).toInt()
        return if (secs > 0) secs.toUInt() else null
    }

    /**
     * Artwork as a small JPEG, made once per track and picture size (apps
     * hand over a new copy of the same picture with every update).
     */
    private class ArtCache {
        private val made = object : LinkedHashMap<String, Pair<String, ByteArray>>(8, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Pair<String, ByteArray>>) = size > 8
        }

        fun get(track: String, bitmap: Bitmap): Pair<String, ByteArray> {
            val identity = "$track\u0001${bitmap.width}x${bitmap.height}"
            made[identity]?.let { return it }
            val scale = minOf(1f, ART_SIZE.toFloat() / maxOf(bitmap.width, bitmap.height))
            val small = if (scale < 1f) {
                bitmap.scale((bitmap.width * scale).toInt().coerceAtLeast(1), (bitmap.height * scale).toInt().coerceAtLeast(1))
            } else {
                bitmap
            }
            val jpeg = ByteArrayOutputStream().also { small.compress(Bitmap.CompressFormat.JPEG, 88, it) }.toByteArray()
            if (small !== bitmap) small.recycle()
            val key = jpeg.contentHashCode().toUInt().toString(16) + "-" + jpeg.size.toString(16)
            return (key to jpeg).also { made[identity] = it }
        }
    }

    private companion object {
        /**
         * Whether [next] only differs from [last] by time passing: positions
         * moved on as they would while playing (or not at all while paused).
         */
        fun sameButLater(last: List<MediaPlayer>?, lastAt: Long, next: List<MediaPlayer>, now: Long): Boolean {
            if (last == null || last.size != next.size) return false
            return last.zip(next).all { (a, b) ->
                val expected = a.position?.toLong()?.plus(if (a.playing) now - lastAt else 0)
                val actual = b.position
                val drift = if (expected != null && actual != null) kotlin.math.abs(actual.toLong() - expected) else 0
                a.copy(position = null, art = null) == b.copy(position = null, art = null) && drift < POSITION_SLACK_MS
            }
        }

        /** Position jitter that isn't a seek. */
        const val POSITION_SLACK_MS = 1_500L
        const val TAG = "PhoneMedia"
        const val SETTLE_MS = 200L
        const val MAX_PLAYERS = 4
        /** Artwork's longest side, in pixels. */
        const val ART_SIZE = 360
    }
}
