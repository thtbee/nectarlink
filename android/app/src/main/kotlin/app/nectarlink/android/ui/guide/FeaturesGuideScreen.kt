// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.guide

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.PairingNotification
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.ui.theme.LocalGlassTheme
import app.nectarlink.android.ui.theme.bentoWell
import app.nectarlink.android.ui.theme.glassModalBlur
import app.nectarlink.android.ui.theme.glassModalContainerColor
import app.nectarlink.android.ui.theme.liquidGlassCard
import kotlinx.coroutines.launch

/**
 * Interactive Features Guide screen with search, area filters, live capability badges,
 * "Show me" navigation, and detailed "Learn more" user guide sheets.
 */
@androidx.compose.runtime.Immutable
private data class ResolvedGuideCard(
    val id: String,
    val title: String,
    val summary: String,
    val whereLine: String,
    val needsLine: String,
    val externalSteps: List<String>,
    val showMeTarget: GuideHighlightTarget?,
    val badge: FeatureLiveBadge?,
    val badgeText: String?,
    val fixAction: GuideFixAction?,
    val fixButtonLabel: String?,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FeaturesGuideScreen(
    state: CoreState,
    onBack: () -> Unit,
    onShowMe: (GuideHighlightTarget) -> Unit,
    onOpenWalkthrough: () -> Unit,
    onPairNew: () -> Unit,
    onAccessChanged: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val isGlass = LocalGlassTheme.current
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var selectedAreaId by rememberSaveable { mutableStateOf(FeatureArea.All.id) }
    var activeLearnMoreId by rememberSaveable { mutableStateOf<String?>(null) }

    val hasConnectedPc = state.devices.any { it.online }

    val selectedArea = remember(selectedAreaId) {
        FeatureArea.entries.firstOrNull { it.id == selectedAreaId } ?: FeatureArea.All
    }
    val filteredEntries = remember(context, selectedArea, searchQuery, state.hideElevated) {
        FeaturesGuideCatalog.filter(
            context = context,
            area = selectedArea,
            searchQuery = searchQuery,
            includeElevated = !state.hideElevated,
        )
    }
    val resolvedCards = remember(context, filteredEntries, state, hasConnectedPc) {
        val res = context.resources
        filteredEntries.map { entry ->
            val badge = entry.evaluateBadge(state)
            val showBadge = hasConnectedPc ||
                badge is FeatureLiveBadge.NeedsPermission ||
                badge is FeatureLiveBadge.NeedsPowerLevel
            val badgeText = if (showBadge) {
                when (badge) {
                    is FeatureLiveBadge.Available -> res.getString(badge.labelRes)
                    is FeatureLiveBadge.NeedsPermission ->
                        res.getString(badge.labelRes, res.getString(badge.permissionNameRes))
                    is FeatureLiveBadge.NeedsPowerLevel ->
                        res.getString(badge.labelRes, res.getString(badge.levelNameRes))
                    is FeatureLiveBadge.ConnectPcToCheck -> res.getString(badge.labelRes)
                }
            } else {
                null
            }
            val fixAction = entry.evaluateFixAction(state)
            ResolvedGuideCard(
                id = entry.id,
                title = res.getString(entry.titleRes),
                summary = res.getString(entry.summaryRes),
                whereLine = res.getString(R.string.guide_where_prefix, res.getString(entry.whereTextRes)),
                needsLine = res.getString(R.string.guide_needs_prefix, res.getString(entry.needsSummaryRes)),
                externalSteps = entry.externalStepResIds.map { res.getString(it) },
                showMeTarget = entry.showMeTarget,
                badge = if (showBadge) badge else null,
                badgeText = badgeText,
                fixAction = fixAction,
                fixButtonLabel = fixAction?.let { res.getString(it.buttonLabelRes) },
            )
        }
    }
    val activeLearnMoreEntry = remember(activeLearnMoreId) {
        activeLearnMoreId?.let { id -> FeaturesGuideCatalog.entries.firstOrNull { it.id == id } }
    }

    val callsLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { onAccessChanged() }

    val smsContactsLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { onAccessChanged() }

    val photosLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { onAccessChanged() }

    val runFixAction: (GuideFixAction) -> Unit = remember(context, scope, onAccessChanged, onPairNew) {
        { action ->
            when (action) {
                GuideFixAction.GRANT_NOTIFICATIONS -> {
                    runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }
                }
                GuideFixAction.GRANT_BATTERY -> {
                    runCatching { context.startActivity(BackgroundAccess.requestIntent(context)) }
                }
                GuideFixAction.GRANT_CALLS -> {
                    callsLauncher.launch(PhoneCalls.permissions)
                }
                GuideFixAction.GRANT_SMS_CONTACTS -> {
                    smsContactsLauncher.launch(PhoneSms.permissions + PhoneContacts.permissions)
                }
                GuideFixAction.GRANT_PHOTOS -> {
                    photosLauncher.launch(RecentPhotos.permissions)
                }
                GuideFixAction.GRANT_STORAGE -> {
                    runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }
                }
                GuideFixAction.OPEN_ACCESSIBILITY -> {
                    runCatching { context.startActivity(InputService.settingsIntent()) }
                }
                GuideFixAction.ENABLE_ELEVATED -> {
                    scope.launch {
                        if (Elevated.hasSystemService(context)) {
                            Elevated.start()
                        } else {
                            PairingNotification.show(context)
                            if (!Elevated.start()) {
                                openDevOptions(context)
                            }
                        }
                        onAccessChanged()
                    }
                }
                GuideFixAction.ENABLE_DND -> {
                    runCatching { context.startActivity(PhoneToggles.dndSettingsIntent()) }
                }
                GuideFixAction.ENABLE_WRITE_SETTINGS -> {
                    runCatching { context.startActivity(PhoneToggles.writeSettingsIntent(context)) }
                }
                GuideFixAction.PAIR_PC -> {
                    onPairNew()
                }
            }
        }
    }

    val searchDesc = stringResource(R.string.guide_search_desc)
    val showMeLabel = stringResource(R.string.guide_action_show_me)
    val learnMoreLabel = stringResource(R.string.guide_action_learn_more)
    val externalHeading = stringResource(R.string.guide_external_heading)

    BackHandler(onBack = onBack)

    Surface(
        modifier = modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .glassModalBlur(isGlass && activeLearnMoreEntry != null),
            contentPadding = PaddingValues(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 36.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            item(key = "guide_header", contentType = "guide_header") {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        TextButton(
                            onClick = onBack,
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                        ) {
                            Text(stringResource(R.string.guide_back))
                        }
                        TextButton(
                            onClick = onOpenWalkthrough,
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                        ) {
                            Text(stringResource(R.string.guide_open_walkthrough))
                        }
                    }
                    Text(
                        text = stringResource(R.string.guide_title),
                        style = MaterialTheme.typography.headlineMedium,
                    )
                    Text(
                        text = stringResource(R.string.guide_subtitle),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )

                    if (!hasConnectedPc) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .bentoWell(
                                    cornerRadius = 16.dp,
                                    fillColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                                )
                                .padding(horizontal = 14.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Text(
                                text = stringResource(R.string.guide_connect_pc_banner),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.weight(1f),
                            )
                            if (state.devices.isEmpty()) {
                                TextButton(
                                    onClick = onPairNew,
                                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 2.dp),
                                ) {
                                    Text(stringResource(R.string.guide_fix_pair))
                                }
                            }
                        }
                    }

                    OutlinedTextField(
                        value = searchQuery,
                        onValueChange = { searchQuery = it },
                        placeholder = { Text(stringResource(R.string.guide_search_placeholder)) },
                        singleLine = true,
                        modifier = Modifier
                            .fillMaxWidth()
                            .semantics { contentDescription = searchDesc },
                    )
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FeatureArea.entries.forEach { area ->
                            val selected = area == selectedArea
                            val chipBg = if (selected) {
                                MaterialTheme.colorScheme.primaryContainer
                            } else {
                                Color.Transparent
                            }
                            val chipBorder = if (selected) {
                                Color.Transparent
                            } else {
                                MaterialTheme.colorScheme.outlineVariant
                            }
                            val chipFg = if (selected) {
                                MaterialTheme.colorScheme.onPrimaryContainer
                            } else {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            }
                            Box(
                                modifier = Modifier
                                    .bentoWell(
                                        cornerRadius = 10.dp,
                                        fillColor = chipBg,
                                        borderColor = chipBorder,
                                    )
                                    .clickable(role = Role.Tab) { selectedAreaId = area.id }
                                    .padding(horizontal = 14.dp, vertical = 8.dp),
                                contentAlignment = Alignment.Center,
                            ) {
                                Text(
                                    text = stringResource(area.labelRes),
                                    style = MaterialTheme.typography.labelLarge,
                                    color = chipFg,
                                )
                            }
                        }
                    }
                }
            }

            if (resolvedCards.isEmpty()) {
                item(key = "guide_empty", contentType = "guide_empty") {
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
                            .padding(20.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            text = stringResource(R.string.guide_empty_title),
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Text(
                            text = stringResource(R.string.guide_empty_body),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            } else {
                items(
                    items = resolvedCards,
                    key = { it.id },
                    contentType = {
                        (if (it.badge != null) 1 else 0) or
                            (if (it.externalSteps.isNotEmpty()) 2 else 0) or
                            (if (it.fixAction != null) 4 else 0)
                    },
                ) { card ->
                    FeatureGuideCard(
                        card = card,
                        isGlass = isGlass,
                        showMeLabel = showMeLabel,
                        learnMoreLabel = learnMoreLabel,
                        externalHeading = externalHeading,
                        onShowMe = onShowMe,
                        onRunFix = runFixAction,
                        onLearnMore = { activeLearnMoreId = card.id },
                    )
                }
            }
        }
    }

    if (activeLearnMoreEntry != null) {
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = { activeLearnMoreId = null },
            sheetState = sheetState,
            containerColor = glassModalContainerColor(),
        ) {
            FeatureLearnMoreSheetContent(
                entry = activeLearnMoreEntry,
                state = state,
                hasConnectedPc = hasConnectedPc,
                onShowMe = { target ->
                    activeLearnMoreId = null
                    onShowMe(target)
                },
                onRunFix = runFixAction,
                onClose = { activeLearnMoreId = null },
            )
        }
    }
}

