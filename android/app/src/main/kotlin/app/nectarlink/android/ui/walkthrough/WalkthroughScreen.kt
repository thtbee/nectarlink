// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.walkthrough

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.StringRes
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.PairingNotification
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.ui.pairing.PairingActions
import app.nectarlink.android.ui.pairing.QrScanner
import app.nectarlink.android.ui.theme.LocalGlassTheme
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.bentoWell
import app.nectarlink.android.ui.theme.liquidGlassCard
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.PairingFailure
import kotlin.math.cos
import kotlin.math.sin
import kotlinx.coroutines.launch

/**
 * First-run setup walkthrough screen with 5 steps:
 * 1. Welcome
 * 2. Permissions
 * 3. Pair with your PC
 * 4. Choose your power level (Standard, Assist, Elevated)
 * 5. Setup complete
 */
@Composable
fun WalkthroughScreen(
    state: CoreState,
    step: WalkthroughStep,
    pairingActions: PairingActions,
    onStepChange: (WalkthroughStep) -> Unit,
    onSkip: () -> Unit,
    onFinishToHome: () -> Unit,
    onOpenFeaturesGuide: () -> Unit,
    onAccessChanged: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val reducedMotion = LocalReducedMotion.current
    val isGlass = LocalGlassTheme.current

    BackHandler(enabled = step.previous != null) {
        step.previous?.let(onStepChange)
    }

    fun checkPostNotifGranted(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED
        } else {
            true
        }

    var postNotifGranted by remember { mutableStateOf(checkPostNotifGranted()) }
    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, state.notificationAccess) {
        postNotifGranted = checkPostNotifGranted()
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
                postNotifGranted = checkPostNotifGranted()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    val notifyAccessAndPostNotifChanged: () -> Unit = {
        postNotifGranted = checkPostNotifGranted()
        onAccessChanged()
    }

    val permissionGrantedFlags = remember(
        state.notificationAccess,
        postNotifGranted,
        state.backgroundUnrestricted,
        state.callAccess,
        state.smsAccess,
        state.contactsAccess,
        state.photoAccess,
        state.storageAllFilesAccess,
        state.storageSafFolders,
        state.inputAccess,
        state.elevated,
    ) {
        listOf(
            state.notificationAccess && postNotifGranted,
            state.backgroundUnrestricted,
            state.callAccess,
            state.smsAccess && state.contactsAccess,
            state.photoAccess,
            state.storageAllFilesAccess || state.storageSafFolders.isNotEmpty(),
            state.inputAccess || state.elevated,
        )
    }

    val hasPairedPc = state.devices.isNotEmpty() || state.pairing is PairingState.Paired
    val activeTierIndex = when {
        state.elevated -> 2
        state.inputAccess -> 1
        else -> 0
    }

    Surface(
        modifier = modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = 20.dp, vertical = 12.dp),
        ) {
            WalkthroughTopProgressRow(
                currentStep = step,
                onSelectStep = onStepChange,
                onSkip = onSkip,
            )
            Spacer(Modifier.height(12.dp))

            HoneycombCellStage(
                step = step,
                permissionGrantedFlags = permissionGrantedFlags,
                hasPairedPc = hasPairedPc,
                activeTierIndex = activeTierIndex,
                isGlass = isGlass,
                reducedMotion = reducedMotion,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(128.dp),
            )
            Spacer(Modifier.height(14.dp))

            AnimatedContent(
                targetState = step,
                transitionSpec = {
                    if (reducedMotion) {
                        fadeIn(tween(0)) togetherWith fadeOut(tween(0))
                    } else {
                        fadeIn(tween(180)) togetherWith fadeOut(tween(140))
                    }
                },
                // Content scrolled up fades out under the stage instead of being cut mid-line.
                modifier = Modifier
                    .weight(1f)
                    .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                    .drawWithContent {
                        drawContent()
                        val fade = 20.dp.toPx()
                        drawRect(
                            brush = Brush.verticalGradient(
                                0f to Color.Transparent,
                                fade / size.height.coerceAtLeast(1f) to Color.Black,
                            ),
                            blendMode = BlendMode.DstIn,
                        )
                    },
                label = "walkthrough_step",
            ) { targetStep ->
                when (targetStep) {
                    WalkthroughStep.Welcome -> WelcomeStepContent(
                        isGlass = isGlass,
                    )
                    WalkthroughStep.Permissions -> PermissionsStepContent(
                        state = state,
                        postNotifGranted = postNotifGranted,
                        isGlass = isGlass,
                        onAccessChanged = notifyAccessAndPostNotifChanged,
                    )
                    WalkthroughStep.Pair -> PairStepContent(
                        state = state,
                        actions = pairingActions,
                        isGlass = isGlass,
                        onContinue = { onStepChange(WalkthroughStep.PowerLevel) },
                    )
                    WalkthroughStep.PowerLevel -> PowerLevelStepContent(
                        state = state,
                        isGlass = isGlass,
                        onAccessChanged = notifyAccessAndPostNotifChanged,
                    )
                    WalkthroughStep.Done -> DoneStepContent(
                        state = state,
                        postNotifGranted = postNotifGranted,
                        isGlass = isGlass,
                    )
                }
            }

            WalkthroughFooterBar(
                step = step,
                hasPairedPc = hasPairedPc,
                onBack = step.previous?.let { prev -> { onStepChange(prev) } },
                onNext = {
                    step.next?.let(onStepChange)
                },
                onOpenFeaturesGuide = onOpenFeaturesGuide,
                onFinish = onFinishToHome,
            )
        }
    }
}

@Composable
private fun WalkthroughTopProgressRow(
    currentStep: WalkthroughStep,
    onSelectStep: (WalkthroughStep) -> Unit,
    onSkip: () -> Unit,
) {
    val totalSteps = WalkthroughStep.entries.size
    val stepProgressDesc = stringResource(
        R.string.walkthrough_step_progress,
        currentStep.index + 1,
        totalSteps,
    )
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 36.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier
                .weight(1f)
                .semantics { contentDescription = stepProgressDesc },
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            WalkthroughStep.entries.forEach { item ->
                val active = item.index <= currentStep.index
                val color = if (active) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.surfaceContainerHighest
                }
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .height(6.dp)
                        .bentoWell(cornerRadius = 3.dp, fillColor = color)
                        .clickable(role = Role.Tab) { onSelectStep(item) },
                )
            }
        }
        if (currentStep != WalkthroughStep.Done) {
            TextButton(
                onClick = onSkip,
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
            ) {
                Text(stringResource(R.string.walkthrough_action_skip))
            }
        }
    }
}

