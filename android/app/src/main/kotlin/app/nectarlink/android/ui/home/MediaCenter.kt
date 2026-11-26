// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioManager
import android.media.MediaRouter2
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.media.PcMedia
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.core.DeckState
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MediaPlayer
import kotlinx.coroutines.delay

/**
 * One place for everything that plays (#536): every PC's players and this
 * phone's own, a page each to swipe through, with play and pause, the
 * previous and next track, the position, and the volume and output of
 * where it plays.
 */
internal sealed interface MediaEntry {
    val player: MediaPlayer
    val atElapsedMs: Long
    val key: String

    data class OnPc(val pc: PcMedia.Player, val deckState: DeckState?) : MediaEntry {
        override val player get() = pc.player
        override val atElapsedMs get() = pc.atElapsedMs
        override val key get() = "pc/${pc.pc}/${pc.player.id}"
    }

    data class OnPhone(override val player: MediaPlayer, override val atElapsedMs: Long) : MediaEntry {
        override val key get() = "phone/${player.id}"
    }
}

/** What the media center shows, playing players first, the phone's after a PC's at the same. */
internal fun mediaEntries(
    pc: List<PcMedia.Player>,
    phone: PhoneMedia.Players,
    deckStates: Map<String, DeckState?>,
): List<MediaEntry> =
    (pc.map { MediaEntry.OnPc(it, deckStates[it.pc]) } + phone.players.map { MediaEntry.OnPhone(it, phone.atElapsedMs) })
        .sortedByDescending { it.player.playing }

@Composable
internal fun MediaCenterCard(
    entries: List<MediaEntry>,
    onPcCommand: (pc: String, player: String, action: MediaAction, position: ULong?) -> Unit,
    onPhoneCommand: (player: String, action: MediaAction, position: ULong?) -> Unit,
    onSetPcAudio: (pc: String, volume: Int?, muted: Boolean?) -> Unit,
    onPickPcOutput: (pc: String, output: String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (entries.isEmpty()) return
    val pager = rememberPagerState(pageCount = { entries.size })
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    stringResource(R.string.media_center_title),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                if (entries.size > 1) {
                    val pageText = stringResource(R.string.media_center_page, pager.currentPage + 1, entries.size)
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        modifier = Modifier.semantics { contentDescription = pageText },
                    ) {
                        repeat(entries.size) { i ->
                            Box(
                                Modifier
                                    .size(if (i == pager.currentPage) 8.dp else 6.dp)
                                    .clip(CircleShape)
                                    .background(
                                        if (i == pager.currentPage) {
                                            MaterialTheme.colorScheme.primary
                                        } else {
                                            MaterialTheme.colorScheme.outlineVariant
                                        },
                                    ),
                            )
                        }
                    }
                }
            }
            HorizontalPager(state = pager, key = { entries[it].key }, pageSpacing = 12.dp) { page ->
                val entry = entries[page]
                MediaPage(
                    entry = entry,
                    onCommand = { action, position ->
                        when (entry) {
                            is MediaEntry.OnPc -> onPcCommand(entry.pc.pc, entry.player.id, action, position)
                            is MediaEntry.OnPhone -> onPhoneCommand(entry.player.id, action, position)
                        }
                    },
                    onSetPcAudio = onSetPcAudio,
                    onPickPcOutput = onPickPcOutput,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )
            }
        }
    }
}

