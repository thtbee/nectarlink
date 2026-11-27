// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.pairing

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
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
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.ui.theme.LocalAppFonts
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.PairingFailure
import kotlinx.coroutines.delay

/** What the user does on the pairing screen. */
interface PairingActions {
    /** The screen is shown or hidden (discovery needs a multicast lock). */
    fun visible(open: Boolean)
    fun join(link: String)
    fun pairNearby(id: String)
    fun confirm(matches: Boolean)
    fun reset()

    /** A pairing link for another phone or tablet to scan, or null. */
    suspend fun showCode(): String?
}

internal enum class PairingLayoutMode {
    Portrait,
    TwoPaneLandscape,
    CompactScrollable,
}

internal fun choosePairingLayout(widthDp: Float, heightDp: Float): PairingLayoutMode {
    if (widthDp > heightDp && widthDp >= 480f) return PairingLayoutMode.TwoPaneLandscape
    if (heightDp < 520f) return PairingLayoutMode.CompactScrollable
    return PairingLayoutMode.Portrait
}

internal fun fitSquareSideDp(
    maxWidthDp: Float,
    maxHeightDp: Float,
    widthFraction: Float = 1f,
): Float {
    val w = maxWidthDp.coerceAtLeast(0f) * widthFraction
    return if (maxHeightDp.isFinite()) minOf(w, maxHeightDp.coerceAtLeast(0f)) else w
}

internal fun codeDigitWidthDp(
    maxWidthDp: Float,
    digitCount: Int = 6,
    spacingDp: Float = 8f,
    preferredWidthDp: Float = 44f,
): Float {
    if (digitCount <= 0) return preferredWidthDp
    val totalSpacing = spacingDp * (digitCount - 1)
    val availablePerDigit = ((maxWidthDp - totalSpacing) / digitCount).coerceAtLeast(24f)
    return minOf(preferredWidthDp, availablePerDigit)
}

/**
 * Pairing with a PC: scan the QR code it shows, or pick it from the PCs
 * nearby and compare a 6-digit code. Pairing with another phone or tablet:
 * one shows a code ("Show a code") and the other scans it. `onDone` runs
 * after success (or when the user backs out, if `cancellable`).
 */
@Composable
fun PairingScreen(
    state: CoreState,
    actions: PairingActions,
    cancellable: Boolean,
    onDone: () -> Unit,
) {
    var nearby by rememberSaveable { mutableStateOf(false) }
    var showing by rememberSaveable { mutableStateOf(false) }
    var showCodeLink by remember { mutableStateOf<String?>(null) }
    val pairing = state.pairing
    val reducedMotion = LocalReducedMotion.current
    DisposableEffect(Unit) {
        actions.visible(true)
        onDispose { actions.visible(false) }
    }

    LaunchedEffect(showing) {
        if (showing) {
            if (showCodeLink == null) showCodeLink = actions.showCode()
        } else {
            showCodeLink = null
        }
    }

    LaunchedEffect(pairing) {
        if (pairing is PairingState.Paired) {
            delay(1_400)
            actions.reset()
            onDone()
        }
    }

    val toggleNearby: () -> Unit = { nearby = !nearby }
    val toggleShowing: () -> Unit = {
        showing = !showing
        nearby = false
    }
    val cancel: () -> Unit = {
        actions.reset()
        onDone()
    }

    BoxWithConstraints(
        modifier = Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .padding(horizontal = 24.dp, vertical = 16.dp),
    ) {
        when (choosePairingLayout(maxWidth.value, maxHeight.value)) {
            PairingLayoutMode.Portrait -> PortraitPairingLayout(
                state = state,
                actions = actions,
                pairing = pairing,
                nearby = nearby,
                showing = showing,
                showCodeLink = showCodeLink,
                cancellable = cancellable,
                reducedMotion = reducedMotion,
                onToggleNearby = toggleNearby,
                onToggleShowing = toggleShowing,
                onCancel = cancel,
            )
            PairingLayoutMode.TwoPaneLandscape -> LandscapePairingLayout(
                state = state,
                actions = actions,
                pairing = pairing,
                nearby = nearby,
                showing = showing,
                showCodeLink = showCodeLink,
                cancellable = cancellable,
                reducedMotion = reducedMotion,
                onToggleNearby = toggleNearby,
                onToggleShowing = toggleShowing,
                onCancel = cancel,
            )
            PairingLayoutMode.CompactScrollable -> CompactScrollablePairingLayout(
                state = state,
                actions = actions,
                pairing = pairing,
                nearby = nearby,
                showing = showing,
                showCodeLink = showCodeLink,
                cancellable = cancellable,
                reducedMotion = reducedMotion,
                maxWidth = maxWidth,
                maxHeight = maxHeight,
                onToggleNearby = toggleNearby,
                onToggleShowing = toggleShowing,
                onCancel = cancel,
            )
        }
    }
}