@Composable
private fun WalkthroughFooterBar(
    step: WalkthroughStep,
    hasPairedPc: Boolean,
    onBack: (() -> Unit)?,
    onNext: () -> Unit,
    onOpenFeaturesGuide: () -> Unit,
    onFinish: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (onBack != null) {
            OutlinedButton(
                onClick = onBack,
                modifier = Modifier.heightIn(min = 46.dp),
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
            ) {
                Text(
                    text = stringResource(R.string.walkthrough_action_back),
                    maxLines = 1,
                    softWrap = false,
                )
            }
        }
        if (step == WalkthroughStep.Done) {
            Box(
                modifier = Modifier.weight(1f),
                contentAlignment = Alignment.CenterEnd,
            ) {
                FilledTonalButton(
                    onClick = onOpenFeaturesGuide,
                    modifier = Modifier.heightIn(min = 46.dp),
                    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
                ) {
                    Text(
                        text = stringResource(R.string.walkthrough_action_open_guide),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Button(
                onClick = onFinish,
                modifier = Modifier.heightIn(min = 46.dp),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
            ) {
                Text(
                    text = stringResource(R.string.walkthrough_action_done),
                    maxLines = 1,
                    softWrap = false,
                )
            }
        } else {
            Spacer(Modifier.weight(1f))
            val primaryLabelRes = when (step) {
                WalkthroughStep.Welcome -> R.string.walkthrough_action_get_started
                WalkthroughStep.Pair -> if (hasPairedPc) {
                    R.string.walkthrough_action_continue
                } else {
                    R.string.walkthrough_action_pair_later
                }
                else -> R.string.walkthrough_action_continue
            }
            Button(
                onClick = onNext,
                modifier = Modifier.heightIn(min = 46.dp),
            ) {
                Text(
                    text = stringResource(primaryLabelRes),
                    maxLines = 1,
                    softWrap = false,
                )
            }
        }
    }
}

/**
 * Fixed-height (128.dp) vector Honeycomb Cell stage shown at the top of every walkthrough step.
 * Combines the 5-cell step honeycomb cluster on the left with the active step's honeycomb motif on the right.
 */
@Composable
private fun HoneycombCellStage(
    step: WalkthroughStep,
    permissionGrantedFlags: List<Boolean>,
    hasPairedPc: Boolean,
    activeTierIndex: Int,
    isGlass: Boolean,
    reducedMotion: Boolean,
    modifier: Modifier = Modifier,
) {
    val primary = MaterialTheme.colorScheme.primary
    val primaryContainer = MaterialTheme.colorScheme.primaryContainer
    val onPrimaryContainer = MaterialTheme.colorScheme.onPrimaryContainer
    val surfaceHigh = MaterialTheme.colorScheme.surfaceContainerHigh
    val outlineVariant = MaterialTheme.colorScheme.outlineVariant
    val stageDesc = stringResource(
        R.string.walkthrough_stage_desc,
        step.index + 1,
        WalkthroughStep.entries.size,
    )

    val pairJoinProgress by animateFloatAsState(
        targetValue = if (hasPairedPc) 1f else 0f,
        animationSpec = if (reducedMotion) snap() else tween(durationMillis = 320),
        label = "pair_hex_join",
    )

    val hexPath = remember { Path() }

    Box(
        modifier = modifier
            .liquidGlassCard(
                cornerRadius = 24.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = primary,
                outlineColor = if (isGlass) outlineVariant else Color.Transparent,
            )
            .semantics { contentDescription = stageDesc }
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Canvas(modifier = Modifier.fillMaxSize()) {
            val w = size.width
            val h = size.height
            val strokePx = 2.dp.toPx()
            val accentStrokePx = 1.5.dp.toPx()

            // Left section (36% width): 5-cell honeycomb cluster representing Steps 0..4
            val clusterCx = w * 0.22f
            val clusterCy = h * 0.5f
            val cellR = 13.dp.toPx()
            val dx = cellR * 1.62f
            val dy = cellR * 1.42f

            // 5-cell honeycomb offsets (top row 3 cells, bottom row 2 interlocking cells)
            for (i in 0 until 5) {
                val cx = when (i) {
                    0 -> clusterCx - dx
                    1 -> clusterCx
                    2 -> clusterCx + dx
                    3 -> clusterCx - dx * 0.5f
                    else -> clusterCx + dx * 0.5f
                }
                val cy = if (i < 3) clusterCy - dy * 0.5f else clusterCy + dy * 0.5f

                val completed = i < step.index
                val isCurrent = i == step.index
                val fill = if (completed || isCurrent) primaryContainer else surfaceHigh
                val border = if (completed || isCurrent) primary else outlineVariant

                drawHexCell(
                    path = hexPath,
                    center = Offset(cx, cy),
                    radius = cellR,
                    fillColor = fill,
                    strokeColor = border,
                    strokeWidth = strokePx,
                )

                if (isCurrent) {
                    drawHexRing(
                        path = hexPath,
                        center = Offset(cx, cy),
                        radius = cellR + 3.5.dp.toPx(),
                        strokeColor = primary.copy(alpha = 0.55f),
                        strokeWidth = accentStrokePx,
                    )
                    drawCircle(
                        color = primary,
                        radius = 3.dp.toPx(),
                        center = Offset(cx, cy),
                    )
                } else if (completed) {
                    drawCheckGlyph(
                        path = hexPath,
                        center = Offset(cx, cy),
                        size = cellR * 0.45f,
                        color = primary,
                        strokeWidth = strokePx,
                    )
                }
            }

            // Subtle vertical divider between step cluster and active step motif
            val dividerX = w * 0.44f
            drawLine(
                color = outlineVariant.copy(alpha = 0.45f),
                start = Offset(dividerX, h * 0.14f),
                end = Offset(dividerX, h * 0.86f),
                strokeWidth = 1.dp.toPx(),
                cap = StrokeCap.Round,
            )

            // Right section (56% width): Active step honeycomb motif
            val motifCx = w * 0.72f
            val motifCy = h * 0.5f

            when (step) {
                WalkthroughStep.Welcome -> {
                    val sideR = 20.dp.toPx()
                    val linkR = 14.dp.toPx()
                    val leftCenter = Offset(motifCx - 46.dp.toPx(), motifCy)
                    val rightCenter = Offset(motifCx + 46.dp.toPx(), motifCy)
                    val midCenter = Offset(motifCx, motifCy)

                    drawLine(
                        color = primary,
                        start = Offset(leftCenter.x + sideR, motifCy),
                        end = Offset(midCenter.x - linkR, motifCy),
                        strokeWidth = strokePx,
                        cap = StrokeCap.Round,
                    )
                    drawLine(
                        color = primary,
                        start = Offset(midCenter.x + linkR, motifCy),
                        end = Offset(rightCenter.x - sideR, motifCy),
                        strokeWidth = strokePx,
                        cap = StrokeCap.Round,
                    )

                    drawHexCell(hexPath, leftCenter, sideR, primaryContainer, primary, strokePx)
                    drawPhoneGlyph(leftCenter, sideR * 0.48f, onPrimaryContainer, strokePx)

                    drawHexCell(hexPath, midCenter, linkR, primaryContainer, primary, strokePx)
                    drawHexRing(hexPath, midCenter, linkR + 3.dp.toPx(), primary.copy(alpha = 0.5f), accentStrokePx)
                    drawCircle(color = primary, radius = 3.dp.toPx(), center = midCenter)

                    drawHexCell(hexPath, rightCenter, sideR, primaryContainer, primary, strokePx)
                    drawMonitorGlyph(rightCenter, sideR * 0.52f, onPrimaryContainer, strokePx)
                }

                WalkthroughStep.Permissions -> {
                    val centerR = 18.dp.toPx()
                    val nodeR = 9.5.dp.toPx()
                    val orbitR = 34.dp.toPx()

                    for (idx in 0 until 7) {
                        val angleRad = Math.toRadians((idx * (360.0 / 7.0)) - 90.0)
                        val nx = motifCx + (orbitR * cos(angleRad)).toFloat()
                        val ny = motifCy + (orbitR * sin(angleRad)).toFloat()
                        val granted = permissionGrantedFlags.getOrElse(idx) { false }

                        drawLine(
                            color = if (granted) primary else outlineVariant,
                            start = Offset(motifCx, motifCy),
                            end = Offset(nx, ny),
                            strokeWidth = accentStrokePx,
                        )
                        drawHexCell(
                            path = hexPath,
                            center = Offset(nx, ny),
                            radius = nodeR,
                            fillColor = if (granted) primaryContainer else surfaceHigh,
                            strokeColor = if (granted) primary else outlineVariant,
                            strokeWidth = accentStrokePx,
                        )
                        if (granted) {
                            drawCircle(
                                color = primary,
                                radius = 2.5.dp.toPx(),
                                center = Offset(nx, ny),
                            )
                        }
                    }

                    drawHexCell(
                        path = hexPath,
                        center = Offset(motifCx, motifCy),
                        radius = centerR,
                        fillColor = primaryContainer,
                        strokeColor = primary,
                        strokeWidth = strokePx,
                    )
                    drawShieldGlyph(
                        path = hexPath,
                        center = Offset(motifCx, motifCy),
                        size = centerR * 0.52f,
                        color = primary,
                        strokeWidth = strokePx,
                    )
                }

                WalkthroughStep.Pair -> {
                    val hexR = 22.dp.toPx()
                    val maxOffset = 38.dp.toPx()
                    val minOffset = 19.dp.toPx()
                    val currentOffset = maxOffset - (maxOffset - minOffset) * pairJoinProgress
                    val leftCenter = Offset(motifCx - currentOffset, motifCy)
                    val rightCenter = Offset(motifCx + currentOffset, motifCy)

                    drawLine(
                        color = if (hasPairedPc) primary else outlineVariant,
                        start = leftCenter,
                        end = rightCenter,
                        strokeWidth = strokePx,
                    )
                    drawHexCell(
                        path = hexPath,
                        center = leftCenter,
                        radius = hexR,
                        fillColor = primaryContainer,
                        strokeColor = primary,
                        strokeWidth = strokePx,
                    )
                    drawPhoneGlyph(leftCenter, hexR * 0.45f, onPrimaryContainer, strokePx)

                    drawHexCell(
                        path = hexPath,
                        center = rightCenter,
                        radius = hexR,
                        fillColor = if (hasPairedPc) primaryContainer else surfaceHigh,
                        strokeColor = if (hasPairedPc) primary else outlineVariant,
                        strokeWidth = strokePx,
                    )
                    drawMonitorGlyph(rightCenter, hexR * 0.48f, onPrimaryContainer, strokePx)

                    if (hasPairedPc) {
                        val bondR = 11.dp.toPx()
                        drawHexCell(
                            path = hexPath,
                            center = Offset(motifCx, motifCy),
                            radius = bondR,
                            fillColor = primaryContainer,
                            strokeColor = primary,
                            strokeWidth = strokePx,
                        )
                        drawCheckGlyph(
                            path = hexPath,
                            center = Offset(motifCx, motifCy),
                            size = bondR * 0.45f,
                            color = primary,
                            strokeWidth = strokePx,
                        )
                    }
                }

                WalkthroughStep.PowerLevel -> {
                    val tierR = 18.dp.toPx()
                    val stepX = 36.dp.toPx()
                    for (tier in 0..2) {
                        val tx = motifCx + (tier - 1) * stepX
                        val ty = motifCy + (1 - tier) * 7.dp.toPx()
                        val isSelected = tier == activeTierIndex
                        val isUnlocked = tier <= activeTierIndex

                        drawHexCell(
                            path = hexPath,
                            center = Offset(tx, ty),
                            radius = tierR,
                            fillColor = if (isUnlocked) primaryContainer else surfaceHigh,
                            strokeColor = if (isUnlocked) primary else outlineVariant,
                            strokeWidth = strokePx,
                        )
                        if (isSelected) {
                            drawHexRing(
                                path = hexPath,
                                center = Offset(tx, ty),
                                radius = tierR + 3.5.dp.toPx(),
                                strokeColor = primary.copy(alpha = 0.6f),
                                strokeWidth = accentStrokePx,
                            )
                        }
                        // Draw 1, 2, or 3 level bars inside the hex
                        val barCount = tier + 1
                        val barSpacing = 4.5.dp.toPx()
                        val barHalfW = 5.dp.toPx()
                        val startY = ty + ((barCount - 1) * barSpacing) * 0.5f
                        for (b in 0 until barCount) {
                            val by = startY - b * barSpacing
                            drawLine(
                                color = if (isUnlocked) primary else outlineVariant,
                                start = Offset(tx - barHalfW, by),
                                end = Offset(tx + barHalfW, by),
                                strokeWidth = strokePx,
                                cap = StrokeCap.Round,
                            )
                        }
                    }
                }

                WalkthroughStep.Done -> {
                    val outerR = 13.dp.toPx()
                    val ringDist = 25.dp.toPx()
                    for (i in 0 until 5) {
                        val angleRad = Math.toRadians((i * 72.0) - 90.0)
                        val hx = motifCx + (ringDist * cos(angleRad)).toFloat()
                        val hy = motifCy + (ringDist * sin(angleRad)).toFloat()
                        drawHexCell(
                            path = hexPath,
                            center = Offset(hx, hy),
                            radius = outerR,
                            fillColor = primaryContainer,
                            strokeColor = primary,
                            strokeWidth = accentStrokePx,
                        )
                    }
                    val centerR = 19.dp.toPx()
                    drawHexCell(
                        path = hexPath,
                        center = Offset(motifCx, motifCy),
                        radius = centerR,
                        fillColor = primaryContainer,
                        strokeColor = primary,
                        strokeWidth = strokePx,
                    )
                    drawHexRing(
                        path = hexPath,
                        center = Offset(motifCx, motifCy),
                        radius = centerR + 3.5.dp.toPx(),
                        strokeColor = primary.copy(alpha = 0.6f),
                        strokeWidth = accentStrokePx,
                    )
                    drawCheckGlyph(
                        path = hexPath,
                        center = Offset(motifCx, motifCy),
                        size = centerR * 0.48f,
                        color = primary,
                        strokeWidth = strokePx * 1.2f,
                    )
                }
            }
        }
    }
}

private fun DrawScope.buildHexPath(
    path: Path,
    center: Offset,
    radius: Float,
) {
    path.reset()
    for (i in 0 until 6) {
        val angleRad = Math.toRadians((60.0 * i) - 30.0)
        val x = center.x + (radius * cos(angleRad)).toFloat()
        val y = center.y + (radius * sin(angleRad)).toFloat()
        if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
    }
    path.close()
}

private fun DrawScope.drawHexCell(
    path: Path,
    center: Offset,
    radius: Float,
    fillColor: Color,
    strokeColor: Color,
    strokeWidth: Float,
) {
    buildHexPath(path, center, radius)
    drawPath(path = path, color = fillColor, style = Fill)
    drawPath(
        path = path,
        color = strokeColor,
        style = Stroke(width = strokeWidth, join = StrokeJoin.Round),
    )
}

private fun DrawScope.drawHexRing(
    path: Path,
    center: Offset,
    radius: Float,
    strokeColor: Color,
    strokeWidth: Float,
) {
    buildHexPath(path, center, radius)
    drawPath(
        path = path,
        color = strokeColor,
        style = Stroke(width = strokeWidth, join = StrokeJoin.Round),
    )
}

private fun DrawScope.drawCheckGlyph(
    path: Path,
    center: Offset,
    size: Float,
    color: Color,
    strokeWidth: Float,
) {
    path.reset()
    path.moveTo(center.x - size * 0.75f, center.y + size * 0.05f)
    path.lineTo(center.x - size * 0.15f, center.y + size * 0.65f)
    path.lineTo(center.x + size * 0.85f, center.y - size * 0.55f)
    drawPath(
        path = path,
        color = color,
        style = Stroke(width = strokeWidth, cap = StrokeCap.Round, join = StrokeJoin.Round),
    )
}

private fun DrawScope.drawPhoneGlyph(
    center: Offset,
    halfH: Float,
    color: Color,
    strokeWidth: Float,
) {
    val halfW = halfH * 0.62f
    drawRoundRect(
        color = color,
        topLeft = Offset(center.x - halfW, center.y - halfH),
        size = Size(halfW * 2f, halfH * 2f),
        cornerRadius = CornerRadius(strokeWidth * 1.5f, strokeWidth * 1.5f),
        style = Stroke(width = strokeWidth),
    )
}

private fun DrawScope.drawMonitorGlyph(
    center: Offset,
    halfW: Float,
    color: Color,
    strokeWidth: Float,
) {
    val halfH = halfW * 0.68f
    val top = center.y - halfH
    val bottom = center.y + halfH * 0.65f
    drawRoundRect(
        color = color,
        topLeft = Offset(center.x - halfW, top),
        size = Size(halfW * 2f, bottom - top),
        cornerRadius = CornerRadius(strokeWidth * 1.2f, strokeWidth * 1.2f),
        style = Stroke(width = strokeWidth),
    )
    val baseY = center.y + halfH
    drawLine(
        color = color,
        start = Offset(center.x, bottom),
        end = Offset(center.x, baseY),
        strokeWidth = strokeWidth,
        cap = StrokeCap.Round,
    )
    drawLine(
        color = color,
        start = Offset(center.x - halfW * 0.45f, baseY),
        end = Offset(center.x + halfW * 0.45f, baseY),
        strokeWidth = strokeWidth,
        cap = StrokeCap.Round,
    )
}

private fun DrawScope.drawShieldGlyph(
    path: Path,
    center: Offset,
    size: Float,
    color: Color,
    strokeWidth: Float,
) {
    path.reset()
    path.moveTo(center.x, center.y - size)
    path.lineTo(center.x + size * 0.78f, center.y - size * 0.55f)
    path.lineTo(center.x + size * 0.65f, center.y + size * 0.25f)
    path.lineTo(center.x, center.y + size)
    path.lineTo(center.x - size * 0.65f, center.y + size * 0.25f)
    path.lineTo(center.x - size * 0.78f, center.y - size * 0.55f)
    path.close()
    drawPath(
        path = path,
        color = color,
        style = Stroke(width = strokeWidth, join = StrokeJoin.Round),
    )
}

@Composable
private fun WelcomeStepContent(
    isGlass: Boolean,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = stringResource(R.string.walkthrough_welcome_title),
            style = MaterialTheme.typography.headlineSmall,
        )
        Text(
            text = stringResource(R.string.walkthrough_welcome_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 24.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.surfaceContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = stringResource(R.string.walkthrough_welcome_card_heading),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_welcome_tls_title),
                detail = stringResource(R.string.walkthrough_welcome_tls_body),
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_welcome_lan_title),
                detail = stringResource(R.string.walkthrough_welcome_lan_body),
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_welcome_local_title),
                detail = stringResource(R.string.walkthrough_welcome_local_body),
            )
        }
    }
}