@Composable
private fun MediaPage(
    entry: MediaEntry,
    onCommand: (MediaAction, ULong?) -> Unit,
    onSetPcAudio: (pc: String, volume: Int?, muted: Boolean?) -> Unit,
    onPickPcOutput: (pc: String, output: String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val player = entry.player
    val artwork: Bitmap? = when (entry) {
        is MediaEntry.OnPc -> entry.pc.artwork
        is MediaEntry.OnPhone -> remember(player.artKey) {
            player.art?.let { runCatching { BitmapFactory.decodeByteArray(it, 0, it.size) }.getOrNull() }
        }
    }
    val where = when (entry) {
        is MediaEntry.OnPc -> entry.pc.pcName
        is MediaEntry.OnPhone -> stringResource(R.string.media_center_on_phone)
    }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier
                    .size(64.dp)
                    .clip(RoundedCornerShape(14.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                contentAlignment = Alignment.Center,
            ) {
                if (artwork != null) {
                    Image(
                        artwork.asImageBitmap(),
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.size(64.dp),
                    )
                } else {
                    Icon(
                        painterResource(R.drawable.ic_music),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Column(Modifier.weight(1f)) {
                Text(player.title ?: player.app, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                player.artist?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    stringResource(R.string.media_on, player.app, where),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Position(player, entry.atElapsedMs, onSeek = { onCommand(MediaAction.SEEK, it.toULong()) })
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(
                onClick = { onCommand(MediaAction.PREVIOUS, null) },
                enabled = MediaAction.PREVIOUS in player.actions,
            ) {
                Icon(painterResource(R.drawable.ic_skip_previous), stringResource(R.string.media_previous))
            }
            val playing = player.playing
            FilledTonalIconButton(
                onClick = { onCommand(if (playing) MediaAction.PAUSE else MediaAction.PLAY, null) },
                enabled = (if (playing) MediaAction.PAUSE else MediaAction.PLAY) in player.actions,
                modifier = Modifier.size(52.dp),
            ) {
                Icon(
                    painterResource(if (playing) R.drawable.ic_pause else R.drawable.ic_play),
                    stringResource(if (playing) R.string.media_pause else R.string.media_play),
                )
            }
            IconButton(
                onClick = { onCommand(MediaAction.NEXT, null) },
                enabled = MediaAction.NEXT in player.actions,
            ) {
                Icon(painterResource(R.drawable.ic_skip_next), stringResource(R.string.media_next))
            }
        }
        when (entry) {
            is MediaEntry.OnPc -> PcVolume(entry, onSetPcAudio, onPickPcOutput)
            is MediaEntry.OnPhone -> PhoneVolume()
        }
    }
}

/** Where it is in the track, moving while it plays, and a seek bar where the player takes it. */
@Composable
private fun Position(player: MediaPlayer, atElapsedMs: Long, onSeek: (Long) -> Unit) {
    val duration = player.duration?.toLong()?.takeIf { it > 0 } ?: return
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(player.playing, atElapsedMs) {
        while (player.playing) {
            now = SystemClock.elapsedRealtime()
            delay(500)
        }
    }
    val base = player.position?.toLong() ?: 0L
    val position = (base + if (player.playing) (now - atElapsedMs).coerceAtLeast(0) else 0).coerceIn(0, duration)
    var dragging by remember { mutableStateOf<Float?>(null) }
    val label = stringResource(R.string.media_seek)
    Column {
        Slider(
            value = dragging ?: (position.toFloat() / duration),
            onValueChange = { dragging = it },
            onValueChangeFinished = {
                dragging?.let { onSeek((it * duration).toLong()) }
                dragging = null
            },
            enabled = MediaAction.SEEK in player.actions,
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
        )
        Row(Modifier.fillMaxWidth()) {
            Text(
                clock(dragging?.let { (it * duration).toLong() } ?: position),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            Text(clock(duration), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private fun clock(ms: Long): String {
    val s = ms / 1000
    return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s % 3600 / 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
}

/** The PC's volume and output, as on its card. */
@Composable
private fun PcVolume(
    entry: MediaEntry.OnPc,
    onSetPcAudio: (pc: String, volume: Int?, muted: Boolean?) -> Unit,
    onPickPcOutput: (pc: String, output: String) -> Unit,
) {
    val state = entry.deckState ?: return
    var dragging by remember { mutableStateOf<Float?>(null) }
    var showOutputs by remember { mutableStateOf(false) }
    val label = stringResource(R.string.pc_audio_volume)
    Slider(
        value = dragging ?: state.volume.toFloat(),
        onValueChange = { dragging = it },
        onValueChangeFinished = {
            dragging?.let { onSetPcAudio(entry.pc.pc, it.toInt(), null) }
            dragging = null
        },
        valueRange = 0f..100f,
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
    )
    val outputs = state.outputDevices.filterNot { it.id == "nectarlink:phone_speaker" }
    val current = outputs.firstOrNull { it.isDefault } ?: outputs.firstOrNull()
    if (current != null) {
        TextButton(onClick = { showOutputs = true }, enabled = outputs.size > 1) {
            Text(stringResource(R.string.pc_audio_output, current.name), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
    if (showOutputs) {
        PcAudioOutputsDialog(
            pcName = entry.pc.pcName,
            devices = outputs,
            onPick = {
                onPickPcOutput(entry.pc.pc, it)
                showOutputs = false
            },
            onDismiss = { showOutputs = false },
        )
    }
}

/** This phone's media volume, and Android's own choice of where it plays. */
@Composable
private fun PhoneVolume() {
    val context = LocalContext.current
    val audio = remember { context.getSystemService(AudioManager::class.java) }
    val max = remember { audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1) }
    var volume by remember { mutableFloatStateOf(audio.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat()) }
    // Picks up the volume keys and other apps between recompositions.
    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000)
            volume = audio.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat()
        }
    }
    val label = stringResource(R.string.media_center_phone_volume)
    Slider(
        value = volume,
        onValueChange = {
            volume = it
            runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, it.toInt(), 0) }
        },
        valueRange = 0f..max.toFloat(),
        steps = (max - 1).coerceAtLeast(0),
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
    )
    if (canPickOutput()) {
        TextButton(onClick = { pickOutput(context) }) { Text(stringResource(R.string.media_center_output)) }
    }
}

private fun canPickOutput(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q

/** Android's own output switcher (14+), or its volume panel, which has it on earlier versions. */
private fun pickOutput(context: Context) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE &&
        runCatching { MediaRouter2.getInstance(context).showSystemOutputSwitcher() }.getOrDefault(false)
    ) {
        return
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        runCatching {
            context.startActivity(Intent(Settings.Panel.ACTION_VOLUME).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}