@Composable
private fun PortraitPairingLayout(
    state: CoreState,
    actions: PairingActions,
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
    showCodeLink: String?,
    cancellable: Boolean,
    reducedMotion: Boolean,
    onToggleNearby: () -> Unit,
    onToggleShowing: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(24.dp))
        PairingTitle(pairing, nearby, showing)
        Spacer(Modifier.height(20.dp))

        PairingAnimatedContent(
            state = state,
            actions = actions,
            pairing = pairing,
            nearby = nearby,
            showing = showing,
            reducedMotion = reducedMotion,
            modifier = Modifier.weight(1f),
            scrollableNonIdle = true,
            showCompareHint = true,
        ) { showNearby, showCode ->
            when {
                showCode -> ShowCode(showCodeLink)
                showNearby -> NearbyList(
                    devices = state.discovered,
                    onPair = actions::pairNearby,
                    modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                )
                else -> Scan(actions::join)
            }
        }

        PairingActionsFooter(
            pairing = pairing,
            nearby = nearby,
            showing = showing,
            cancellable = cancellable,
            onToggleNearby = onToggleNearby,
            onToggleShowing = onToggleShowing,
            onCancel = onCancel,
        )
    }
}

@Composable
private fun LandscapePairingLayout(
    state: CoreState,
    actions: PairingActions,
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
    showCodeLink: String?,
    cancellable: Boolean,
    reducedMotion: Boolean,
    onToggleNearby: () -> Unit,
    onToggleShowing: () -> Unit,
    onCancel: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxSize(),
        horizontalArrangement = Arrangement.spacedBy(24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxHeight()
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            PairingTitle(pairing, nearby, showing)
            val hint = pairingHint(state, pairing, nearby, showing)
            if (hint != null) {
                Spacer(Modifier.height(12.dp))
                val isShowCodeHint = pairing == PairingState.Idle && showing
                Text(
                    text = hint,
                    style = if (isShowCodeHint) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.bodyMedium,
                    color = if (isShowCodeHint) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            if (pairing == PairingState.Idle || (cancellable && pairing !is PairingState.Paired)) {
                Spacer(Modifier.height(16.dp))
                PairingActionsFooter(
                    pairing = pairing,
                    nearby = nearby,
                    showing = showing,
                    cancellable = cancellable,
                    onToggleNearby = onToggleNearby,
                    onToggleShowing = onToggleShowing,
                    onCancel = onCancel,
                )
            }
        }

        PairingAnimatedContent(
            state = state,
            actions = actions,
            pairing = pairing,
            nearby = nearby,
            showing = showing,
            reducedMotion = reducedMotion,
            modifier = Modifier.weight(1f).fillMaxHeight(),
            contentAlignment = Alignment.Center,
            scrollableNonIdle = true,
            showCompareHint = false,
        ) { showNearby, showCode ->
            when {
                showCode -> BoxWithConstraints(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    val side = fitSquareSideDp(maxWidth.value, maxHeight.value).dp
                    QrCodeBox(showCodeLink, side)
                }
                showNearby -> NearbyDevicesList(
                    devices = state.discovered,
                    onPair = actions::pairNearby,
                    modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                )
                else -> BoxWithConstraints(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    val side = fitSquareSideDp(maxWidth.value, maxHeight.value).dp
                    QrScanner(
                        onLink = actions::join,
                        modifier = Modifier
                            .size(side)
                            .clip(MaterialTheme.shapes.extraLarge)
                            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
                    )
                }
            }
        }
    }
}

@Composable
private fun CompactScrollablePairingLayout(
    state: CoreState,
    actions: PairingActions,
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
    showCodeLink: String?,
    cancellable: Boolean,
    reducedMotion: Boolean,
    maxWidth: Dp,
    maxHeight: Dp,
    onToggleNearby: () -> Unit,
    onToggleShowing: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(8.dp))
        PairingTitle(pairing, nearby, showing)
        Spacer(Modifier.height(16.dp))

        val boundedHeight = (maxHeight.value * 0.55f).coerceAtLeast(160f)
        PairingAnimatedContent(
            state = state,
            actions = actions,
            pairing = pairing,
            nearby = nearby,
            showing = showing,
            reducedMotion = reducedMotion,
            scrollableNonIdle = false,
            showCompareHint = true,
        ) { showNearby, showCode ->
            when {
                showCode -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        stringResource(R.string.pair_show_hint),
                        style = MaterialTheme.typography.bodyLarge,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(16.dp))
                    val side = fitSquareSideDp(maxWidth.value, boundedHeight, widthFraction = 0.8f).dp
                    QrCodeBox(showCodeLink, side)
                }
                showNearby -> NearbyList(state.discovered, actions::pairNearby)
                else -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        stringResource(R.string.pair_scan_hint),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(12.dp))
                    val side = fitSquareSideDp(maxWidth.value, boundedHeight).dp
                    QrScanner(
                        onLink = actions::join,
                        modifier = Modifier
                            .size(side)
                            .clip(MaterialTheme.shapes.extraLarge)
                            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
                    )
                }
            }
        }

        Spacer(Modifier.height(16.dp))
        PairingActionsFooter(
            pairing = pairing,
            nearby = nearby,
            showing = showing,
            cancellable = cancellable,
            onToggleNearby = onToggleNearby,
            onToggleShowing = onToggleShowing,
            onCancel = onCancel,
        )
    }
}

