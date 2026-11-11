// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.desk

import android.app.Activity
import android.text.format.DateFormat
import android.view.WindowManager
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLocale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.remote.RemoteMode
import app.nectarlink.core.Link
import app.nectarlink.core.MediaAction
import java.text.SimpleDateFormat
import java.util.Date
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Full-screen ambient Desk Mode dock (`docs/PLAN.md` §3.13 / §8.6 3.4):
 * keeps the display awake with subtle 1-minute OLED burn-in pixel shift,
 * large tabular clock & date, live PC song title/artist/artwork & audio controls,
 * and a balanced 2×2 quick-action dock.
 */
@Composable
fun DeskScreen(
    device: Device,
    core: Core,
    onOpenRemote: (RemoteMode) -> Unit,
    onOpenDeck: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    DisposableEffect(Unit) {
        val window = (context as? Activity)?.window
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val controller = window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller?.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    var minuteBucket by remember { mutableLongStateOf(System.currentTimeMillis() / 60_000L) }
    LaunchedEffect(Unit) {
        while (true) {
            val now = System.currentTimeMillis()
            minuteBucket = now / 60_000L
            delay((60_000L - (now % 60_000L)).coerceIn(1_000L, 60_000L))
        }
    }

    // Subtle 1-minute OLED burn-in protection pixel shift (±8 px X, ±6 px Y) for the ambient clock/card.
    val minutePhase = (minuteBucket % 12L).toDouble() * (Math.PI / 6.0)
    val shiftX = (sin(minutePhase) * 8.0).roundToInt()
    val shiftY = (cos(minutePhase) * 6.0).roundToInt()

    val deckState = device.deckState
    val summaries by core.pcMedia.summaries.collectAsStateWithLifecycle()
    val activeSummary = summaries[device.id]
    val artBitmap = remember(activeSummary?.artwork) { activeSummary?.artwork?.asImageBitmap() }

    var draggingVolume by remember { mutableStateOf<Float?>(null) }
    var lastSentVolInt by remember { mutableIntStateOf(-1) }
    val scope = rememberCoroutineScope()

    Surface(
        color = Color.Black,
        contentColor = Color(0xFFEAECEF),
        modifier = modifier.fillMaxSize(),
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 20.dp, vertical = 20.dp),
        ) {
            // Top bar: PC connection status + Exit button
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .align(Alignment.TopCenter),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Surface(
                    shape = CircleShape,
                    color = if (device.online) Color(0xFF0E2217) else Color(0xFF16181E),
                    border = BorderStroke(
                        1.dp,
                        if (device.online) Color(0xFF2EA05E) else Color(0xFF2B2F3A),
                    ),
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        val dotColor = if (device.online) Color(0xFF4ADE80) else Color(0xFF9CA3AF)
                        Spacer(
                            modifier = Modifier
                                .size(8.dp)
                                .drawBehind { drawCircle(dotColor) },
                        )
                        val linkDetail = when (val l = device.link) {
                            is Link.Online -> "${device.name} · ${l.rttMs} ms"
                            else -> "${device.name} · Offline"
                        }
                        Text(
                            text = linkDetail,
                            style = MaterialTheme.typography.labelLarge,
                            color = if (device.online) Color(0xFFB8F5CD) else Color(0xFFD1D5DB),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                OutlinedButton(
                    onClick = onBack,
                    border = BorderStroke(1.dp, Color(0xFF282C37)),
                ) {
                    Text(stringResource(R.string.desk_exit), color = Color(0xFFD1D5DB))
                }
            }

            // Center: Ambient Clock + Date + PC Media / Volume Card (with OLED burn-in drift)
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .align(Alignment.Center)
                    .offset { IntOffset(shiftX, shiftY) },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                DeskClockDisplay()

                Spacer(Modifier.height(2.dp))

                // PC Media & Audio Dock Card
                if (device.online && (deckState != null || activeSummary != null)) {
                    val isPlaying = activeSummary?.playing ?: (deckState?.playing == true)
                    val outName = deckState?.outputDevices?.firstOrNull { it.isDefault }?.name
                        ?: deckState?.outputDevices?.firstOrNull()?.name
                        ?: device.name
                    val songTitle = activeSummary?.title?.takeIf { it.isNotBlank() }
                        ?: if (isPlaying) stringResource(R.string.desk_media_playing) else stringResource(R.string.desk_media_audio)
                    val subtitleText = buildString {
                        val sub = activeSummary?.subtitle?.trim().orEmpty()
                        if (sub.isNotEmpty()) {
                            append(sub)
                            if (outName.isNotBlank() && !sub.contains(outName, ignoreCase = true)) {
                                append(" · ")
                                append(outName)
                            }
                        } else {
                            append(outName)
                        }
                    }

                    Surface(
                        shape = RoundedCornerShape(26.dp),
                        color = Color(0xFF0E1015),
                        border = BorderStroke(1.dp, Color(0xFF222631)),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                // Album art or music badge
                                if (artBitmap != null) {
                                    Image(
                                        bitmap = artBitmap,
                                        contentDescription = null,
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier
                                            .size(48.dp)
                                            .clip(RoundedCornerShape(12.dp)),
                                    )
                                } else {
                                    Box(
                                        modifier = Modifier
                                            .size(48.dp)
                                            .clip(RoundedCornerShape(12.dp))
                                            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.16f)),
                                        contentAlignment = Alignment.Center,
                                    ) {
                                        Icon(
                                            painter = painterResource(R.drawable.ic_music),
                                            contentDescription = null,
                                            tint = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                }

                                Column(
                                    modifier = Modifier.weight(1f),
                                    verticalArrangement = Arrangement.spacedBy(2.dp),
                                ) {
                                    Text(
                                        text = songTitle,
                                        style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold),
                                        color = if (activeSummary != null) Color(0xFFF3F4F6) else MaterialTheme.colorScheme.primary,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        text = subtitleText,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = Color(0xFF9CA3AF),
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }

                                Row(
                                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.command(device.id, activeSummary.playerId, MediaAction.PREVIOUS)
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_previous") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = Color(0xFF1B1F2A),
                                            contentColor = Color(0xFFE5E7EB),
                                        ),
                                        modifier = Modifier.size(38.dp),
                                    ) {
                                        Icon(
                                            painterResource(R.drawable.ic_skip_previous),
                                            contentDescription = stringResource(R.string.media_previous),
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.command(
                                                    device.id,
                                                    activeSummary.playerId,
                                                    if (isPlaying) MediaAction.PAUSE else MediaAction.PLAY,
                                                )
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_play_pause") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = MaterialTheme.colorScheme.primaryContainer,
                                            contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                                        ),
                                        modifier = Modifier.size(42.dp),
                                    ) {
                                        Icon(
                                            painterResource(
                                                if (isPlaying) R.drawable.ic_pause else R.drawable.ic_play,
                                            ),
                                            contentDescription = stringResource(R.string.deck_default_play_pause),
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.command(device.id, activeSummary.playerId, MediaAction.NEXT)
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_next") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = Color(0xFF1B1F2A),
                                            contentColor = Color(0xFFE5E7EB),
                                        ),
                                        modifier = Modifier.size(38.dp),
                                    ) {
                                        Icon(
                                            painterResource(R.drawable.ic_skip_next),
                                            contentDescription = stringResource(R.string.media_next),
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                }
                            }

                            // Volume slider row
                            if (deckState != null) {
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    IconButton(
                                        onClick = { core.setPcAudio(device.id, null, !deckState.muted) },
                                        modifier = Modifier.size(32.dp),
                                    ) {
                                        Icon(
                                            painterResource(
                                                if (deckState.muted) R.drawable.ic_sound_off else R.drawable.ic_speaker,
                                            ),
                                            contentDescription = null,
                                            tint = Color(0xFFD1D5DB),
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                    val shownVol = (draggingVolume ?: deckState.volume.toFloat()).coerceIn(0f, 100f)
                                    Slider(
                                        value = shownVol,
                                        onValueChange = { v ->
                                            draggingVolume = v
                                            val rounded = v.roundToInt()
                                            if (rounded != lastSentVolInt) {
                                                lastSentVolInt = rounded
                                                core.setPcAudio(device.id, rounded, null)
                                            }
                                        },
                                        onValueChangeFinished = {
                                            draggingVolume = null
                                            lastSentVolInt = -1
                                        },
                                        valueRange = 0f..100f,
                                        colors = SliderDefaults.colors(
                                            thumbColor = MaterialTheme.colorScheme.primary,
                                            activeTrackColor = MaterialTheme.colorScheme.primary,
                                            inactiveTrackColor = Color(0xFF252A36),
                                        ),
                                        modifier = Modifier.weight(1f),
                                    )
                                    Text(
                                        text = if (deckState.muted) "Muted" else "${shownVol.roundToInt()}%",
                                        style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                                        color = Color(0xFFD1D5DB),
                                        modifier = Modifier.width(44.dp),
                                    )
                                }
                            }
                        }
                    }
                } else if (!device.online && device.canWake) {
                    FilledTonalButton(onClick = { core.wake(device.id) }) {
                        Text(stringResource(R.string.action_wake_pc))
                    }
                }
            }

            // Bottom Dock Bar: Symmetric 2×2 Grid of Equal-Width Quick Action Pills
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .align(Alignment.BottomCenter),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    DeskActionButton(
                        label = stringResource(R.string.desk_pc_screen),
                        enabled = device.online,
                        onClick = { onOpenRemote(RemoteMode.Screen) },
                        modifier = Modifier.weight(1f),
                    )
                    DeskActionButton(
                        label = stringResource(R.string.action_remote_pc),
                        enabled = device.online,
                        onClick = { onOpenRemote(RemoteMode.Touchpad) },
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    DeskActionButton(
                        label = stringResource(R.string.action_deck_pc),
                        enabled = device.online,
                        onClick = onOpenDeck,
                        modifier = Modifier.weight(1f),
                    )
                    DeskActionButton(
                        label = stringResource(R.string.action_lock_pc),
                        enabled = device.online,
                        isSecondary = true,
                        onClick = { core.pcPower(device.id, sleep = false) },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun DeskActionButton(
    label: String,
    enabled: Boolean,
    isSecondary: Boolean = false,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    FilledTonalButton(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(18.dp),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = if (isSecondary) Color(0xFF12141B) else Color(0xFF171B24),
            contentColor = if (isSecondary) Color(0xFFD1D5DB) else Color(0xFFF3F4F6),
            disabledContainerColor = Color(0xFF0C0E12),
            disabledContentColor = Color(0xFF4B5563),
        ),
        border = BorderStroke(
            1.dp,
            if (isSecondary) Color(0xFF262A36) else Color(0xFF2B3140),
        ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 13.dp),
        modifier = modifier,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun DeskClockDisplay() {
    var nowMs by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            val current = System.currentTimeMillis()
            nowMs = current
            delay((1_000L - (current % 1_000L)).coerceIn(100L, 1_000L))
        }
    }

    val locale = LocalLocale.current.platformLocale
    val timeFormat = remember(locale) { SimpleDateFormat("HH:mm", locale) }
    val secFormat = remember(locale) { SimpleDateFormat(":ss", locale) }
    val dateFormat = remember(locale) {
        val pattern = DateFormat.getBestDateTimePattern(locale, "EEEEMMMMd")
        SimpleDateFormat(pattern, locale)
    }

    val minuteKey = nowMs / 60_000L
    val secondKey = nowMs / 1_000L
    val timeText = remember(minuteKey, locale) { timeFormat.format(Date(minuteKey * 60_000L)) }
    val dateText = remember(minuteKey, locale) { dateFormat.format(Date(minuteKey * 60_000L)) }
    val secText = remember(secondKey, locale) { secFormat.format(Date(secondKey * 1_000L)) }

    Row(
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.Center,
    ) {
        Text(
            text = timeText,
            fontSize = 84.sp,
            lineHeight = 88.sp,
            fontWeight = FontWeight.Light,
            fontFamily = FontFamily.Monospace,
            color = Color(0xFFF3F4F6),
        )
        Text(
            text = secText,
            fontSize = 28.sp,
            fontWeight = FontWeight.Normal,
            fontFamily = FontFamily.Monospace,
            color = Color(0xFF9CA3AF),
            modifier = Modifier.padding(bottom = 12.dp, start = 4.dp),
        )
    }

    Text(
        text = dateText,
        style = MaterialTheme.typography.titleMedium,
        color = Color(0xFF9CA3AF),
    )
}


