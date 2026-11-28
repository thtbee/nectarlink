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
 * Runs on the main thread. NectarLink's own session (a PC's media, see
 * [PcMedia]) is left out, so a PC never gets its own music back.
 */
class PhoneMedia(context: Context, private val onChange: (List<MediaPlayer>) -> Unit) {
    /** What plays on this phone, for the media center (#536). */
    data class Players(
        val players: List<MediaPlayer>,
        /** `SystemClock.elapsedRealtime` when their positions were right. */
        val atElapsedMs: Long,
    )

    private val _players = kotlinx.coroutines.flow.MutableStateFlow(Players(emptyList(), 0L))

    /** This phone's own players, playing ones first, for the media center (#536). */
    val players: kotlinx.coroutines.flow.StateFlow<Players> = _players

    private val context = context.applicationContext
    private val manager = context.getSystemService(MediaSessionManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    /** Watched sessions, with their callbacks. */
    private val watched = mutableMapOf<MediaController, MediaController.Callback>()
    /** The players last sent, by ID, for commands. */
    @Volatile
    private var controllers: Map<String, MediaController> = emptyMap()
    @Volatile
    var activeSeedColor: UInt? = null
        private set
    private val appNames = java.util.concurrent.ConcurrentHashMap<String, String>()
    private val art = ArtCache()
    private val worker = java.util.concurrent.Executors.newSingleThreadExecutor { r ->
        Thread(r, "nectarlink-phone-media").apply { isDaemon = true }
    }
    @Volatile
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
            running = true
            watch(manager.getActiveSessions(component))
        } catch (e: SecurityException) {
            running = false
            Log.w(TAG, "no access to media sessions", e)
        }
    }

    fun stop() {
        if (!running) return
        running = false
        manager.removeOnActiveSessionsChangedListener(sessionsChanged)
        watched.forEach { (controller, callback) -> controller.unregisterCallback(callback) }
        watched.clear()
        main.removeCallbacks(publish)
        scheduled = false
        worker.execute {
            controllers = emptyMap()
            lastSnapshots = emptyList()
            activeSeedColor = null
            lastSent = null
            _players.value = Players(emptyList(), SystemClock.elapsedRealtime())
            onChange(emptyList())
        }
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
    @Volatile
    private var lastSent: List<MediaPlayer>? = null
    @Volatile
    private var lastSentAt = 0L
    @Volatile
    private var lastSnapshots: List<ControllerSnapshot> = emptyList()

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

    /** Snapshot of the currently active media session with an optional source URI/ID. */
    data class MediaLinkSnapshot(
        val packageName: String,
        val title: String?,
        val mediaUri: String?,
        val mediaId: String?,
        val positionSecs: UInt?,
    )

    private class ControllerSnapshot(
        val controller: MediaController,
        val packageName: String,
        val title: String,
        val artist: String?,
        val rawArtist: String?,
        val album: String?,
        val playing: Boolean,
        val duration: Long?,
        val position: Long?,
        val actions: List<MediaAction>,
        val picture: Bitmap?,
        val updated: Long,
        val mediaUri: String?,
        val mediaId: String?,
    )

    private fun snapshot(controller: MediaController): ControllerSnapshot? {
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
        return ControllerSnapshot(
            controller = controller,
            packageName = controller.packageName,
            title = title,
            artist = metadata.text(MediaMetadata.METADATA_KEY_ARTIST)
                ?: metadata.text(MediaMetadata.METADATA_KEY_ALBUM_ARTIST)
                ?: metadata.text(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE),
            rawArtist = metadata.getString(MediaMetadata.METADATA_KEY_ARTIST),
            album = metadata.text(MediaMetadata.METADATA_KEY_ALBUM),
            playing = playing,
            duration = duration,
            position = position,
            actions = actions,
            picture = picture,
            updated = state?.lastPositionUpdateTime ?: 0L,
            mediaUri = metadata.text(MediaMetadata.METADATA_KEY_MEDIA_URI),
            mediaId = metadata.text(MediaMetadata.METADATA_KEY_MEDIA_ID),
        )
    }

    private fun publishNow() {
        scheduled = false
        val snapshots = watched.keys
            .mapNotNull(::snapshot)
            .sortedWith(
                compareByDescending<ControllerSnapshot> { it.playing }
                    .thenByDescending { it.updated },
            )
            .take(MAX_PLAYERS)
        worker.execute {
            if (!running) return@execute
            val ids = mutableMapOf<String, MediaController>()
            var topPlayingSeed: UInt? = null
            var firstSeed: UInt? = null
            val players = snapshots.map { snap ->
                var id = snap.packageName
                var n = 2
                while (id in ids) id = "${snap.packageName}#${n++}"
                ids[id] = snap.controller
                val track = listOf(snap.packageName, snap.title, snap.rawArtist, snap.album).joinToString("\u0001")
                val cachedArt = snap.picture?.let { art.get(track, it) }
                if (firstSeed == null && cachedArt?.seedColor != null) {
                    firstSeed = cachedArt.seedColor
                }
                if (topPlayingSeed == null && snap.playing && cachedArt?.seedColor != null) {
                    topPlayingSeed = cachedArt.seedColor
                }
                MediaPlayer(
                    id = id,
                    app = appName(snap.packageName),
                    title = snap.title,
                    artist = snap.artist,
                    album = snap.album,
                    playing = snap.playing,
                    duration = snap.duration?.toULong(),
                    position = snap.position?.toULong(),
                    actions = snap.actions,
                    artKey = cachedArt?.key,
                    art = cachedArt?.jpeg,
                )
            }
            if (!running) return@execute
            controllers = ids
            lastSnapshots = snapshots
            activeSeedColor = topPlayingSeed ?: firstSeed
            val now = SystemClock.elapsedRealtime()
            if (sameButLater(lastSent, lastSentAt, players, now)) return@execute
            lastSent = players
            lastSentAt = now
            _players.value = Players(players, now)
            onChange(players)
        }
    }

    private fun MediaMetadata.text(key: String): String? = getString(key)?.trim()?.takeIf { it.isNotEmpty() }

    private fun appName(packageName: String): String = appNames.getOrPut(packageName) {
        runCatching {
            val pm = context.packageManager
            pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString()
        }.getOrDefault(packageName)
    }

    /** The media center's command for one of this phone's players (#536); false when it's gone. */
    fun control(player: String, action: MediaAction, position: ULong? = null): Boolean =
        runCatching { command(player, action, position) }.isSuccess

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

    private fun livePositionSeconds(position: Long?, duration: Long?, playing: Boolean, atMs: Long): UInt? {
        val baseMs = position ?: return null
        val elapsedMs = if (playing && atMs > 0) {
            (SystemClock.elapsedRealtime() - atMs).coerceAtLeast(0L)
        } else {
            0L
        }
        val totalMs = (baseMs + elapsedMs).let { pos ->
            duration?.let { dur -> pos.coerceAtMost(dur) } ?: pos
        }
        val secs = (totalMs / 1000L).toInt()
        return if (secs > 0) secs.toUInt() else null
    }

    /** Current playback position in seconds of the active media session, if any. */
    fun activePositionSeconds(): UInt? {
        val list = lastSent ?: return null
        val first = list.firstOrNull { it.playing } ?: list.firstOrNull() ?: return null
        return livePositionSeconds(first.position?.toLong(), first.duration?.toLong(), first.playing, lastSentAt)
    }

    /** Metadata and live playback position for the active media session, if any. */
    internal fun activeMediaLink(): MediaLinkSnapshot? {
        val snap = lastSnapshots.firstOrNull { it.playing } ?: lastSnapshots.firstOrNull() ?: return null
        val secs = livePositionSeconds(snap.position, snap.duration, snap.playing, lastSentAt)
        return MediaLinkSnapshot(
            packageName = snap.packageName,
            title = snap.title,
            mediaUri = snap.mediaUri,
            mediaId = snap.mediaId,
            positionSecs = secs,
        )
    }

    private class CachedArt(val key: String, val jpeg: ByteArray, val seedColor: UInt?)

    /**
     * Artwork as a small JPEG, made once per track and picture size (apps
     * hand over a new copy of the same picture with every update).
     */
    private class ArtCache {
        private val made = object : LinkedHashMap<String, CachedArt>(8, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, CachedArt>) = size > 8
        }

        @Synchronized
        fun get(track: String, bitmap: Bitmap): CachedArt {
            val identity = "$track\u0001${bitmap.width}x${bitmap.height}"
            made[identity]?.let { return it }
            val seedColor = extractArtworkSeedColor(bitmap, identity)
            val scale = minOf(1f, ART_SIZE.toFloat() / maxOf(bitmap.width, bitmap.height))
            val small = if (scale < 1f) {
                bitmap.scale((bitmap.width * scale).toInt().coerceAtLeast(1), (bitmap.height * scale).toInt().coerceAtLeast(1))
            } else {
                bitmap
            }
            val jpeg = ByteArrayOutputStream().also { small.compress(Bitmap.CompressFormat.JPEG, 88, it) }.toByteArray()
            if (small !== bitmap) small.recycle()
            val key = jpeg.contentHashCode().toUInt().toString(16) + "-" + jpeg.size.toString(16)
            return CachedArt(key, jpeg, seedColor).also { made[identity] = it }
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

/**
 * Extracts a perceptual Material 3 seed color (`0xRRGGBB`) using the shared
 * Rust perceptual color engine (`app.nectarlink.core.extractArtworkScheme`) over a
 * downsampled `<= 112x112` RGBA buffer so both Android and Desktop select the exact
 * same color for the same artwork.
 */
internal fun extractArtworkSeedColor(bitmap: Bitmap, cacheKey: String? = null): UInt? = runCatching {
    val w0 = bitmap.width
    val h0 = bitmap.height
    if (w0 <= 0 || h0 <= 0) return null
    val maxDim = 112
    val scale = minOf(1f, maxDim.toFloat() / maxOf(w0, h0))
    val w = (w0 * scale).toInt().coerceAtLeast(1)
    val h = (h0 * scale).toInt().coerceAtLeast(1)
    val sampled = if (w != w0 || h != h0) {
        bitmap.scale(w, h)
    } else {
        bitmap
    }
    val pixels = IntArray(w * h)
    sampled.getPixels(pixels, 0, w, 0, 0, w, h)
    if (sampled !== bitmap) sampled.recycle()

    val rgba = ByteArray(w * h * 4)
    var outIdx = 0
    for (px in pixels) {
        rgba[outIdx] = ((px ushr 16) and 0xFF).toByte()
        rgba[outIdx + 1] = ((px ushr 8) and 0xFF).toByte()
        rgba[outIdx + 2] = (px and 0xFF).toByte()
        rgba[outIdx + 3] = ((px ushr 24) and 0xFF).toByte()
        outIdx += 4
    }
    return app.nectarlink.core.extractArtworkScheme(rgba, w.toUInt(), h.toUInt(), cacheKey)?.seedRgb
}.getOrNull()

