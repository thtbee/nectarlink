// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.media

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.core.content.ContextCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MediaPlayer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/**
 * What plays on a paired PC, in Android's own media controls (the
 * notification shade and Quick Settings player): a media session that
 * stands for the PC's player, and its notification. What the user presses
 * goes to the PC through [send]; the PC's new state comes back as an update.
 *
 * While a PC player plays and [volumeKeys] is on, the session's volume is
 * the PC's: Android then sends the phone's volume buttons to it (unless
 * the phone plays something itself), and [setPcAudio] passes them on.
 *
 * Applies snapshots in arrival order and decodes artwork off the main thread.
 */
class PcMedia(
    context: Context,
    scope: CoroutineScope,
    private val onSeedColorChanged: () -> Unit = {},
    private val setPcAudio: (pc: String, volume: Int?, muted: Boolean?) -> Unit = { _, _, _ -> },
    private val send: (pc: String, player: String, action: MediaAction, position: ULong?) -> Unit,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())

    /** Players per PC, in the PC's order, and the PCs' names. */
    private val players = linkedMapOf<String, List<MediaPlayer>>()
    private val names = mutableMapOf<String, String>()
    /** Decoded artwork and seed color by PC and key (artwork arrives once per key). */
    private val art = object : LinkedHashMap<String, Pair<Bitmap, UInt?>>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Pair<Bitmap, UInt?>>) = size > 8
    }

    @androidx.compose.runtime.Immutable
    data class Summary(
        val playerId: String,
        val title: String,
        val subtitle: String,
        val playing: Boolean,
        val artwork: Bitmap?,
    )

    @Volatile
    var activeSeedColor: UInt? = null
        private set

    private val _playing = MutableStateFlow<Map<String, Boolean>>(emptyMap())

    /** Whether each PC player plays, by "pc/player" (for Play on phone's controls). */
    val playing: StateFlow<Map<String, Boolean>> = _playing

    /**
     * Where a PC player is in what it plays: [positionMs] as of [atElapsedMs]
     * (`SystemClock.elapsedRealtime`), moving on while [playing].
     */
    data class Timeline(
        val positionMs: Long,
        val durationMs: Long,
        val playing: Boolean,
        val canSeek: Boolean,
        val atElapsedMs: Long,
    ) {
        /** Where it is now. */
        fun positionAt(elapsedMs: Long): Long =
            (positionMs + if (playing) (elapsedMs - atElapsedMs).coerceAtLeast(0) else 0).coerceIn(0, durationMs)
    }

    private val _timelines = MutableStateFlow<Map<String, Timeline>>(emptyMap())

    /** Each PC player's [Timeline] that has one, by "pc/player" (Play on phone's seek bar). */
    val timelines: StateFlow<Map<String, Timeline>> = _timelines

    private val _summaries = MutableStateFlow<Map<String, Summary>>(emptyMap())
    val summaries: StateFlow<Map<String, Summary>> = _summaries

    private data class Snapshot(val pc: String, val name: String, val players: List<MediaPlayer>)
    private val updates = Channel<Snapshot>(Channel.UNLIMITED)

    init {
        // The Rust listener delivers snapshots and disconnects in order. Keep
        // that order even when an earlier snapshot takes longer to decode.
        scope.launch(Dispatchers.Default) {
            for (update in updates) applyUpdate(update.pc, update.name, update.players)
        }
    }

    private var session: MediaSession? = null
    private var shown: Pair<String, MediaPlayer>? = null

    /** Each PC's master volume, for the PCs whose volume this phone may set. */
    private var levels: Map<String, PcLevel> = emptyMap()
    private var volumeKeys = true
    /** The PC the session's volume stands for, while it does. */
    private var remote: PcVolume? = null

    /** A PC's master volume (`0..=100`) and mute. */
    data class PcLevel(val volume: Int, val muted: Boolean)
    private val stalePauseTimeout = Runnable { hide() }

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = command(MediaAction.PLAY)
        override fun onPause() = command(MediaAction.PAUSE)
        override fun onSkipToNext() = command(MediaAction.NEXT)
        override fun onSkipToPrevious() = command(MediaAction.PREVIOUS)
        override fun onSeekTo(pos: Long) = command(MediaAction.SEEK, pos.coerceAtLeast(0).toULong())
    }

    /** A PC's players changed (empty: nothing plays there, or it's offline). */
    fun update(pc: String, pcName: String, list: List<MediaPlayer>) {
        updates.trySend(Snapshot(pc, pcName, list))
    }

    private fun applyUpdate(pc: String, pcName: String, list: List<MediaPlayer>) {
        val decoded = list.mapNotNull { player ->
            val key = player.artKey ?: return@mapNotNull null
            val bytes = player.art ?: return@mapNotNull null
            val bmp = decodeArtwork(bytes) ?: return@mapNotNull null
            "$pc/$key" to (bmp to extractArtworkSeedColor(bmp, "$pc/$key"))
        }
        main.post {
            names[pc] = pcName
            for ((cacheKey, entry) in decoded) {
                art[cacheKey] = entry
            }
            if (list.isEmpty()) players.remove(pc) else players[pc] = list
            _playing.value = players.flatMap { (id, all) -> all.map { "$id/${it.id}" to it.playing } }.toMap()
            val now = android.os.SystemClock.elapsedRealtime()
            _timelines.value = players.flatMap { (id, all) ->
                all.mapNotNull { p ->
                    val duration = p.duration?.toLong()?.takeIf { it > 0 } ?: return@mapNotNull null
                    "$id/${p.id}" to Timeline(
                        positionMs = (p.position?.toLong() ?: 0).coerceIn(0, duration),
                        durationMs = duration,
                        playing = p.playing,
                        canSeek = MediaAction.SEEK in p.actions,
                        atElapsedMs = now,
                    )
                }
            }.toMap()
            render()
        }
    }

    /** The volume of each PC this phone may set (from its deck state). */
    fun levels(levels: Map<String, PcLevel>) = main.post {
        if (levels == this.levels) return@post
        this.levels = levels
        session?.let { route(it) }
    }

    /** Whether the volume buttons may set a playing PC's volume (Settings). */
    fun volumeKeys(enabled: Boolean) = main.post {
        if (enabled == volumeKeys) return@post
        volumeKeys = enabled
        session?.let { route(it) }
    }

    /** Asks the PC [pc]'s [player] to [action] (Play on phone's controls). */
    fun command(pc: String, player: String, action: MediaAction) = send(pc, player, action, null)

    /** Asks the PC [pc]'s [player] to go to [positionMs] (Play on phone's seek bar). */
    fun seek(pc: String, player: String, positionMs: Long) =
        send(pc, player, MediaAction.SEEK, positionMs.coerceAtLeast(0).toULong())

    private fun command(action: MediaAction, position: ULong? = null) {
        val (pc, player) = shown ?: return
        send(pc, player.id, action, position)
    }

    private fun summary(pc: String, player: MediaPlayer): Summary {
        val pcName = names[pc].orEmpty().ifBlank { context.getString(R.string.your_pc) }
        val cleanApp = sanitizeAppName(player.app)
        val cleanTitle = player.title?.trim()?.takeIf { it.isNotEmpty() } ?: cleanApp.ifEmpty { pcName }
        val cleanArtist = player.artist?.let(::sanitizeArtistLabel)?.takeIf {
            it.isNotEmpty() && !it.equals(cleanApp, ignoreCase = true) && !it.equals(cleanTitle, ignoreCase = true)
        }
        val titleAlreadyMentionsApp = cleanApp.isNotEmpty() &&
            cleanTitle.endsWith(" on $cleanApp", ignoreCase = true)
        val where = if (cleanApp.isEmpty() ||
            titleAlreadyMentionsApp ||
            cleanApp.equals("Browser", ignoreCase = true) ||
            cleanApp.equals("Media", ignoreCase = true)
        ) {
            pcName
        } else {
            context.getString(R.string.media_on, cleanApp, pcName)
        }
        val subtitleLine = listOfNotNull(cleanArtist, where).distinct().joinToString(" · ")

        return Summary(
            playerId = player.id,
            title = cleanTitle,
            subtitle = subtitleLine,
            playing = player.playing,
            artwork = player.artKey?.let { art["$pc/$it"]?.first },
        )
    }

    private fun render() {
        main.removeCallbacks(stalePauseTimeout)
        val preferred = players.mapValues { (_, list) -> list.firstOrNull { it.playing } ?: list.first() }
        val nextSummaries = preferred.mapValues { (pc, player) -> summary(pc, player) }
        if (_summaries.value != nextSummaries) {
            _summaries.value = nextSummaries
            app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context)
        }

        val first = preferred.entries.firstOrNull { it.value.playing } ?: preferred.entries.firstOrNull()
        if (first == null) return hide()
        val (pc, player) = first
        shown = pc to player
        val session = session ?: MediaSession(context, "NectarLink PC media").also {
            it.setCallback(callback, main)
            it.setSessionActivity(openApp())
            session = it
        }
        val nextSeed = player.artKey?.let { art["$pc/$it"]?.second }
        if (activeSeedColor != nextSeed) {
            activeSeedColor = nextSeed
            onSeedColorChanged()
        }
        val summary = nextSummaries.getValue(pc)
        val cleanTitle = summary.title
        val subtitleLine = summary.subtitle
        val picture = summary.artwork

        session.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, cleanTitle)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, subtitleLine)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, player.album)
                .apply { player.duration?.let { putLong(MediaMetadata.METADATA_KEY_DURATION, it.toLong()) } }
                .apply { picture?.let { putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) } }
                .build(),
        )
        var actions = 0L
        if (MediaAction.PLAY in player.actions) actions = actions or PlaybackState.ACTION_PLAY
        if (MediaAction.PAUSE in player.actions) actions = actions or PlaybackState.ACTION_PAUSE
        if (MediaAction.PLAY in player.actions && MediaAction.PAUSE in player.actions) {
            actions = actions or PlaybackState.ACTION_PLAY_PAUSE
        }
        if (MediaAction.NEXT in player.actions) actions = actions or PlaybackState.ACTION_SKIP_TO_NEXT
        if (MediaAction.PREVIOUS in player.actions) actions = actions or PlaybackState.ACTION_SKIP_TO_PREVIOUS
        if (MediaAction.SEEK in player.actions) actions = actions or PlaybackState.ACTION_SEEK_TO
        session.setPlaybackState(
            PlaybackState.Builder()
                .setActions(actions)
                .setState(
                    if (player.playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
                    player.position?.toLong() ?: PlaybackState.PLAYBACK_POSITION_UNKNOWN,
                    if (player.playing) 1f else 0f,
                    SystemClock.elapsedRealtime(),
                )
                .build(),
        )
        route(session)
        session.isActive = true
        post(notification(session, player, cleanTitle, subtitleLine, picture))
        if (!player.playing) {
            main.postDelayed(stalePauseTimeout, 600_000L)
        }
    }

    /**
     * Gives the session the shown PC's volume while that PC plays and its
     * volume is known, and the phone's media volume otherwise.
     */
    private fun route(session: MediaSession) {
        val (pc, player) = shown ?: return
        val level = levels[pc]?.takeIf { volumeKeys && player.playing }
        if (level == null) {
            if (remote != null) {
                remote = null
                session.setPlaybackToLocal(
                    AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build(),
                )
            }
            return
        }
        val current = remote
        if (current != null && current.pc == pc) {
            current.show(level)
            return
        }
        remote = PcVolume(pc, level).also(session::setPlaybackToRemote)
    }

    /** The PC [pc]'s master volume as Android's volume controls see it, in [VOLUME_STEPS]. */
    private inner class PcVolume(val pc: String, private var level: PcLevel) :
        VolumeProvider(VOLUME_CONTROL_ABSOLUTE, VOLUME_STEPS, toSteps(level.volume)) {

        fun show(level: PcLevel) {
            this.level = level
            currentVolume = toSteps(level.volume)
        }

        override fun onSetVolumeTo(volume: Int) {
            main.post { set(volume.coerceIn(0, VOLUME_STEPS), null) }
        }

        override fun onAdjustVolume(direction: Int) {
            main.post {
                when (direction) {
                    AudioManager.ADJUST_TOGGLE_MUTE -> set(null, !level.muted)
                    AudioManager.ADJUST_MUTE -> set(null, true)
                    AudioManager.ADJUST_UNMUTE -> set(null, false)
                    AudioManager.ADJUST_SAME -> {}
                    // Turning it up unmutes it, as the PC's own keys do.
                    else -> set(
                        (currentVolume + direction.coerceIn(-1, 1)).coerceIn(0, VOLUME_STEPS),
                        if (direction > 0 && level.muted) false else null,
                    )
                }
            }
        }

        private fun set(steps: Int?, muted: Boolean?) {
            if (remote !== this) return
            val volume = steps?.let { it * 100 / VOLUME_STEPS }
            show(PcLevel(volume ?: level.volume, muted ?: level.muted))
            setPcAudio(pc, volume, muted)
        }
    }

    private fun hide() {
        main.removeCallbacks(stalePauseTimeout)
        shown = null
        remote = null
        if (activeSeedColor != null) {
            activeSeedColor = null
            onSeedColorChanged()
        }
        session?.run {
            isActive = false
            release()
        }
        session = null
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_TAG, NOTIFICATION_ID)
    }

    private fun notification(
        session: MediaSession,
        player: MediaPlayer,
        cleanTitle: String,
        subtitleLine: String,
        picture: Bitmap?,
    ): Notification {
        val buttons = buildList {
            if (MediaAction.PREVIOUS in player.actions) add(button(R.drawable.ic_skip_previous, R.string.media_previous, MediaAction.PREVIOUS))
            if (player.playing && MediaAction.PAUSE in player.actions) add(button(R.drawable.ic_pause, R.string.media_pause, MediaAction.PAUSE))
            if (!player.playing && MediaAction.PLAY in player.actions) add(button(R.drawable.ic_play, R.string.media_play, MediaAction.PLAY))
            if (MediaAction.NEXT in player.actions) add(button(R.drawable.ic_skip_next, R.string.media_next, MediaAction.NEXT))
        }
        return Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(cleanTitle)
            .setContentText(subtitleLine)
            .setLargeIcon(picture)
            .setContentIntent(openApp())
            .setOngoing(player.playing)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .apply { buttons.forEach(::addAction) }
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(session.sessionToken)
                    .setShowActionsInCompactView(*buttons.indices.take(3).toIntArray()),
            )
            .build()
    }

    private fun button(icon: Int, label: Int, action: MediaAction): Notification.Action {
        val intent = Intent(context, CommandReceiver::class.java).putExtra(EXTRA_ACTION, action.name)
        val pending = PendingIntent.getBroadcast(
            context,
            action.ordinal,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Action.Builder(Icon.createWithResource(context, icon), context.getString(label), pending).build()
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        PendingIntent.FLAG_IMMUTABLE,
    )

    private fun post(notification: Notification) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        context.getSystemService(NotificationManager::class.java).notify(NOTIFICATION_TAG, NOTIFICATION_ID, notification)
    }

    /** The notification's buttons on Android 12 and earlier (later ones use the session), plus widget controls. */
    class CommandReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val action = intent.getStringExtra(EXTRA_ACTION)?.let { name -> MediaAction.entries.find { it.name == name } } ?: return
            (context.applicationContext as NectarlinkApplication).core.pcMedia.pressed(action)
        }
    }

    /** A button in the notification or widget. */
    internal fun pressed(action: MediaAction) = main.post { command(action) }

    companion object {
        private const val CHANNEL = "pc_media"
        private const val NOTIFICATION_TAG = "pc_media"
        private const val NOTIFICATION_ID = 1
        const val EXTRA_ACTION = "action"

        /** Steps of the PC's volume on the phone: each button press moves it 4%. */
        internal const val VOLUME_STEPS = 25

        /** A PC volume (`0..=100`) as the nearest step. */
        internal fun toSteps(volume: Int): Int = ((volume.coerceIn(0, 100) * VOLUME_STEPS + 50) / 100)

        /** Artwork is decoded to about this many pixels on its longer side, plenty for a notification. */
        private const val ART_PX = 512

        /** The power-of-two sample size that brings a `width` x `height` image down to about [ART_PX]. */
        internal fun artSampleSize(width: Int, height: Int): Int {
            var sample = 1
            while (maxOf(width, height) / (sample * 2) >= ART_PX) sample *= 2
            return sample
        }

        /**
         * Decodes the PC's artwork small, whatever size it claims: a small file can
         * describe a huge image, which decoded in full would run the app out of memory.
         */
        private fun decodeArtwork(bytes: ByteArray): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
            val options = BitmapFactory.Options().apply { inSampleSize = artSampleSize(bounds.outWidth, bounds.outHeight) }
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        }

        private fun isOpaqueHexOrAumid(s: String): Boolean {
            val trimmed = s.trim()
            if (trimmed.isEmpty()) return true
            if (trimmed.startsWith("_crx_", ignoreCase = true)) return true
            return trimmed.length >= 8 &&
                trimmed.any { it in '0'..'9' } &&
                trimmed.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' || it == '-' || it == '_' }
        }

        /**
         * Strips raw Windows AUMID hex hashes (like `F0DC299D809B9700`), GUIDs,
         * or package suffixes from application identifiers so Android notifications never show random hex strings.
         */
        internal fun sanitizeAppName(raw: String?): String {
            val trimmed = raw?.trim().orEmpty()
            if (trimmed.isEmpty()) return ""
            val head = trimmed.substringBefore('!').substringAfterLast('\\').substringAfterLast('/').trim()
            if (head.isEmpty()) return ""
            val lower = head.lowercase()
            if (lower.startsWith("308046b0af4a39cb") || lower.contains("firefox")) return "Firefox"
            if (isOpaqueHexOrAumid(head)) return ""
            return when {
                lower.contains("chrome") -> "Chrome"
                lower.contains("msedge") || lower == "edge" -> "Edge"
                lower.contains("brave") -> "Brave"
                lower.contains("vivaldi") -> "Vivaldi"
                lower.contains("opera") -> "Opera"
                lower.contains("spotify") -> "Spotify"
                else -> head.removeSuffix(".exe").removeSuffix(".EXE")
            }
        }

        /**
         * Strips raw Windows AUMID hex hashes or GUIDs that sometimes leak into `artist`
         * without corrupting legitimate artist names like `"AC/DC"`, `"Panic! At The Disco"`, or `"Chromeo"`.
         */
        internal fun sanitizeArtistLabel(raw: String?): String {
            val trimmed = raw?.trim().orEmpty()
            if (trimmed.isEmpty()) return ""
            val lower = trimmed.lowercase()
            if (lower.startsWith("308046b0af4a39cb")) return ""
            if (isOpaqueHexOrAumid(trimmed)) return ""
            return trimmed
        }

        fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_pc_media), NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }
}