@Composable
private fun PrivacyPointRow(
    title: String,
    detail: String,
) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
        )
        Text(
            text = detail,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private data class WalkthroughPermissionSpec(
    val id: String,
    @StringRes val titleRes: Int,
    @StringRes val whatItDoesRes: Int,
    @StringRes val whatAndroidRequiresRes: Int,
    val optional: Boolean,
    val granted: Boolean,
    val onGrant: () -> Unit,
    /** Opens where the permission can be changed once it's granted. */
    val onManage: () -> Unit,
    /** Android may hold it back as a restricted setting for a sideloaded app. */
    val restrictable: Boolean = false,
)

@Composable
private fun PermissionsStepContent(
    state: CoreState,
    postNotifGranted: Boolean,
    isGlass: Boolean,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    var activeCardIndex by rememberSaveable { mutableIntStateOf(0) }
    // Cards whose Grant was tapped, so restricted-settings help shows only
    // after Android has had its chance (its menu item appears only then).
    var attempted by rememberSaveable { mutableStateOf("") }
    val sideloaded = remember { BackgroundAccess.isSideloaded(context) }
    val appInfo = { runCatching { context.startActivity(BackgroundAccess.appDetailsIntent(context)) } }

    val notifRuntimeLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) {
        onAccessChanged()
        if (!NotificationListener.hasAccess(context)) {
            runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }
        }
    }

    val callsLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        onAccessChanged()
    }

    val smsContactsLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        onAccessChanged()
    }

    val photosLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        onAccessChanged()
    }

    val specs = listOf(
        WalkthroughPermissionSpec(
            id = "notifications",
            titleRes = R.string.walkthrough_perm_notif_title,
            whatItDoesRes = R.string.walkthrough_perm_notif_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_notif_requires,
            optional = false,
            granted = state.notificationAccess && postNotifGranted,
            onGrant = {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                    ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                ) {
                    notifRuntimeLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                } else {
                    runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }
                }
            },
            onManage = { runCatching { context.startActivity(NotificationListener.settingsIntent(context)) } },
            restrictable = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU,
        ),
        WalkthroughPermissionSpec(
            id = "battery",
            titleRes = R.string.walkthrough_perm_battery_title,
            whatItDoesRes = R.string.walkthrough_perm_battery_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_battery_requires,
            optional = false,
            granted = state.backgroundUnrestricted,
            onGrant = {
                runCatching { context.startActivity(BackgroundAccess.requestIntent(context)) }
            },
            onManage = { appInfo() },
        ),
        WalkthroughPermissionSpec(
            id = "calls",
            titleRes = R.string.walkthrough_perm_calls_title,
            whatItDoesRes = R.string.walkthrough_perm_calls_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_calls_requires,
            optional = false,
            granted = state.callAccess,
            onGrant = {
                callsLauncher.launch(PhoneCalls.permissions)
            },
            onManage = { appInfo() },
        ),
        WalkthroughPermissionSpec(
            id = "sms_contacts",
            titleRes = R.string.walkthrough_perm_sms_title,
            whatItDoesRes = R.string.walkthrough_perm_sms_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_sms_requires,
            optional = false,
            granted = state.smsAccess && state.contactsAccess,
            onGrant = {
                smsContactsLauncher.launch(PhoneSms.permissions + PhoneContacts.permissions)
            },
            onManage = { appInfo() },
            restrictable = Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM,
        ),
        WalkthroughPermissionSpec(
            id = "photos",
            titleRes = R.string.walkthrough_perm_photos_title,
            whatItDoesRes = R.string.walkthrough_perm_photos_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_photos_requires,
            optional = false,
            granted = state.photoAccess,
            onGrant = {
                photosLauncher.launch(RecentPhotos.permissions)
            },
            onManage = { appInfo() },
        ),
        WalkthroughPermissionSpec(
            id = "storage",
            titleRes = R.string.walkthrough_perm_storage_title,
            whatItDoesRes = R.string.walkthrough_perm_storage_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_storage_requires,
            optional = true,
            granted = state.storageAllFilesAccess || state.storageSafFolders.isNotEmpty(),
            onGrant = {
                runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }
            },
            onManage = { runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) } },
        ),
        WalkthroughPermissionSpec(
            id = "accessibility",
            titleRes = R.string.walkthrough_perm_access_title,
            whatItDoesRes = R.string.walkthrough_perm_access_does,
            whatAndroidRequiresRes = R.string.walkthrough_perm_access_requires,
            optional = true,
            granted = state.inputAccess || state.elevated,
            onGrant = {
                runCatching { context.startActivity(InputService.settingsIntent()) }
            },
            onManage = { runCatching { context.startActivity(InputService.settingsIntent()) } },
            restrictable = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU,
        ),
    )

    val safeIndex = activeCardIndex.coerceIn(0, specs.lastIndex)
    val currentSpec = specs[safeIndex]
    val grantedCount = specs.count { it.granted }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = stringResource(R.string.walkthrough_permissions_title),
                style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.weight(1f).padding(end = 12.dp),
            )
            StatusPillBadge(
                label = stringResource(R.string.walkthrough_permissions_granted_badge, grantedCount, specs.size),
                active = grantedCount > 0,
            )
        }
        Text(
            text = stringResource(R.string.walkthrough_permissions_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 24.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.surfaceContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // Compact horizontal row of 7 interactive permission selector pills
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                specs.forEachIndexed { idx, spec ->
                    val isSelected = idx == safeIndex
                    val statusLabel = stringResource(
                        if (spec.granted) R.string.walkthrough_perm_status_granted else R.string.walkthrough_perm_status_not_granted,
                    )
                    val dotDesc = stringResource(
                        R.string.walkthrough_perm_dot_desc,
                        stringResource(spec.titleRes),
                        statusLabel,
                    )
                    val containerColor = when {
                        isSelected -> MaterialTheme.colorScheme.primary
                        spec.granted -> MaterialTheme.colorScheme.primaryContainer
                        else -> MaterialTheme.colorScheme.surfaceContainerHighest
                    }
                    val contentColor = when {
                        isSelected -> MaterialTheme.colorScheme.onPrimary
                        spec.granted -> MaterialTheme.colorScheme.onPrimaryContainer
                        else -> MaterialTheme.colorScheme.onSurfaceVariant
                    }
                    Surface(
                        shape = CircleShape,
                        color = containerColor,
                        border = if (isSelected) {
                            BorderStroke(1.5.dp, MaterialTheme.colorScheme.primary)
                        } else {
                            null
                        },
                        modifier = Modifier
                            .weight(1f)
                            .height(32.dp)
                            .semantics { contentDescription = dotDesc }
                            .clickable(role = Role.Tab) { activeCardIndex = idx },
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            if (spec.granted) {
                                val checkPath = remember { Path() }
                                Canvas(modifier = Modifier.size(14.dp)) {
                                    drawCheckGlyph(
                                        path = checkPath,
                                        center = Offset(size.width * 0.5f, size.height * 0.5f),
                                        size = size.minDimension * 0.42f,
                                        color = contentColor,
                                        strokeWidth = 2.dp.toPx(),
                                    )
                                }
                            } else {
                                Text(
                                    text = stringResource(R.string.walkthrough_perm_pill_number, idx + 1),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = contentColor,
                                )
                            }
                        }
                    }
                }
            }

            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = stringResource(
                        if (currentSpec.optional) R.string.walkthrough_perm_counter_optional else R.string.walkthrough_perm_counter,
                        safeIndex + 1,
                        specs.size,
                    ),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f).padding(end = 12.dp),
                )
                StatusPillBadge(
                    label = stringResource(
                        if (currentSpec.granted) R.string.walkthrough_perm_status_granted else R.string.walkthrough_perm_status_not_granted,
                    ),
                    active = currentSpec.granted,
                )
            }

            Text(
                text = stringResource(currentSpec.titleRes),
                style = MaterialTheme.typography.titleLarge,
            )
            Text(
                text = stringResource(currentSpec.whatItDoesRes),
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                text = stringResource(currentSpec.whatAndroidRequiresRes),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (currentSpec.restrictable &&
                sideloaded &&
                !currentSpec.granted &&
                currentSpec.id in attempted.split(',')
            ) {
                RestrictedSettingsHelp(onOpenAppInfo = { appInfo() })
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = {
                        if (currentSpec.granted) {
                            currentSpec.onManage()
                        } else {
                            if (currentSpec.id !in attempted.split(',')) attempted += ",${currentSpec.id}"
                            currentSpec.onGrant()
                        }
                    },
                    modifier = Modifier.weight(1f),
                ) {
                    Text(
                        stringResource(
                            if (currentSpec.granted) R.string.walkthrough_perm_action_manage else R.string.walkthrough_perm_action_grant,
                        ),
                    )
                }
                if (safeIndex < specs.lastIndex) {
                    FilledTonalButton(
                        onClick = { activeCardIndex = safeIndex + 1 },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.walkthrough_perm_action_next))
                    }
                }
            }
        }
    }
}