@Composable
private fun PairingAnimatedContent(
    state: CoreState,
    actions: PairingActions,
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
    reducedMotion: Boolean,
    modifier: Modifier = Modifier,
    contentAlignment: Alignment = Alignment.TopStart,
    scrollableNonIdle: Boolean,
    showCompareHint: Boolean,
    idleContent: @Composable (showNearby: Boolean, showCode: Boolean) -> Unit,
) {
    val nonIdleModifier = if (scrollableNonIdle) {
        Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
    } else {
        Modifier
    }
    AnimatedContent(
        targetState = Triple(pairing, nearby, showing),
        transitionSpec = {
            if (reducedMotion) {
                EnterTransition.None togetherWith ExitTransition.None
            } else {
                fadeIn() togetherWith fadeOut()
            }
        },
        modifier = modifier,
        contentAlignment = contentAlignment,
        label = "pairing",
    ) { (current, showNearby, showCode) ->
        when (current) {
            PairingState.Idle -> idleContent(showNearby, showCode)
            PairingState.Joining -> Working(stringResource(R.string.pair_joining))
            is PairingState.Connecting ->
                Working(stringResource(R.string.pair_connecting, state.nameOf(current.peer) ?: ""))
            is PairingState.Comparing -> Column(
                modifier = nonIdleModifier,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                if (showCompareHint) {
                    Compare(state.nameOf(current.peer), current.code, actions::confirm)
                } else {
                    CompareActions(current.code, actions::confirm)
                }
            }
            is PairingState.Confirmed -> Column(
                modifier = nonIdleModifier,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                CodeDigits(current.code)
                Spacer(Modifier.height(24.dp))
                Working(stringResource(R.string.pair_waiting, state.nameOf(current.peer) ?: ""))
            }
            is PairingState.Paired -> Outcome(true, stringResource(R.string.pair_done, current.name))
            is PairingState.Failed -> Column(
                modifier = nonIdleModifier,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Outcome(false, failureText(current.failure))
                Spacer(Modifier.height(16.dp))
                Button(onClick = actions::reset) { Text(stringResource(R.string.action_try_again)) }
            }
        }
    }
}

