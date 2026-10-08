// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.deck

import android.app.Activity
import android.content.res.Configuration
import android.view.HapticFeedbackConstants
import android.view.WindowManager
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.core.DeckLayout
import app.nectarlink.core.DeckPage
import app.nectarlink.core.DeckState
import app.nectarlink.core.DeckTile
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun DeckScreen(
    device: Device,
    core: Core,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val view = LocalView.current
    val context = LocalContext.current
    val configuration = LocalConfiguration.current
    val isLandscape = configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
    val scope = rememberCoroutineScope()

    var fullScreen by rememberSaveable { mutableStateOf(false) }
    var access by remember(device.id) { mutableStateOf(Core.RemoteAccess.Allowed) }
    var commandsDenied by remember(device.id) { mutableStateOf(false) }

    // Keep the phone's screen on while Deck is open, and restore system bars on exit.
    DisposableEffect(view, device.id) {
        val window = (context as? Activity)?.window
        val prevKeepOn = view.keepScreenOn
        view.keepScreenOn = true
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            view.keepScreenOn = prevKeepOn
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            if (window != null) {
                WindowCompat.getInsetsController(window, view)
                    .show(WindowInsetsCompat.Type.systemBars())
            }
        }
    }

    LaunchedEffect(fullScreen, view) {
        val window = (context as? Activity)?.window ?: return@LaunchedEffect
        val controller = WindowCompat.getInsetsController(window, view)
        if (fullScreen) {
            controller.systemBarsBehavior =
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    // Check remote_input access on entry and poll while denied so the banner clears
    // as soon as the user clicks Allow on the PC.
    LaunchedEffect(device.id, device.online) {
        if (!device.online) {
            access = Core.RemoteAccess.Offline
            return@LaunchedEffect
        }
        access = core.remoteCheck(device.id)
        while (access == Core.RemoteAccess.Denied && device.online) {
            delay(1_500)
            access = core.remoteCheck(device.id)
        }
    }

    val layout = device.deckLayout ?: remember { fallbackDefaultLayout() }
    val liveState = device.deckState ?: DeckState(
        playing = false,
        volume = 50u,
        muted = false,
        micMuted = false,
    )
    val pages = layout.pages.ifEmpty { fallbackDefaultLayout().pages }
    val pagerState = rememberPagerState(pageCount = { pages.size })

    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(
                horizontal = 16.dp,
                vertical = if (fullScreen || isLandscape) 8.dp else 12.dp,
            ),
        verticalArrangement = Arrangement.spacedBy(if (isLandscape) 8.dp else 12.dp),
    ) {
        // Header row: Back, PC name + status dot, Full-screen toggle
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(
                    painter = painterResource(R.drawable.ic_back),
                    contentDescription = stringResource(R.string.action_back),
                )
            }
            Spacer(Modifier.width(4.dp))
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        device.name,
                        style = if (isLandscape) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.width(8.dp))
                    Surface(
                        shape = CircleShape,
                        color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                        modifier = Modifier.size(8.dp),
                    ) {}
                }
                if (!fullScreen && !isLandscape) {
                    Text(
                        stringResource(R.string.deck_hint, device.name),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            OutlinedButton(
                onClick = { fullScreen = !fullScreen },
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(
                    stringResource(if (fullScreen) R.string.deck_exit_fullscreen else R.string.deck_fullscreen),
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        }

        // Status / permission banners
        if (!device.online) {
            DeckBanner(
                title = stringResource(R.string.remote_offline_text, device.name),
                text = null,
            )
        } else if (access == Core.RemoteAccess.Denied) {
            DeckBanner(
                title = stringResource(R.string.remote_denied_title, device.name),
                text = stringResource(R.string.remote_denied_text, device.name),
            )
        } else if (commandsDenied) {
            DeckBanner(
                title = stringResource(R.string.deck_commands_denied_title, device.name),
                text = stringResource(R.string.deck_commands_denied_text, device.name),
                onDismiss = { commandsDenied = false },
            )
        }

        // Page tabs when multiple pages exist
        if (pages.size > 1) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                pages.forEachIndexed { index, page ->
                    FilterChip(
                        selected = pagerState.currentPage == index,
                        onClick = {
                            scope.launch { pagerState.animateScrollToPage(index) }
                        },
                        label = {
                            Text(
                                page.name,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        },
                    )
                }
            }
        }

        // Swipable pages of tactile Deck tiles
        HorizontalPager(
            state = pagerState,
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
        ) { pageIndex ->
            val page = pages.getOrNull(pageIndex) ?: pages.first()
            if (page.tiles.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        stringResource(R.string.deck_empty_page, device.name),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialResource.onSurfaceVariant(),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(24.dp),
                    )
                }
            } else {
                val largeFont = LocalConfiguration.current.fontScale >= 1.2f
                val columns = when {
                    isLandscape -> if (largeFont) 3 else 4
                    largeFont -> 2
                    else -> 3
                }
                LazyVerticalGrid(
                    columns = GridCells.Fixed(columns),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(page.tiles, key = { it.id }) { tile ->
                        DeckTileButton(
                            tile = tile,
                            liveState = liveState,
                            compact = isLandscape,
                            enabled = device.online,
                            onPress = {
                                view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
                                scope.launch {
                                    when (core.deckPress(device.id, tile.id)) {
                                        Core.DeckPressResult.Ok -> {
                                            access = Core.RemoteAccess.Allowed
                                            if (tile.kind == "run_command") commandsDenied = false
                                        }
                                        Core.DeckPressResult.Denied -> {
                                            val chk = core.remoteCheck(device.id)
                                            access = chk
                                            if (chk == Core.RemoteAccess.Allowed && tile.kind == "run_command") {
                                                commandsDenied = true
                                            }
                                        }
                                        Core.DeckPressResult.Offline -> {
                                            access = Core.RemoteAccess.Offline
                                        }
                                        else -> {}
                                    }
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

private object MaterialResource {
    @Composable
    fun onSurfaceVariant(): Color = MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
private fun DeckBanner(
    title: String,
    text: String?,
    onDismiss: (() -> Unit)? = null,
) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                if (text != null) {
                    Spacer(Modifier.height(2.dp))
                    Text(text, style = MaterialTheme.typography.bodySmall)
                }
            }
            if (onDismiss != null) {
                TextButton(onClick = onDismiss) {
                    Text("×", style = MaterialTheme.typography.titleMedium)
                }
            }
        }
    }
}

private data class TilePalette(
    val bg: Color,
    val border: Color,
    val accent: Color,
    val fg: Color,
    val iconWell: Color,
)

@Composable
private fun tilePalette(
    @Suppress("UNUSED_PARAMETER") colorName: String,
    highlighted: Boolean,
): TilePalette {
    val scheme = MaterialTheme.colorScheme
    return if (highlighted) {
        TilePalette(
            bg = scheme.primaryContainer,
            border = scheme.primary,
            accent = scheme.primary,
            fg = scheme.onPrimaryContainer,
            iconWell = scheme.surfaceContainerHigh,
        )
    } else {
        TilePalette(
            bg = scheme.surfaceContainerHigh,
            border = scheme.outlineVariant,
            accent = scheme.primary,
            fg = scheme.onSurface,
            iconWell = scheme.secondaryContainer,
        )
    }
}

@Composable
private fun liveBadgeText(kind: String, state: DeckState): String? = when (kind) {
    "media_play_pause" -> stringResource(if (state.playing) R.string.deck_playing else R.string.deck_paused)
    "volume_mute" ->
        if (state.muted) stringResource(R.string.deck_muted)
        else if (state.volume > 0u) stringResource(R.string.deck_volume_percent, state.volume.toInt())
        else null
    "mic_mute" -> when (state.micMuted) {
        true -> stringResource(R.string.deck_muted)
        false -> stringResource(R.string.deck_live)
        null -> null
    }
    else -> null
}

private fun isLiveHighlighted(kind: String, state: DeckState): Boolean = when (kind) {
    "media_play_pause" -> state.playing
    "volume_mute" -> state.muted
    "mic_mute" -> state.micMuted == true
    else -> false
}

private fun effectiveIcon(tile: DeckTile, state: DeckState): String = when {
    tile.kind == "media_play_pause" && state.playing -> "pause"
    tile.kind == "mic_mute" && state.micMuted == true -> "mic_off"
    tile.kind == "volume_mute" && state.muted -> "volume_off"
    else -> tile.icon
}

@Composable
private fun DeckTileButton(
    tile: DeckTile,
    liveState: DeckState,
    compact: Boolean,
    enabled: Boolean,
    onPress: () -> Unit,
) {
    val badge = liveBadgeText(tile.kind, liveState)
    val highlighted = isLiveHighlighted(tile.kind, liveState)
    val palette = tilePalette(tile.color, highlighted)
    val iconName = effectiveIcon(tile, liveState)

    var pressed by remember { mutableStateOf(false) }
    val scale by animateFloatAsState(
        targetValue = if (pressed) 0.96f else 1f,
        animationSpec = spring(stiffness = Spring.StiffnessHigh),
        label = "tileScale",
    )

    val a11yDescription = if (badge != null) "${tile.label}, $badge" else tile.label
    val fontScale = LocalConfiguration.current.fontScale.coerceIn(1f, 1.35f)
    val tileHeight = ((if (compact) 98 else 118) * fontScale).dp

    Surface(
        shape = MaterialTheme.shapes.large,
        color = palette.bg.copy(alpha = if (enabled) 1f else 0.55f),
        contentColor = palette.fg,
        border = BorderStroke(
            width = if (highlighted) 2.dp else 1.dp,
            color = if (highlighted) palette.accent else palette.border,
        ),
        modifier = Modifier
            .fillMaxWidth()
            .height(tileHeight)
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
            }
            .semantics {
                role = Role.Button
                contentDescription = a11yDescription
            }
            .pointerInput(enabled, tile.id) {
                if (!enabled) return@pointerInput
                detectTapGestures(
                    onPress = {
                        pressed = true
                        try {
                            if (tryAwaitRelease()) {
                                onPress()
                            }
                        } finally {
                            pressed = false
                        }
                    },
                )
            },
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 11.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top,
            ) {
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    color = palette.iconWell,
                    modifier = Modifier.size(if (compact) 34.dp else 36.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        DeckIcon(
                            name = iconName,
                            tint = palette.fg,
                            modifier = Modifier.size(if (compact) 18.dp else 20.dp),
                        )
                    }
                }

                if (badge != null) {
                    Spacer(Modifier.width(6.dp))
                    Surface(
                        shape = CircleShape,
                        color = if (highlighted) palette.accent else palette.iconWell,
                        contentColor = if (highlighted) MaterialTheme.colorScheme.onPrimary else palette.fg,
                    ) {
                        Text(
                            text = badge,
                            style = MaterialTheme.typography.labelSmall,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            softWrap = false,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                        )
                    }
                }
            }

            Text(
                text = tile.label,
                style = if (compact) MaterialTheme.typography.labelLarge else MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                color = palette.fg,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun DeckIcon(
    name: String,
    tint: Color,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier = modifier) {
        drawDeckIcon(name, tint)
    }
}

private fun DrawScope.drawDeckIcon(name: String, color: Color) {
    val w = size.width
    val h = size.height
    val stroke = Stroke(
        width = w * 0.095f,
        cap = StrokeCap.Round,
        join = StrokeJoin.Round,
    )
    fun sx(x: Float) = x / 24f * w
    fun sy(y: Float) = y / 24f * h
    fun pt(x: Float, y: Float) = Offset(sx(x), sy(y))

    when (name) {
        "play", "play_pause" -> {
            val play = Path().apply {
                moveTo(sx(6f), sy(4.5f))
                lineTo(sx(19f), sy(12f))
                lineTo(sx(6f), sy(19.5f))
                close()
            }
            drawPath(play, color = color, style = stroke)
        }
        "pause" -> {
            drawLine(color, pt(8f, 5f), pt(8f, 19f), strokeWidth = stroke.width * 1.15f, cap = StrokeCap.Round)
            drawLine(color, pt(16f, 5f), pt(16f, 19f), strokeWidth = stroke.width * 1.15f, cap = StrokeCap.Round)
        }
        "skip_next" -> {
            val tri = Path().apply {
                moveTo(sx(5f), sy(5f))
                lineTo(sx(15f), sy(12f))
                lineTo(sx(5f), sy(19f))
                close()
            }
            drawPath(tri, color = color, style = stroke)
            drawLine(color, pt(19f, 5f), pt(19f, 19f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "skip_previous" -> {
            val tri = Path().apply {
                moveTo(sx(19f), sy(5f))
                lineTo(sx(9f), sy(12f))
                lineTo(sx(19f), sy(19f))
                close()
            }
            drawPath(tri, color = color, style = stroke)
            drawLine(color, pt(5f, 5f), pt(5f, 19f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "volume_up" -> {
            drawSpeakerCone(color, stroke, ::sx, ::sy)
            drawArc(
                color = color,
                startAngle = -45f,
                sweepAngle = 90f,
                useCenter = false,
                topLeft = pt(12f, 8f),
                size = Size(sx(5f), sy(8f)),
                style = stroke,
            )
            drawArc(
                color = color,
                startAngle = -45f,
                sweepAngle = 90f,
                useCenter = false,
                topLeft = pt(14f, 5f),
                size = Size(sx(7f), sy(14f)),
                style = stroke,
            )
        }
        "volume_down" -> {
            drawSpeakerCone(color, stroke, ::sx, ::sy)
            drawArc(
                color = color,
                startAngle = -45f,
                sweepAngle = 90f,
                useCenter = false,
                topLeft = pt(12.5f, 8f),
                size = Size(sx(5f), sy(8f)),
                style = stroke,
            )
        }
        "volume_off" -> {
            drawSpeakerCone(color, stroke, ::sx, ::sy)
            drawLine(color, pt(16f, 9f), pt(21f, 15f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(21f, 9f), pt(16f, 15f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "mic", "mic_off" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(9f, 3f),
                size = Size(sx(6f), sy(10f)),
                cornerRadius = CornerRadius(sx(3f), sy(3f)),
                style = stroke,
            )
            drawArc(
                color = color,
                startAngle = 0f,
                sweepAngle = 180f,
                useCenter = false,
                topLeft = pt(5f, 6f),
                size = Size(sx(14f), sy(11f)),
                style = stroke,
            )
            drawLine(color, pt(12f, 17f), pt(12f, 21f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(8f, 21f), pt(16f, 21f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            if (name == "mic_off") {
                drawLine(color, pt(4f, 4f), pt(20f, 20f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            }
        }
        "lock" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(5f, 11f),
                size = Size(sx(14f), sy(10f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            drawArc(
                color = color,
                startAngle = 180f,
                sweepAngle = 180f,
                useCenter = false,
                topLeft = pt(8f, 4f),
                size = Size(sx(8f), sy(8f)),
                style = stroke,
            )
            drawLine(color, pt(8f, 8f), pt(8f, 11f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(16f, 8f), pt(16f, 11f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "desktop" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(3f, 4f),
                size = Size(sx(18f), sy(12f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            drawLine(color, pt(8f, 20f), pt(16f, 20f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(12f, 16f), pt(12f, 20f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "switch_window", "windows" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(3f, 7f),
                size = Size(sx(13f), sy(12f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            val back = Path().apply {
                moveTo(sx(8f), sy(7f))
                lineTo(sx(8f), sy(5f))
                lineTo(sx(21f), sy(5f))
                lineTo(sx(21f), sy(15f))
                lineTo(sx(16f), sy(15f))
            }
            drawPath(back, color = color, style = stroke)
        }
        "screenshot", "camera" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(3f, 7f),
                size = Size(sx(18f), sy(12f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            drawCircle(color = color, radius = sx(3.2f), center = pt(12f, 13f), style = stroke)
            drawLine(color, pt(9f, 4.5f), pt(15f, 4.5f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "keyboard" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(2f, 5f),
                size = Size(sx(20f), sy(14f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            drawLine(color, pt(8f, 15f), pt(16f, 15f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            for (x in listOf(6f, 10f, 14f, 18f)) {
                drawCircle(color = color, radius = sx(1f), center = pt(x, 10f))
            }
        }
        "globe" -> {
            drawCircle(color = color, radius = sx(9f), center = pt(12f, 12f), style = stroke)
            drawLine(color, pt(3f, 12f), pt(21f, 12f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawOval(
                color = color,
                topLeft = pt(8f, 3f),
                size = Size(sx(8f), sy(18f)),
                style = stroke,
            )
        }
        "text" -> {
            drawLine(color, pt(4f, 6f), pt(20f, 6f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(12f, 6f), pt(12f, 19f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawLine(color, pt(9f, 19f), pt(15f, 19f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "app", "rocket" -> {
            for (gx in listOf(4f, 13f)) {
                for (gy in listOf(4f, 13f)) {
                    drawRoundRect(
                        color = color,
                        topLeft = pt(gx, gy),
                        size = Size(sx(7f), sy(7f)),
                        cornerRadius = CornerRadius(sx(1.5f), sy(1.5f)),
                        style = stroke,
                    )
                }
            }
        }
        "terminal" -> {
            val chevron = Path().apply {
                moveTo(sx(4f), sy(7f))
                lineTo(sx(10f), sy(12f))
                lineTo(sx(4f), sy(17f))
            }
            drawPath(chevron, color = color, style = stroke)
            drawLine(color, pt(12f, 17f), pt(20f, 17f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        "spark", "bolt" -> {
            val bolt = Path().apply {
                moveTo(sx(13f), sy(2f))
                lineTo(sx(4f), sy(14f))
                lineTo(sx(11f), sy(14f))
                lineTo(sx(10f), sy(22f))
                lineTo(sx(20f), sy(10f))
                lineTo(sx(13f), sy(10f))
                close()
            }
            drawPath(bolt, color = color, style = stroke)
        }
        "music" -> {
            val note = Path().apply {
                moveTo(sx(9f), sy(18f))
                lineTo(sx(9f), sy(6f))
                lineTo(sx(19f), sy(4f))
                lineTo(sx(19f), sy(16f))
            }
            drawPath(note, color = color, style = stroke)
            drawCircle(color = color, radius = sx(2.5f), center = pt(6.5f, 18f), style = stroke)
            drawCircle(color = color, radius = sx(2.5f), center = pt(16.5f, 16f), style = stroke)
        }
        "video" -> {
            drawRoundRect(
                color = color,
                topLeft = pt(3f, 6f),
                size = Size(sx(12f), sy(12f)),
                cornerRadius = CornerRadius(sx(2f), sy(2f)),
                style = stroke,
            )
            val lens = Path().apply {
                moveTo(sx(15f), sy(10f))
                lineTo(sx(21f), sy(7f))
                lineTo(sx(21f), sy(17f))
                lineTo(sx(15f), sy(14f))
                close()
            }
            drawPath(lens, color = color, style = stroke)
        }
        "folder" -> {
            val folder = Path().apply {
                moveTo(sx(3f), sy(6f))
                lineTo(sx(9f), sy(6f))
                lineTo(sx(11f), sy(8f))
                lineTo(sx(21f), sy(8f))
                lineTo(sx(21f), sy(19f))
                lineTo(sx(3f), sy(19f))
                close()
            }
            drawPath(folder, color = color, style = stroke)
        }
        "star" -> {
            val star = Path().apply {
                moveTo(sx(12f), sy(3f))
                lineTo(sx(14.8f), sy(8.8f))
                lineTo(sx(21f), sy(9.7f))
                lineTo(sx(16.5f), sy(14.1f))
                lineTo(sx(17.6f), sy(20.3f))
                lineTo(sx(12f), sy(17.4f))
                lineTo(sx(6.4f), sy(20.3f))
                lineTo(sx(7.5f), sy(14.1f))
                lineTo(sx(3f), sy(9.7f))
                lineTo(sx(9.2f), sy(8.8f))
                close()
            }
            drawPath(star, color = color, style = stroke)
        }
        "bell" -> {
            val bell = Path().apply {
                moveTo(sx(6f), sy(16f))
                lineTo(sx(6f), sy(10f))
                cubicTo(sx(6f), sy(6.5f), sx(18f), sy(6.5f), sx(18f), sy(10f))
                lineTo(sx(18f), sy(16f))
                lineTo(sx(4f), sy(16f))
                lineTo(sx(20f), sy(16f))
            }
            drawPath(bell, color = color, style = stroke)
            drawLine(color, pt(10f, 19.5f), pt(14f, 19.5f), strokeWidth = stroke.width, cap = StrokeCap.Round)
        }
        else -> {
            // "power" or fallback
            drawLine(color, pt(12f, 3f), pt(12f, 11f), strokeWidth = stroke.width, cap = StrokeCap.Round)
            drawArc(
                color = color,
                startAngle = -55f,
                sweepAngle = 290f,
                useCenter = false,
                topLeft = pt(5f, 5f),
                size = Size(sx(14f), sy(14f)),
                style = stroke,
            )
        }
    }
}

private fun DrawScope.drawSpeakerCone(
    color: Color,
    stroke: Stroke,
    sx: (Float) -> Float,
    sy: (Float) -> Float,
) {
    val cone = Path().apply {
        moveTo(sx(11f), sy(5f))
        lineTo(sx(6f), sy(9f))
        lineTo(sx(3f), sy(9f))
        lineTo(sx(3f), sy(15f))
        lineTo(sx(6f), sy(15f))
        lineTo(sx(11f), sy(19f))
        close()
    }
    drawPath(cone, color = color, style = stroke)
}

private fun fallbackDefaultLayout(): DeckLayout = DeckLayout(
    pages = listOf(
        DeckPage(
            id = "main",
            name = "Main",
            tiles = listOf(
                DeckTile("play_pause", "Play / pause", "play", "amber", "media_play_pause"),
                DeckTile("prev_track", "Previous", "skip_previous", "slate", "media_previous"),
                DeckTile("next_track", "Next", "skip_next", "slate", "media_next"),
                DeckTile("vol_down", "Volume down", "volume_down", "teal", "volume_down"),
                DeckTile("vol_up", "Volume up", "volume_up", "teal", "volume_up"),
                DeckTile("vol_mute", "Mute audio", "volume_off", "teal", "volume_mute"),
                DeckTile("mic_mute", "Mic mute", "mic", "coral", "mic_mute"),
                DeckTile("show_desktop", "Show desktop", "desktop", "blue", "show_desktop"),
                DeckTile("switch_window", "Switch window", "switch_window", "blue", "switch_window"),
                DeckTile("screenshot", "Screenshot", "screenshot", "violet", "screenshot"),
                DeckTile("lock_pc", "Lock PC", "lock", "red", "lock_pc"),
            ),
        ),
    ),
)