/**
 * Android holds some settings back for apps installed from a file. Steps to
 * allow them from App info (the menu item appears once a grant was tried).
 */
@Composable
private fun RestrictedSettingsHelp(onOpenAppInfo: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.secondaryContainer, RoundedCornerShape(16.dp))
            .padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = stringResource(R.string.walkthrough_restricted_title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
        Text(
            text = stringResource(R.string.walkthrough_restricted_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
        listOf(
            R.string.walkthrough_restricted_step1,
            R.string.walkthrough_restricted_step2,
            R.string.walkthrough_restricted_step3,
        ).forEachIndexed { i, step ->
            Text(
                text = "${i + 1}. " + stringResource(step),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
        }
        Text(
            text = stringResource(R.string.walkthrough_restricted_note),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.8f),
        )
        OutlinedButton(onClick = onOpenAppInfo) {
            Text(stringResource(R.string.walkthrough_restricted_open))
        }
    }
}

@Composable
private fun PairStepContent(
    state: CoreState,
    actions: PairingActions,
    isGlass: Boolean,
    onContinue: () -> Unit,
) {
    var showQrScanner by rememberSaveable { mutableStateOf(false) }

    DisposableEffect(Unit) {
        actions.visible(true)
        onDispose { actions.visible(false) }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = stringResource(R.string.walkthrough_pair_title),
            style = MaterialTheme.typography.headlineSmall,
        )
        Text(
            text = stringResource(R.string.walkthrough_pair_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 24.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.surfaceContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (state.devices.isNotEmpty()) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = stringResource(R.string.walkthrough_pair_ready_title),
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    StatusPillBadge(
                        label = stringResource(R.string.walkthrough_pair_badge_paired),
                        active = true,
                    )
                }
                state.devices.forEach { dev ->
                    Text(
                        text = stringResource(
                            if (dev.online) R.string.walkthrough_pair_device_online else R.string.walkthrough_pair_device_offline,
                            dev.name,
                        ),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            }

            when (val p = state.pairing) {
                PairingState.Idle -> {
                    Button(
                        onClick = { showQrScanner = !showQrScanner },
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 46.dp),
                    ) {
                        Text(
                            stringResource(
                                if (showQrScanner) R.string.walkthrough_pair_hide_qr else R.string.walkthrough_pair_scan_qr,
                            ),
                        )
                    }

                    if (showQrScanner) {
                        Text(
                            text = stringResource(R.string.pair_scan_hint),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        QrScanner(
                            onLink = actions::join,
                            modifier = Modifier
                                .fillMaxWidth()
                                .aspectRatio(1f)
                                .clip(MaterialTheme.shapes.extraLarge)
                                .background(MaterialTheme.colorScheme.surfaceContainerHigh),
                        )
                    }

                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

                    Text(
                        text = stringResource(R.string.walkthrough_pair_nearby_heading),
                        style = MaterialTheme.typography.titleSmall,
                    )
                    if (state.discovered.isEmpty()) {
                        Text(
                            text = stringResource(R.string.pair_looking),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    } else {
                        state.discovered.forEach { pc ->
                            NearbyPcWalkthroughRow(pc = pc, onPair = { actions.pairNearby(pc.id) })
                        }
                    }

                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

                    PrivacyPointRow(
                        title = stringResource(R.string.walkthrough_pair_usb_title),
                        detail = stringResource(R.string.walkthrough_pair_usb_hint),
                    )
                }
                PairingState.Joining -> {
                    PairingStatusCard(
                        title = stringResource(R.string.pair_joining),
                        showSpinner = true,
                        actionLabel = stringResource(R.string.action_cancel),
                        onAction = actions::reset,
                    )
                }
                is PairingState.Connecting -> {
                    val peer = state.nameOf(p.peer) ?: stringResource(R.string.your_pc)
                    PairingStatusCard(
                        title = stringResource(R.string.pair_connecting, peer),
                        showSpinner = true,
                        actionLabel = stringResource(R.string.action_cancel),
                        onAction = actions::reset,
                    )
                }
                is PairingState.Comparing -> {
                    val peer = state.nameOf(p.peer) ?: stringResource(R.string.your_pc)
                    VerificationCodeWalkthroughCard(
                        peer = peer,
                        code = p.code,
                        confirmed = false,
                        onMatch = { actions.confirm(true) },
                        onReject = { actions.confirm(false) },
                    )
                }
                is PairingState.Confirmed -> {
                    val peer = state.nameOf(p.peer) ?: stringResource(R.string.your_pc)
                    VerificationCodeWalkthroughCard(
                        peer = peer,
                        code = p.code,
                        confirmed = true,
                        onMatch = {},
                        onReject = actions::reset,
                    )
                }
                is PairingState.Paired -> {
                    PairingStatusCard(
                        title = stringResource(R.string.pair_done, p.name),
                        showSpinner = false,
                        actionLabel = stringResource(R.string.walkthrough_action_continue),
                        onAction = {
                            actions.reset()
                            onContinue()
                        },
                    )
                }
                is PairingState.Failed -> {
                    val reason = when (p.failure) {
                        PairingFailure.Rejected -> stringResource(R.string.pair_failed_rejected)
                        PairingFailure.Declined -> stringResource(R.string.pair_failed_declined)
                        PairingFailure.Expired -> stringResource(R.string.pair_failed_expired)
                        PairingFailure.Unreachable -> stringResource(R.string.pair_failed_unreachable)
                        is PairingFailure.Other -> stringResource(R.string.pair_failed_other)
                    }
                    PairingStatusCard(
                        title = reason,
                        showSpinner = false,
                        actionLabel = stringResource(R.string.action_try_again),
                        onAction = actions::reset,
                    )
                }
            }
        }
    }
}

@Composable
private fun NearbyPcWalkthroughRow(
    pc: DiscoveredDevice,
    onPair: () -> Unit,
) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onPair),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                Text(pc.name ?: stringResource(R.string.your_pc), style = MaterialTheme.typography.titleSmall)
                Text(
                    text = stringResource(R.string.walkthrough_pair_nearby_tap),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            FilledTonalButton(onClick = onPair) {
                Text(stringResource(R.string.action_pair))
            }
        }
    }
}