@Composable
private fun PairingTitle(pairing: PairingState, nearby: Boolean, showing: Boolean) {
    Text(
        text = when (pairing) {
            is PairingState.Comparing, is PairingState.Confirmed -> stringResource(R.string.pair_check_code)
            is PairingState.Paired -> stringResource(R.string.pair_done_title)
            is PairingState.Failed -> stringResource(R.string.pair_failed_title)
            else -> when {
                showing -> stringResource(R.string.pair_show_title)
                nearby -> stringResource(R.string.pair_nearby_title)
                else -> stringResource(R.string.pair_title)
            }
        },
        style = MaterialTheme.typography.headlineMedium,
        textAlign = TextAlign.Center,
    )
}

@Composable
private fun pairingHint(
    state: CoreState,
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
): String? = when (pairing) {
    PairingState.Idle -> when {
        showing -> stringResource(R.string.pair_show_hint)
        nearby -> stringResource(R.string.pair_nearby_hint)
        else -> stringResource(R.string.pair_scan_hint)
    }
    is PairingState.Comparing ->
        stringResource(R.string.pair_compare_hint, state.nameOf(pairing.peer) ?: stringResource(R.string.your_pc))
    else -> null
}

@Composable
private fun PairingActionsFooter(
    pairing: PairingState,
    nearby: Boolean,
    showing: Boolean,
    cancellable: Boolean,
    onToggleNearby: () -> Unit,
    onToggleShowing: () -> Unit,
    onCancel: () -> Unit,
) {
    if (pairing == PairingState.Idle) {
        if (!showing) {
            FilledTonalButton(onClick = onToggleNearby) {
                Text(stringResource(if (nearby) R.string.action_scan_instead else R.string.action_pair_nearby))
            }
        }
        TextButton(onClick = onToggleShowing) {
            Text(stringResource(if (showing) R.string.action_scan_instead else R.string.action_show_code))
        }
    }
    if (cancellable && pairing !is PairingState.Paired) {
        TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) }
    }
}

/** This device's pairing code, for another phone or tablet to scan. */
@Composable
private fun ShowCode(link: String?) {
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.pair_show_hint),
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(20.dp))
        BoxWithConstraints(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentAlignment = Alignment.TopCenter,
        ) {
            val side = fitSquareSideDp(maxWidth.value, maxHeight.value, widthFraction = 0.8f).dp
            QrCodeBox(link, side)
        }
    }
}

@Composable
private fun QrCodeBox(link: String?, side: Dp) {
    val label = stringResource(R.string.pair_show_qr_desc)
    Box(
        modifier = Modifier
            .size(side)
            .clip(androidx.compose.foundation.shape.RoundedCornerShape(20.dp))
            .background(androidx.compose.ui.graphics.Color.White)
            .padding(16.dp)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        if (link == null) {
            CircularProgressIndicator()
        } else {
            QrCode(link)
        }
    }
}

/** [text] as a QR code, black on the white it sits on. */
@Composable
private fun QrCode(text: String) {
    val matrix = androidx.compose.runtime.remember(text) {
        runCatching {
            com.google.zxing.qrcode.QRCodeWriter().encode(
                text,
                com.google.zxing.BarcodeFormat.QR_CODE,
                0,
                0,
                mapOf(com.google.zxing.EncodeHintType.MARGIN to 0),
            )
        }.getOrNull()
    } ?: return
    androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
        val cell = size.minDimension / matrix.width
        for (y in 0 until matrix.height) {
            for (x in 0 until matrix.width) {
                if (matrix.get(x, y)) {
                    drawRect(
                        androidx.compose.ui.graphics.Color.Black,
                        topLeft = androidx.compose.ui.geometry.Offset(x * cell, y * cell),
                        size = androidx.compose.ui.geometry.Size(cell, cell),
                    )
                }
            }
        }
    }
}

@Composable
private fun Scan(onLink: (String) -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.pair_scan_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(16.dp))
        BoxWithConstraints(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentAlignment = Alignment.TopCenter,
        ) {
            val side = fitSquareSideDp(maxWidth.value, maxHeight.value).dp
            QrScanner(
                onLink = onLink,
                modifier = Modifier
                    .size(side)
                    .clip(MaterialTheme.shapes.extraLarge)
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh),
            )
        }
    }
}

