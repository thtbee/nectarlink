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
import android.media.MediaMetadata
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

/**
 * What plays on a paired PC, in Android's own media controls (the
 * notification shade and Quick Settings player): a media session that
 * stands for the PC's player, and its notification. What the user presses
 * goes to the PC through [send]; the PC's new state comes back as an update.
 *
 * Everything runs on the main thread.
 */
class PcMedia(
    context: Context,
    private val send: (pc: String, player: String, action: MediaAction, position: ULong?) -> Unit,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())

    /** Players per PC, in the PC's order, and the PCs' names. */
    private val players = linkedMapOf<String, List<MediaPlayer>>()
    private val names = mutableMapOf<String, String>()
    /** Decoded artwork by PC and key (artwork arrives once per key). */
    private val art = object : LinkedHashMap<String, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>) = size > 8
    }

    private var session: MediaSession? = null
    private var shown: Pair<String, MediaPlayer>? = null

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = command(MediaAction.PLAY)
        override fun onPause() = command(MediaAction.PAUSE)
        override fun onSkipToNext() = command(MediaAction.NEXT)
        override fun onSkipToPrevious() = command(MediaAction.PREVIOUS)
        override fun onSeekTo(pos: Long) = command(MediaAction.SEEK, pos.coerceAtLeast(0).toULong())
    }

    /** A PC's players changed (empty: nothing plays there, or it's offline). */
    fun update(pc: String, pcName: String, list: List<MediaPlayer>) = main.post {
        names[pc] = pcName
        for (player in list) {
            val key = player.artKey ?: continue
            val bytes = player.art ?: continue
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.let { art["$pc/$key"] = it }
        }
        if (list.isEmpty()) players.remove(pc) else players[pc] = list
        render()
    }

    private fun command(action: MediaAction, position: ULong? = null) {
        val (pc, player) = shown ?: return
        send(pc, player.id, action, position)
    }

    private fun render() {
        val first = players.entries.firstNotNullOfOrNull { (pc, list) -> list.firstOrNull()?.let { pc to it } }
        if (first == null) return hide()
        val (pc, player) = first
        shown = first
        val session = session ?: MediaSession(context, "Nectarlink PC media").also {
            it.setCallback(callback, main)
            it.setSessionActivity(openApp())
            session = it
        }
        val picture = player.artKey?.let { art["$pc/$it"] }
        val where = context.getString(R.string.media_on, player.app, names[pc].orEmpty())
        session.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, player.title ?: player.app)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, player.artist ?: where)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, player.album)
                .putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, where)
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
        session.isActive = true
        post(notification(session, player, where, picture))
    }

    private fun hide() {
        shown = null
        session?.run {
            isActive = false
            release()
        }
        session = null
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_TAG, NOTIFICATION_ID)
    }

    private fun notification(session: MediaSession, player: MediaPlayer, where: String, picture: Bitmap?): Notification {
        val buttons = buildList {
            if (MediaAction.PREVIOUS in player.actions) add(button(R.drawable.ic_skip_previous, R.string.media_previous, MediaAction.PREVIOUS))
            if (player.playing && MediaAction.PAUSE in player.actions) add(button(R.drawable.ic_pause, R.string.media_pause, MediaAction.PAUSE))
            if (!player.playing && MediaAction.PLAY in player.actions) add(button(R.drawable.ic_play, R.string.media_play, MediaAction.PLAY))
            if (MediaAction.NEXT in player.actions) add(button(R.drawable.ic_skip_next, R.string.media_next, MediaAction.NEXT))
        }
        return Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(player.title ?: player.app)
            .setContentText(listOfNotNull(player.artist, where).joinToString(" · "))
            .setLargeIcon(picture)
            .setContentIntent(openApp())
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

    /** The notification's buttons on Android 12 and earlier (later ones use the session). */
    class CommandReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val action = intent.getStringExtra(EXTRA_ACTION)?.let { name -> MediaAction.entries.find { it.name == name } } ?: return
            (context.applicationContext as NectarlinkApplication).core.pcMedia.pressed(action)
        }
    }

    /** A button in the notification. */
    internal fun pressed(action: MediaAction) = main.post { command(action) }

    companion object {
        private const val CHANNEL = "pc_media"
        private const val NOTIFICATION_TAG = "pc_media"
        private const val NOTIFICATION_ID = 1
        private const val EXTRA_ACTION = "action"

        fun createChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.channel_pc_media), NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }
}