@Composable
private fun FeatureGuideCard(
    card: ResolvedGuideCard,
    isGlass: Boolean,
    showMeLabel: String,
    learnMoreLabel: String,
    externalHeading: String,
    onShowMe: (GuideHighlightTarget) -> Unit,
    onRunFix: (GuideFixAction) -> Unit,
    onLearnMore: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .liquidGlassCard(
                cornerRadius = 26.dp,
                isGlass = isGlass,
                baseColor = MaterialTheme.colorScheme.surfaceContainer,
                tintColor = MaterialTheme.colorScheme.primary,
                outlineColor = if (isGlass) MaterialTheme.colorScheme.outlineVariant else Color.Transparent,
            )
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // Title on its own full-width line, with status badge directly underneath when shown
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                text = card.title,
                style = MaterialTheme.typography.titleMedium,
            )
            if (card.badge != null && card.badgeText != null) {
                FeatureBadgeChip(badge = card.badge, text = card.badgeText)
            }
        }

        Text(
            text = card.summary,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .bentoWell(
                    cornerRadius = 16.dp,
                    fillColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                )
                .padding(horizontal = 14.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = card.whereLine,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = card.needsLine,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (card.externalSteps.isNotEmpty()) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .bentoWell(
                        cornerRadius = 16.dp,
                        fillColor = MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.65f),
                    )
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    text = externalHeading,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
                card.externalSteps.forEach { stepText ->
                    Text(
                        text = stepText,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                }
            }
        }

        // Single row of actions: "Show me" (tonal) and "Learn more" (text), plus filled action button when needed
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (card.showMeTarget != null) {
                Box(
                    modifier = Modifier
                        .bentoWell(
                            cornerRadius = 20.dp,
                            fillColor = MaterialTheme.colorScheme.secondaryContainer,
                        )
                        .clickable(role = Role.Button) { onShowMe(card.showMeTarget) }
                        .padding(horizontal = 16.dp, vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = showMeLabel,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                }
            }
            Box(
                modifier = Modifier
                    .clickable(role = Role.Button, onClick = onLearnMore)
                    .padding(horizontal = 12.dp, vertical = 9.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = learnMoreLabel,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            Spacer(Modifier.weight(1f))
            if (card.fixAction != null && card.fixButtonLabel != null) {
                Box(
                    modifier = Modifier
                        .bentoWell(
                            cornerRadius = 20.dp,
                            fillColor = MaterialTheme.colorScheme.primary,
                        )
                        .clickable(role = Role.Button) { onRunFix(card.fixAction) }
                        .padding(horizontal = 16.dp, vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = card.fixButtonLabel,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                }
            }
        }
    }
}

@Composable
private fun FeatureBadgeChip(
    badge: FeatureLiveBadge,
    text: String? = null,
) {
    val (bg, fg) = when (badge) {
        is FeatureLiveBadge.Available ->
            MaterialTheme.colorScheme.primary.copy(alpha = 0.16f) to MaterialTheme.colorScheme.primary
        is FeatureLiveBadge.NeedsPermission,
        is FeatureLiveBadge.NeedsPowerLevel,
        -> MaterialTheme.colorScheme.tertiaryContainer to MaterialTheme.colorScheme.onTertiaryContainer
        is FeatureLiveBadge.ConnectPcToCheck ->
            MaterialTheme.colorScheme.surfaceContainerHighest to MaterialTheme.colorScheme.onSurfaceVariant
    }
    val resolvedText = text ?: when (badge) {
        is FeatureLiveBadge.Available -> stringResource(badge.labelRes)
        is FeatureLiveBadge.NeedsPermission ->
            stringResource(badge.labelRes, stringResource(badge.permissionNameRes))
        is FeatureLiveBadge.NeedsPowerLevel ->
            stringResource(badge.labelRes, stringResource(badge.levelNameRes))
        is FeatureLiveBadge.ConnectPcToCheck -> stringResource(badge.labelRes)
    }
    Text(
        text = resolvedText,
        style = MaterialTheme.typography.labelSmall,
        color = fg,
        modifier = Modifier
            .bentoWell(cornerRadius = 10.dp, fillColor = bg)
            .padding(horizontal = 8.dp, vertical = 4.dp),
    )
}

@Composable
private fun FeatureLearnMoreSheetContent(
    entry: FeatureGuideEntry,
    state: CoreState,
    hasConnectedPc: Boolean,
    onShowMe: (GuideHighlightTarget) -> Unit,
    onRunFix: (GuideFixAction) -> Unit,
    onClose: () -> Unit,
) {
    val badge = entry.evaluateBadge(state)
    val fixAction = entry.evaluateFixAction(state)
    val showBadge = hasConnectedPc ||
        badge is FeatureLiveBadge.NeedsPermission ||
        badge is FeatureLiveBadge.NeedsPowerLevel

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .verticalScroll(rememberScrollState())
            .padding(start = 22.dp, end = 22.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            text = stringResource(entry.area.labelRes),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = stringResource(entry.titleRes),
            style = MaterialTheme.typography.headlineSmall,
        )
        if (showBadge) {
            FeatureBadgeChip(badge = badge)
        }
        Text(
            text = stringResource(entry.summaryRes),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_where),
            items = listOf(stringResource(entry.whereTextRes)),
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_needs),
            items = entry.whatYouNeedResIds.map { stringResource(it) },
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_steps_phone),
            items = entry.stepByStepPhoneResIds.mapIndexed { idx, resId ->
                stringResource(R.string.guide_numbered_step, idx + 1, stringResource(resId))
            },
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_steps_pc),
            items = entry.stepByStepPcResIds.mapIndexed { idx, resId ->
                stringResource(R.string.guide_numbered_step, idx + 1, stringResource(resId))
            },
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_options),
            items = entry.optionsExplainedResIds.map { stringResource(it) },
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_privacy),
            items = listOf(stringResource(entry.dataAndPrivacyRes)),
        )

        GuideSectionBlock(
            heading = stringResource(R.string.guide_sheet_limits),
            items = listOf(stringResource(entry.limitsRes)),
        )

        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f))

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (entry.showMeTarget != null) {
                FilledTonalButton(onClick = { onShowMe(entry.showMeTarget) }) {
                    Text(stringResource(R.string.guide_action_show_me))
                }
            }
            if (fixAction != null) {
                Button(onClick = { onRunFix(fixAction) }) {
                    Text(stringResource(fixAction.buttonLabelRes))
                }
            }
            Spacer(Modifier.weight(1f))
            OutlinedButton(onClick = onClose) {
                Text(stringResource(R.string.guide_action_done))
            }
        }
    }
}

@Composable
private fun GuideSectionBlock(
    heading: String,
    items: List<String>,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = heading,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
        )
        items.forEach { line ->
            Text(
                text = line,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

private fun openDevOptions(context: Context) {
    val devIntent = Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(devIntent) }.isFailure) {
        runCatching {
            context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}