@Composable
private fun NearbyList(
    devices: List<DiscoveredDevice>,
    onPair: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(
            stringResource(R.string.pair_nearby_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(6.dp))
        NearbyDeviceCards(devices, onPair)
    }
}

@Composable
private fun NearbyDevicesList(
    devices: List<DiscoveredDevice>,
    onPair: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        NearbyDeviceCards(devices, onPair)
    }
}

@Composable
private fun NearbyDeviceCards(devices: List<DiscoveredDevice>, onPair: (String) -> Unit) {
    val reducedMotion = LocalReducedMotion.current
    if (devices.isEmpty()) {
        Surface(
            shape = MaterialTheme.shapes.large,
            color = MaterialTheme.colorScheme.surfaceContainer,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(
                Modifier.fillMaxWidth().padding(20.dp),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (reducedMotion) {
                    CircularProgressIndicator(
                        progress = { 0.75f },
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                    )
                } else {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                }
                Spacer(Modifier.width(12.dp))
                Text(
                    stringResource(R.string.pair_looking),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
    devices.forEach { device ->
        Surface(
            shape = MaterialTheme.shapes.large,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    device.name ?: stringResource(R.string.your_pc),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = { onPair(device.id) }) { Text(stringResource(R.string.action_pair)) }
            }
        }
    }
}

@Composable
private fun Compare(peerName: String?, code: String, onConfirm: (Boolean) -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            stringResource(R.string.pair_compare_hint, peerName ?: stringResource(R.string.your_pc)),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        CompareActions(code, onConfirm)
    }
}

@Composable
private fun CompareActions(code: String, onConfirm: (Boolean) -> Unit) {
    CodeDigits(code)
    Spacer(Modifier.height(32.dp))
    Button(onClick = { onConfirm(true) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
        Text(stringResource(R.string.action_codes_match))
    }
    Spacer(Modifier.height(8.dp))
    OutlinedButton(onClick = { onConfirm(false) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
        Text(stringResource(R.string.action_codes_differ))
    }
}

@Composable
private fun CodeDigits(code: String) {
    BoxWithConstraints(
        modifier = Modifier.fillMaxWidth(),
        contentAlignment = Alignment.Center,
    ) {
        val digitCount = code.length.coerceAtLeast(1)
        val digitWidth = codeDigitWidthDp(maxWidth.value, digitCount).dp
        val digitHeight = digitWidth * (56f / 44f)
        val fontSize = (26f * (digitWidth.value / 44f)).sp
        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.semantics { contentDescription = code.toList().joinToString(" ") },
        ) {
            code.forEach { digit ->
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    modifier = Modifier.size(width = digitWidth, height = digitHeight),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Text(
                            digit.toString(),
                            fontFamily = LocalAppFonts.current.mono,
                            fontSize = fontSize,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun Working(text: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (LocalReducedMotion.current) {
            CircularProgressIndicator(
                progress = { 0.75f },
                modifier = Modifier.size(20.dp),
                strokeWidth = 2.dp,
            )
        } else {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        }
        Spacer(Modifier.width(12.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun Outcome(success: Boolean, text: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Surface(
            shape = CircleShape,
            color = if (success) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.size(64.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Text(
                    if (success) "✓" else "!",
                    fontSize = 28.sp,
                    fontWeight = FontWeight.Bold,
                    color = if (success) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
        Spacer(Modifier.height(16.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
    }
}

@Composable
private fun failureText(failure: PairingFailure): String = when (failure) {
    PairingFailure.Rejected -> stringResource(R.string.pair_failed_rejected)
    PairingFailure.Declined -> stringResource(R.string.pair_failed_declined)
    PairingFailure.Expired -> stringResource(R.string.pair_failed_expired)
    PairingFailure.Unreachable -> stringResource(R.string.pair_failed_unreachable)
    PairingFailure.NotPairing -> stringResource(R.string.pair_failed_not_pairing)
    is PairingFailure.Other -> stringResource(R.string.pair_failed_other)
}
