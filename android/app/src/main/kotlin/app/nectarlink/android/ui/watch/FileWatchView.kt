// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.watch

import android.media.AudioAttributes
import android.media.MediaPlayer
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import app.nectarlink.android.R
import app.nectarlink.android.media.PcFileSource
import app.nectarlink.core.WatchFileOffer

/** A video file playing on the phone (Play on phone): whether it plays, and play or pause. */
@Stable
class LocalPlayback {
    var playing by mutableStateOf(false)
        internal set

    internal var player: MediaPlayer? = null

    /** Where to go on from when the picture comes back (after the app was in the background). */
    internal var resumeAtMs: Long? = null

    /** Whether the PC's player was paused for this already. */
    internal var tookOver = false

    fun toggle() {
        val player = player ?: return
        runCatching { if (player.isPlaying) player.pause() else player.start() }
        playing = runCatching { player.isPlaying }.getOrDefault(false)
    }

    /** Where it is, in milliseconds. */
    fun positionMs(): Long =
        player?.let { runCatching { it.currentPosition.toLong() }.getOrNull() } ?: resumeAtMs ?: 0

    /** How long it is, in milliseconds (0 until it's known). */
    fun durationMs(): Long =
        player?.let { runCatching { it.duration.toLong() }.getOrNull() }?.takeIf { it > 0 } ?: 0

    fun seekTo(positionMs: Long) {
        val player = player ?: return
        runCatching { player.seekTo(positionMs, MediaPlayer.SEEK_CLOSEST) }
    }
}

/**
 * Plays [file], read from the PC with [read], full screen and letterboxed, from where the PC
 * had got to. [onStarted] runs once, when it first plays.
 */
@Composable
fun FileWatchView(
    file: WatchFileOffer,
    read: (offset: Long, length: Long) -> ByteArray,
    playback: LocalPlayback,
    onStarted: () -> Unit,
    onTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var surface by remember { mutableStateOf<Surface?>(null) }
    var videoSize by remember { mutableStateOf<IntSize?>(null) }
    var failed by remember { mutableStateOf(false) }
    val started by rememberUpdatedState(onStarted)
    val reader by rememberUpdatedState(read)

    // A player for as long as there is a picture to draw on.
    DisposableEffect(surface) {
        val target = surface ?: return@DisposableEffect onDispose {}
        val source = PcFileSource(file.size.toLong()) { offset, length -> reader(offset, length) }
        val player = MediaPlayer()
        player.setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                .build(),
        )
        player.setSurface(target)
        player.setOnVideoSizeChangedListener { _, width, height ->
            if (width > 0 && height > 0) videoSize = IntSize(width, height)
        }
        player.setOnErrorListener { _, what, extra ->
            Log.w(TAG, "can't play the PC's file: $what/$extra")
            failed = true
            playback.playing = false
            true
        }
        player.setOnCompletionListener { playback.playing = false }
        player.setOnPreparedListener { prepared ->
            playback.player = prepared
            val at = playback.resumeAtMs ?: file.positionMs.toLong()
            if (at > 0) prepared.seekTo(at, MediaPlayer.SEEK_CLOSEST)
            prepared.start()
            playback.playing = true
            if (!playback.tookOver) {
                playback.tookOver = true
                started()
            }
        }
        runCatching {
            player.setDataSource(source)
            player.prepareAsync()
        }.onFailure {
            Log.w(TAG, "can't open the PC's file", it)
            failed = true
        }
        onDispose {
            val prepared = playback.player === player
            if (prepared) {
                playback.resumeAtMs = runCatching { player.currentPosition.toLong() }.getOrNull()?.takeIf { it > 0 }
                    ?: playback.resumeAtMs
                playback.player = null
            }
            playback.playing = false
            // Close the network source first so an in-flight chunk read unblocks immediately
            // instead of stalling MediaPlayer.release() on the main thread.
            source.close()
            if (prepared) {
                runCatching { player.stop() }
            }
            player.release()
        }
    }

    Box(modifier, contentAlignment = Alignment.Center) {
        val ratio = videoSize?.let { it.width.toFloat() / it.height }
        AndroidView(
            factory = { context ->
                SurfaceView(context).apply {
                    keepScreenOn = true
                    holder.addCallback(
                        object : SurfaceHolder.Callback {
                            override fun surfaceCreated(holder: SurfaceHolder) {
                                surface = holder.surface
                            }

                            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}

                            override fun surfaceDestroyed(holder: SurfaceHolder) {
                                surface = null
                            }
                        },
                    )
                }
            },
            modifier = if (ratio != null) Modifier.aspectRatio(ratio) else Modifier.fillMaxSize(),
        )
        // Taps over the picture show and hide the controls.
        Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { onTap() } })
        if (failed) {
            Text(
                text = stringResource(R.string.watch_file_failed),
                style = MaterialTheme.typography.bodyLarge,
                color = Color.White,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(24.dp),
            )
        }
    }
}

private const val TAG = "watch-file"