@Composable
private fun VerificationCodeWalkthroughCard(
    peer: String,
    code: String,
    confirmed: Boolean,
    onMatch: () -> Unit,
    onReject: () -> Unit,
) {
    val spaced = remember(code) { code.chunked(3).joinToString(" ") }
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                text = stringResource(R.string.pair_compare_hint, peer),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            Text(
                text = spaced,
                style = MaterialTheme.typography.displaySmall.copy(
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.SemiBold,
                ),
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            if (confirmed) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
                Text(
                    text = stringResource(R.string.pair_waiting, peer),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
                TextButton(onClick = onReject) {
                    Text(stringResource(R.string.action_cancel))
                }
            } else {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    OutlinedButton(onClick = onReject, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_codes_differ))
                    }
                    Button(onClick = onMatch, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_codes_match))
                    }
                }
            }
        }
    }
}

@Composable
private fun PairingStatusCard(
    title: String,
    showSpinner: Boolean,
    actionLabel: String,
    onAction: () -> Unit,
) {
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (showSpinner) {
                CircularProgressIndicator(modifier = Modifier.size(28.dp), strokeWidth = 2.5.dp)
            }
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
            )
            FilledTonalButton(onClick = onAction) {
                Text(actionLabel)
            }
        }
    }
}

@Composable
private fun PowerLevelStepContent(
    state: CoreState,
    isGlass: Boolean,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var showElevatedGuide by rememberSaveable { mutableStateOf(false) }

    val activeTierLabel = stringResource(
        when {
            state.elevated -> R.string.walkthrough_power_elevated_title
            state.inputAccess -> R.string.walkthrough_power_assist_title
            else -> R.string.walkthrough_power_standard_title
        },
    )

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = stringResource(R.string.walkthrough_power_title),
                style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.weight(1f).padding(end = 12.dp),
            )
            StatusPillBadge(
                label = stringResource(R.string.walkthrough_power_current_badge, activeTierLabel),
                active = true,
            )
        }
        Text(
            text = stringResource(R.string.walkthrough_power_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 24.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.surfaceContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // Tier 1: Standard
            PowerLevelRow(
                title = stringResource(R.string.walkthrough_power_standard_title),
                subtitle = stringResource(R.string.walkthrough_power_standard_sub),
                active = !state.elevated && !state.inputAccess,
                summary = stringResource(R.string.walkthrough_power_standard_desc),
                actionLabel = null,
                onAction = null,
            )

            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

            // Tier 2: Assist
            PowerLevelRow(
                title = stringResource(R.string.walkthrough_power_assist_title),
                subtitle = stringResource(R.string.walkthrough_power_assist_sub),
                active = !state.elevated && state.inputAccess,
                summary = stringResource(R.string.walkthrough_power_assist_desc),
                actionLabel = stringResource(
                    if (state.inputAccess) R.string.walkthrough_power_assist_manage else R.string.walkthrough_power_assist_open,
                ),
                onAction = {
                    runCatching { context.startActivity(InputService.settingsIntent()) }
                },
            )

            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

            // Tier 3: Elevated
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_title),
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_sub),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    StatusPillBadge(
                        label = stringResource(
                            if (state.elevated) R.string.walkthrough_power_badge_active else R.string.walkthrough_power_badge_optional,
                        ),
                        active = state.elevated,
                    )
                }
                Text(
                    text = stringResource(R.string.walkthrough_power_elevated_desc),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(
                        onClick = {
                            scope.launch {
                                PairingNotification.show(context)
                                if (!Elevated.start()) {
                                    openDeveloperOptionsFromWalkthrough(context)
                                }
                                onAccessChanged()
                            }
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(
                            stringResource(
                                if (state.elevated) R.string.walkthrough_power_elevated_reconnect else R.string.walkthrough_power_elevated_enable,
                            ),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    OutlinedButton(
                        onClick = { showElevatedGuide = !showElevatedGuide },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(
                            stringResource(
                                if (showElevatedGuide) R.string.walkthrough_power_elevated_hide_steps else R.string.walkthrough_power_elevated_show_steps,
                            ),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                AnimatedVisibility(
                    visible = showElevatedGuide,
                    enter = expandVertically() + fadeIn(),
                    exit = shrinkVertically() + fadeOut(),
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 4.dp)
                            .bentoWell(
                                cornerRadius = 16.dp,
                                fillColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                            )
                            .padding(14.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_steps_heading),
                            style = MaterialTheme.typography.labelLarge,
                        )
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_step1),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_step2),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = stringResource(R.string.walkthrough_power_elevated_step3),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun PowerLevelRow(
    title: String,
    subtitle: String,
    active: Boolean,
    summary: String,
    actionLabel: String?,
    onAction: (() -> Unit)?,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            StatusPillBadge(
                label = stringResource(
                    if (active) R.string.walkthrough_power_badge_active else R.string.walkthrough_power_badge_available,
                ),
                active = active,
            )
        }
        Text(
            text = summary,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (actionLabel != null && onAction != null) {
            // Same width and style as the Elevated row's first button, so the actions line up.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilledTonalButton(onClick = onAction, modifier = Modifier.weight(1f)) {
                    Text(actionLabel, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Spacer(Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun DoneStepContent(
    state: CoreState,
    postNotifGranted: Boolean,
    isGlass: Boolean,
) {
    val noPcText = stringResource(R.string.walkthrough_done_no_pc)
    val pairedText = if (state.devices.isNotEmpty()) {
        state.devices.joinToString(", ") { it.name }
    } else {
        noPcText
    }
    val powerLevelText = stringResource(
        when {
            state.elevated -> R.string.walkthrough_done_power_elevated
            state.inputAccess -> R.string.walkthrough_done_power_assist
            else -> R.string.walkthrough_done_power_standard
        },
    )
    val notifLabel = stringResource(R.string.walkthrough_perm_notif_title)
    val batteryLabel = stringResource(R.string.walkthrough_perm_battery_title)
    val callsLabel = stringResource(R.string.walkthrough_perm_calls_title)
    val smsLabel = stringResource(R.string.walkthrough_perm_sms_title)
    val photosLabel = stringResource(R.string.walkthrough_perm_photos_title)
    val storageLabel = stringResource(R.string.walkthrough_perm_storage_title)
    val assistLabel = stringResource(R.string.walkthrough_perm_access_title)
    val noneGrantedText = stringResource(R.string.walkthrough_done_no_permissions)

    val grantedPermissions = buildList {
        if (state.notificationAccess && postNotifGranted) add(notifLabel)
        if (state.backgroundUnrestricted) add(batteryLabel)
        if (state.callAccess) add(callsLabel)
        if (state.smsAccess && state.contactsAccess) add(smsLabel)
        if (state.photoAccess) add(photosLabel)
        if (state.storageAllFilesAccess || state.storageSafFolders.isNotEmpty()) add(storageLabel)
        if (state.inputAccess) add(assistLabel)
    }
    val permissionsSummary = if (grantedPermissions.isEmpty()) {
        noneGrantedText
    } else {
        grantedPermissions.joinToString(", ")
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = stringResource(R.string.walkthrough_done_title),
            style = MaterialTheme.typography.headlineSmall,
        )
        Text(
            text = stringResource(R.string.walkthrough_done_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = 24.dp,
                    isGlass = isGlass,
                    baseColor = MaterialTheme.colorScheme.surfaceContainer,
                    tintColor = MaterialTheme.colorScheme.primary,
                    outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
                )
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                text = stringResource(R.string.walkthrough_done_summary_heading),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_done_paired_label),
                detail = pairedText,
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_done_power_label),
                detail = powerLevelText,
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            PrivacyPointRow(
                title = stringResource(R.string.walkthrough_done_permissions_label),
                detail = permissionsSummary,
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))
            Text(
                text = stringResource(R.string.walkthrough_done_try_heading),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                text = stringResource(R.string.walkthrough_done_try_1),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = stringResource(R.string.walkthrough_done_try_2),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
internal fun StatusPillBadge(
    label: String,
    active: Boolean,
) {
    val bg = if (active) {
        MaterialTheme.colorScheme.primary.copy(alpha = 0.16f)
    } else {
        MaterialTheme.colorScheme.surfaceContainerHighest
    }
    val fg = if (active) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }
    Text(
        text = label,
        style = MaterialTheme.typography.labelSmall,
        color = fg,
        maxLines = 1,
        softWrap = false,
        modifier = Modifier
            .bentoWell(cornerRadius = 10.dp, fillColor = bg)
            .padding(horizontal = 8.dp, vertical = 4.dp),
    )
}

private fun openDeveloperOptionsFromWalkthrough(context: Context) {
    val devIntent = Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(devIntent) }.isFailure) {
        runCatching {
            context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}
