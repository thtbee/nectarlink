// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import app.nectarlink.android.ui.AppLanguage
import app.nectarlink.android.ui.StarNotification
import androidx.compose.material3.RadioButton
import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import android.app.Activity
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.companion.PcCompanion
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.PairingNotification
import app.nectarlink.android.elevated.ElevatedMethod
import app.nectarlink.android.elevated.ShizukuElevated
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.CheckForUpdates
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import androidx.compose.ui.draw.drawBehind
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.ui.guide.GuideHighlightTarget
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.LocalGlassTheme
import app.nectarlink.android.ui.theme.Tokens
import app.nectarlink.android.ui.theme.glassModalBlur
import app.nectarlink.android.ui.theme.glassModalContainerColor
import app.nectarlink.android.ui.theme.liquidGlassCard
import kotlinx.coroutines.delay

@Composable
fun SettingsScreen(
    state: CoreState,
    appearance: Appearance,
    onAppearance: ((Appearance) -> Appearance) -> Unit,
    touchpadSensitivity: Float,
    onTouchpadSensitivity: (Float) -> Unit,
    onUnpair: (String) -> Unit,
    onPairNew: () -> Unit,
    onAccessChanged: () -> Unit,
    updater: AppUpdater,
    onSetStorageEnabled: (pcId: String, enabled: Boolean) -> Unit = { _, _ -> },
    onSetMicEnabled: (pcId: String, enabled: Boolean) -> Unit = { _, _ -> },
    startWithoutAsking: Set<String> = emptySet(),
    onSetStartWithoutAsking: (pcId: String, on: Boolean) -> Unit = { _, _ -> },
    onAddSafFolder: (Uri) -> Unit = {},
    onRemoveSafFolder: (String) -> Unit = {},
    onSetClipboardHistoryEnabled: (Boolean) -> Unit = {},
    onClearClipboardHistory: () -> Unit = {},
    suggestClipboardActions: Boolean = true,
    onSetSuggestClipboardActions: (Boolean) -> Unit = {},
    volumeKeysControlPc: Boolean = true,
    onSetVolumeKeysControlPc: (Boolean) -> Unit = {},
    backgroundServiceEnabled: Boolean = true,
    onToggleBackgroundService: (Boolean) -> Unit = {},
    onSetTimelineRetentionDays: (UInt) -> Unit = {},
    onClearTimeline: () -> Unit = {},
    onClearMessageCache: () -> Unit = {},
    onClearPhotoCache: () -> Unit = {},
    onClearReceivedFileHistory: () -> Unit = {},
    onClearEverything: () -> Unit = {},
    onToggleLocalSend: (Boolean) -> Unit = {},
    onToggleAwayMode: (Boolean) -> Unit = {},
    onOpenRelaySettings: () -> Unit = {},
    onOpenWalkthrough: () -> Unit = {},
    onOpenFeaturesGuide: () -> Unit = {},
    highlightTarget: GuideHighlightTarget? = null,
    onHighlightConsumed: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var confirmUnpair by remember { mutableStateOf<Device?>(null) }
    var confirmClearEverything by remember { mutableStateOf(false) }
    var pickLanguage by remember { mutableStateOf(false) }
    var activeCategory by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf("all") }
    var pulsingSectionKey by remember { mutableStateOf<String?>(null) }
    var jumpToControl by remember { mutableStateOf(0) }
    val isGlass = LocalGlassTheme.current
    val anyDialogOpen = confirmUnpair != null || confirmClearEverything

    val categories = remember {
        listOf(
            "all" to R.string.timeline_filter_all,
            "appearance" to R.string.settings_appearance,
            "devices" to R.string.settings_category_devices,
            "permissions" to R.string.walkthrough_permissions_title,
            "features" to R.string.settings_category_features,
        )
    }

    val showAppearance = activeCategory == "all" || activeCategory == "appearance"
    val showDevices = activeCategory == "all" || activeCategory == "devices"
    val showPermissions = activeCategory == "all" || activeCategory == "permissions"
    val showFeatures = activeCategory == "all" || activeCategory == "features"

    // Keep a screen of cards composed on each side, so scrolling back does not rebuild them.
    @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
    val listState = androidx.compose.foundation.lazy.rememberLazyListState(
        cacheWindow = androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow(aheadFraction = 1f, behindFraction = 1f),
    )

    androidx.compose.runtime.LaunchedEffect(jumpToControl) {
        if (jumpToControl == 0) return@LaunchedEffect
        activeCategory = "permissions"
        pulsingSectionKey = "control"
        delay(60)
        runCatching { listState.animateScrollToItem(2) }
        delay(2200)
        if (pulsingSectionKey == "control") pulsingSectionKey = null
    }

    androidx.compose.runtime.LaunchedEffect(highlightTarget) {
        val target = highlightTarget ?: return@LaunchedEffect
        val mapping = when (target) {
            GuideHighlightTarget.SETTINGS_DEVICES -> Triple("devices", "devices", 1)
            GuideHighlightTarget.SETTINGS_NOTIFICATIONS -> Triple("permissions", "notifications", 1)
            GuideHighlightTarget.SETTINGS_CONTROL -> Triple("permissions", "control", 2)
            GuideHighlightTarget.SETTINGS_BACKGROUND -> Triple("permissions", "background", 3)
            GuideHighlightTarget.SETTINGS_CALLS_CONTACTS -> Triple("permissions", "calls_contacts", 4)
            GuideHighlightTarget.SETTINGS_PHOTOS_STORAGE -> Triple("permissions", "photos", 5)
            GuideHighlightTarget.SETTINGS_CLIPBOARD -> Triple("features", "clipboard", 1)
            GuideHighlightTarget.SETTINGS_LOCALSEND -> Triple("features", "localsend", 3)
            GuideHighlightTarget.SETTINGS_DATA_STORAGE -> Triple("features", "data_storage", 4)
            else -> null
        }
        if (mapping != null) {
            val (category, sectionKey, indexInCat) = mapping
            activeCategory = category
            pulsingSectionKey = sectionKey
            onHighlightConsumed()
            delay(60)
            runCatching { listState.animateScrollToItem(indexInCat) }
            delay(2200)
            if (pulsingSectionKey == sectionKey) {
                pulsingSectionKey = null
            }
        } else {
            onHighlightConsumed()
        }
    }

    LazyColumn(
        state = listState,
        modifier = modifier.glassModalBlur(isGlass && anyDialogOpen),
        contentPadding = PaddingValues(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 104.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item(key = "header", contentType = "header") {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        stringResource(R.string.settings_title),
                        style = MaterialTheme.typography.headlineMedium,
                    )
                    FilledTonalButton(onClick = onOpenFeaturesGuide) {
                        Text(stringResource(R.string.guide_title))
                    }
                }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    categories.forEach { (id, label) ->
                        val selected = activeCategory == id
                        FilterChip(
                            selected = selected,
                            onClick = { activeCategory = id },
                            label = { Text(stringResource(label)) },
                            // Solid like the cards: the row sits right on the
                            // background (Hive's comb showed through).
                            colors = FilterChipDefaults.filterChipColors(
                                containerColor = MaterialTheme.colorScheme.surfaceContainer,
                                selectedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                                selectedLabelColor = MaterialTheme.colorScheme.onPrimaryContainer,
                            ),
                        )
                    }
                }
            }
        }

        if (showAppearance) {
            item(key = "appearance", contentType = "section") {
                Section(stringResource(R.string.settings_appearance)) {
                    Label(stringResource(R.string.settings_theme))
                    Choice(
                        listOf(
                            "hive" to stringResource(R.string.theme_hive),
                            "bloom" to stringResource(R.string.theme_bloom),
                            "glass" to "Glass",
                            "graphite" to stringResource(R.string.theme_graphite),
                        ),
                        appearance.theme,
                    ) { value -> onAppearance { it.copy(theme = value) } }
                    Label(stringResource(R.string.settings_mode))
                    Choice(
                        listOf(
                            "system" to stringResource(R.string.mode_system),
                            "light" to stringResource(R.string.mode_light),
                            "dark" to stringResource(R.string.mode_dark),
                        ),
                        appearance.mode,
                    ) { value -> onAppearance { it.copy(mode = value) } }
                    if (AppLanguage.supported) {
                        // System and English, plus the language picked from the list (or More).
                        val context = LocalContext.current
                        val language = remember { AppLanguage.current(context) }
                        val other = AppLanguage.languages.firstOrNull { it.first == language && it.first != "en" }
                        Label(stringResource(R.string.settings_language))
                        Choice(
                            listOf(
                                "" to stringResource(R.string.mode_system),
                                "en" to "English",
                                (other ?: (MORE_LANGUAGES to stringResource(R.string.settings_language_more))),
                            ),
                            language,
                        ) { value ->
                            if (value == "" || value == "en") AppLanguage.set(context, value) else pickLanguage = true
                        }
                    }
                    if (appearance.theme == "hive") {
                        SettingRowDivider()
                        val combTitle = stringResource(R.string.settings_comb_motion)
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                Text(combTitle, style = MaterialTheme.typography.titleSmall)
                                Text(
                                    stringResource(R.string.settings_comb_motion_hint),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Switch(
                                checked = appearance.combMotion,
                                onCheckedChange = { on -> onAppearance { it.copy(combMotion = on) } },
                                colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                modifier = Modifier.semantics { contentDescription = combTitle },
                            )
                        }
                    }
                    if (appearance.theme == "graphite" && appearance.mode != "light") {
                        SettingRowDivider()
                        val amoledTitle = stringResource(R.string.settings_amoled)
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                Text(amoledTitle, style = MaterialTheme.typography.titleSmall)
                                Text(
                                    stringResource(R.string.settings_amoled_hint),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Switch(
                                checked = appearance.amoled,
                                onCheckedChange = { on -> onAppearance { it.copy(amoled = on) } },
                                colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                modifier = Modifier.semantics { contentDescription = amoledTitle },
                            )
                        }
                    }
                    if (appearance.theme == "bloom" || appearance.theme == "glass") {
                        SettingRowDivider()
                        val artworkSync = appearance.seed == "artwork"
                        val artworkColorLabel = stringResource(R.string.settings_theme_artwork_color)
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                Text(stringResource(R.string.settings_theme_artwork_color), style = MaterialTheme.typography.titleSmall)
                                Text(
                                    stringResource(R.string.settings_theme_artwork_body),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Switch(
                                checked = artworkSync,
                                onCheckedChange = { on ->
                                    onAppearance {
                                        it.copy(seed = if (on) "artwork" else Tokens.DEFAULT_SEED)
                                    }
                                },
                                colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                modifier = Modifier.semantics { contentDescription = artworkColorLabel },
                            )
                        }
                        if (!artworkSync && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                            SettingRowDivider()
                            val dynamicColorTitle = stringResource(R.string.settings_dynamic_color)
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(dynamicColorTitle, style = MaterialTheme.typography.titleSmall)
                                    Text(
                                        stringResource(R.string.settings_dynamic_color_hint),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(
                                    checked = appearance.dynamicColor,
                                    onCheckedChange = { on -> onAppearance { it.copy(dynamicColor = on) } },
                                    colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                    modifier = Modifier.semantics { contentDescription = dynamicColorTitle },
                                )
                            }
                        }
                        if (!artworkSync) {
                            SettingRowDivider()
                            val blackSwatchSelected = (!appearance.dynamicColor || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) && appearance.seed == "black"
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .horizontalScroll(rememberScrollState()),
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Tokens.bloomSeeds.forEach { (name, seed) ->
                                    val isSelected = (!appearance.dynamicColor || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) && appearance.seed == name
                                    val seedLabel = stringResource(
                                        when (name) {
                                            "honey" -> R.string.settings_seed_honey
                                            "lavender" -> R.string.settings_seed_lavender
                                            "ocean" -> R.string.settings_seed_ocean
                                            "sage" -> R.string.settings_seed_sage
                                            "black" -> R.string.settings_seed_black
                                            else -> R.string.settings_seed_honey
                                        },
                                    )
                                    val isBlackSeed = name == "black"
                                    val borderColor = when {
                                        isSelected -> MaterialTheme.colorScheme.onSurface
                                        isBlackSeed -> androidx.compose.ui.graphics.Color.White.copy(alpha = 0.28f)
                                        else -> MaterialTheme.colorScheme.outlineVariant
                                    }
                                    val borderWidth = when {
                                        isSelected -> 2.5.dp
                                        isBlackSeed -> 1.2.dp
                                        else -> 1.dp
                                    }
                                    androidx.compose.foundation.layout.Box(
                                        contentAlignment = Alignment.Center,
                                        modifier = Modifier
                                            .size(38.dp)
                                            .drawBehind {
                                                drawCircle(color = seed.color)
                                                val strokePx = borderWidth.toPx()
                                                drawCircle(
                                                    color = borderColor,
                                                    radius = (size.minDimension - strokePx) * 0.5f,
                                                    style = androidx.compose.ui.graphics.drawscope.Stroke(width = strokePx),
                                                )
                                            }
                                            .clickable {
                                                onAppearance {
                                                    it.copy(dynamicColor = false, seed = name)
                                                }
                                            }
                                            .semantics {
                                                role = Role.RadioButton
                                                selected = isSelected
                                                contentDescription = seedLabel
                                            },
                                    ) {
                                        if (isSelected) {
                                            Text(
                                                text = "✓",
                                                style = MaterialTheme.typography.labelLarge,
                                                color = if (isBlackSeed) androidx.compose.ui.graphics.Color.White else androidx.compose.ui.graphics.Color.Black,
                                            )
                                        }
                                    }
                                }

                                // Compact inline AMOLED toggle pill inside the swatch row when Black seed is selected
                                if (blackSwatchSelected) {
                                    val amoledLabel = stringResource(R.string.settings_amoled)
                                    Surface(
                                        shape = RoundedCornerShape(20.dp),
                                        color = MaterialTheme.colorScheme.surfaceContainerHigh,
                                        border = BorderStroke(
                                            1.dp,
                                            if (appearance.amoled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant,
                                        ),
                                        modifier = Modifier.height(38.dp),
                                    ) {
                                        Row(
                                            modifier = Modifier.padding(start = 12.dp, end = 6.dp),
                                            verticalAlignment = Alignment.CenterVertically,
                                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                                        ) {
                                            Text(
                                                text = "AMOLED",
                                                style = MaterialTheme.typography.labelMedium,
                                                color = if (appearance.amoled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                                            )
                                            Switch(
                                                checked = appearance.amoled,
                                                onCheckedChange = { on -> onAppearance { it.copy(amoled = on) } },
                                                colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                                modifier = Modifier
                                                    .graphicsLayer {
                                                        scaleX = 0.76f
                                                        scaleY = 0.76f
                                                    }
                                                    .semantics { contentDescription = amoledLabel },
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if (showDevices) {
            item(key = "devices", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_devices),
                    highlighted = pulsingSectionKey == "devices",
                ) {
                    val context = LocalContext.current
                    state.devices.forEachIndexed { index, device ->
                        if (index > 0) {
                            SettingRowDivider()
                        }
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp), modifier = Modifier.fillMaxWidth()) {
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(device.name, style = MaterialTheme.typography.titleSmall)
                                    val dotColor = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
                                    Row(
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    ) {
                                        Spacer(
                                            modifier = Modifier
                                                .size(8.dp)
                                                .drawBehind { drawCircle(dotColor) },
                                        )
                                        Text(
                                            stringResource(if (device.online) R.string.connected else R.string.not_connected),
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                                OutlinedButton(onClick = { confirmUnpair = device }) { Text(stringResource(R.string.action_unpair)) }
                            }
                            val storageToggleTitle = stringResource(R.string.storage_pc_toggle)
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(storageToggleTitle, style = MaterialTheme.typography.bodyMedium)
                                    Text(
                                        stringResource(R.string.storage_pc_toggle_hint),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(
                                    checked = device.storageEnabled,
                                    onCheckedChange = { enabled ->
                                        onSetStorageEnabled(device.id, enabled)
                                        if (enabled && !state.storageAllFilesAccess && state.storageSafFolders.isEmpty()) {
                                            runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }
                                        }
                                    },
                                    colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                    modifier = Modifier.semantics { contentDescription = storageToggleTitle },
                                )
                            }
                            if (!device.isAndroid) {
                                val micToggleTitle = stringResource(R.string.mic_pc_toggle)
                                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                                    Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                        Text(micToggleTitle, style = MaterialTheme.typography.bodyMedium)
                                        Text(
                                            stringResource(
                                                if (device.id in startWithoutAsking) R.string.mic_pc_toggle_hint_auto else R.string.mic_pc_toggle_hint,
                                            ),
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    Switch(
                                        checked = device.micEnabled,
                                        onCheckedChange = { enabled -> onSetMicEnabled(device.id, enabled) },
                                        colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                        modifier = Modifier.semantics { contentDescription = micToggleTitle },
                                    )
                                }
                                val autoStartTitle = stringResource(R.string.start_without_asking_toggle)
                                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                                    Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                        Text(autoStartTitle, style = MaterialTheme.typography.bodyMedium)
                                        Text(
                                            stringResource(R.string.start_without_asking_hint),
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    Switch(
                                        checked = device.id in startWithoutAsking,
                                        onCheckedChange = { on -> onSetStartWithoutAsking(device.id, on) },
                                        colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                        modifier = Modifier.semantics { contentDescription = autoStartTitle },
                                    )
                                }
                            }
                        }
                    }
                    FilledTonalButton(onClick = onPairNew) { Text(stringResource(R.string.action_pair_new)) }
                    SettingRowDivider()
                    val mobileDataTitle = stringResource(R.string.guide_away_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(mobileDataTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_mobile_data_body),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = state.awayMode,
                            onCheckedChange = onToggleAwayMode,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = mobileDataTitle },
                        )
                    }
                    FilledTonalButton(onClick = onOpenRelaySettings) {
                        Text(stringResource(R.string.relay_settings_title))
                    }
                }
            }
        }

        if (showPermissions) {
            item(key = "notifications", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_notifications),
                    highlighted = pulsingSectionKey == "notifications",
                ) {
                    NotificationAccess(state.notificationAccess)
                }
            }

            item(key = "control", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_control),
                    highlighted = pulsingSectionKey == "control",
                ) {
                    ControlAccess(state.inputAccess)
                    SettingRowDivider()
                    ElevatedAccess()
                    SettingRowDivider()
                    BankingPauseAccess()
                    SettingRowDivider()
                    DndAccess(state.dndAccess)
                    SettingRowDivider()
                    WriteSettingsAccess(state.writeSettingsAccess || state.elevated)
                }
            }

            item(key = "background", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_background_title),
                    highlighted = pulsingSectionKey == "background",
                ) {
                    val runInBackgroundTitle = stringResource(R.string.settings_run_in_background)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(runInBackgroundTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_run_in_background_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = backgroundServiceEnabled,
                            onCheckedChange = onToggleBackgroundService,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = runInBackgroundTitle },
                        )
                    }
                    SettingRowDivider()
                    BackgroundBatteryAccess(state.backgroundUnrestricted)
                    SettingRowDivider()
                    CompanionWakeAccess(
                        associated = state.companionAssociated,
                        devices = state.devices,
                        onAccessChanged = onAccessChanged,
                    )
                }
            }

            item(key = "calls_contacts", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_calls_and_contacts),
                    highlighted = pulsingSectionKey == "calls_contacts",
                ) {
                    RuntimePermissionAccess(
                        title = stringResource(R.string.calls_title),
                        textOn = stringResource(R.string.calls_on),
                        textOff = stringResource(R.string.calls_off),
                        granted = state.callAccess,
                        permissions = PhoneCalls.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    RuntimePermissionAccess(
                        title = stringResource(R.string.contacts_title),
                        textOn = stringResource(R.string.contacts_on),
                        textOff = stringResource(R.string.contacts_off),
                        granted = state.contactsAccess,
                        permissions = PhoneContacts.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    RuntimePermissionAccess(
                        title = stringResource(R.string.sms_title),
                        textOn = stringResource(R.string.sms_on),
                        textOff = stringResource(R.string.sms_off),
                        granted = state.smsAccess,
                        permissions = PhoneSms.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                }
            }

            item(key = "photos", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_photos),
                    highlighted = pulsingSectionKey == "photos",
                ) {
                    PhotosAccess(
                        granted = state.photoAccess,
                        partial = state.photoPartialAccess,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    StorageAccess(
                        allFiles = state.storageAllFilesAccess,
                        safFolders = state.storageSafFolders,
                        onAddSafFolder = onAddSafFolder,
                        onRemoveSafFolder = onRemoveSafFolder,
                    )
                }
            }
        }

        if (showFeatures) {
            item(key = "clipboard", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_clipboard_title),
                    highlighted = pulsingSectionKey == "clipboard",
                ) {
                    val context = LocalContext.current
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.settings_background_clipboard_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                if (state.elevated) {
                                    stringResource(R.string.settings_wireless_debugging_active)
                                } else {
                                    stringResource(R.string.settings_wireless_debugging_hint)
                                },
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        // Elevated's own section knows every way to start it.
                        if (state.elevated) {
                            OutlinedButton(onClick = { jumpToControl++ }) {
                                Text(stringResource(R.string.action_manage))
                            }
                        } else {
                            Button(onClick = { jumpToControl++ }) {
                                Text(stringResource(R.string.elevated_set_up))
                            }
                        }
                    }
                    SettingRowDivider()
                    var syncPasswords by remember { mutableStateOf(PhoneClipboard.syncPasswords(context)) }
                    val syncPasswordsTitle = stringResource(R.string.settings_clipboard_passwords)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(syncPasswordsTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_clipboard_passwords_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = syncPasswords,
                            onCheckedChange = {
                                syncPasswords = it
                                PhoneClipboard.setSyncPasswords(context, it)
                            },
                            modifier = Modifier.semantics { contentDescription = syncPasswordsTitle },
                        )
                    }
                    SettingRowDivider()
                    val suggestionsTitle = stringResource(R.string.settings_clipboard_suggestions)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(suggestionsTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_clipboard_suggestions_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = suggestClipboardActions,
                            onCheckedChange = onSetSuggestClipboardActions,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = suggestionsTitle },
                        )
                    }
                }
            }

            item(key = "remote", contentType = "section") {
                Section(stringResource(R.string.settings_remote)) {
                    val sensitivityTitle = stringResource(R.string.settings_touchpad_sensitivity)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Text(
                            sensitivityTitle,
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            stringResource(R.string.settings_touchpad_sensitivity_value, touchpadSensitivity),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Slider(
                        value = touchpadSensitivity,
                        onValueChange = onTouchpadSensitivity,
                        valueRange = 0.5f..2.5f,
                        modifier = Modifier.semantics { contentDescription = sensitivityTitle },
                    )
                    SettingRowDivider()
                    val volumeKeysTitle = stringResource(R.string.settings_volume_keys_control_pc)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(volumeKeysTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_volume_keys_control_pc_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = volumeKeysControlPc,
                            onCheckedChange = onSetVolumeKeysControlPc,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = volumeKeysTitle },
                        )
                    }
                }
            }

            item(key = "localsend", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_localsend_title),
                    highlighted = pulsingSectionKey == "localsend",
                ) {
                    val localsendTitle = stringResource(R.string.settings_localsend_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(localsendTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_localsend_subtitle),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = state.localsendEnabled,
                            onCheckedChange = onToggleLocalSend,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = localsendTitle },
                        )
                    }
                }
            }

            item(key = "data_storage", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_data_storage_title),
                    highlighted = pulsingSectionKey == "data_storage",
                ) {
                    // 1. Notification history
                    Column(Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.data_notifications_title), style = MaterialTheme.typography.titleSmall)
                        Text(
                            stringResource(R.string.data_notifications_subtitle),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    SettingRowDivider()

                    // 2. Clipboard history
                    val clipboardHistoryTitle = stringResource(R.string.data_clipboard_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(clipboardHistoryTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_clipboard_subtitle),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            if (state.clipboardHistory.isNotEmpty()) {
                                Text(
                                    pluralStringResource(
                                        R.plurals.data_clipboard_count,
                                        state.clipboardHistory.size,
                                        state.clipboardHistory.size,
                                    ),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.primary,
                                    modifier = Modifier.padding(top = 2.dp),
                                )
                            }
                        }
                        Switch(
                            checked = state.clipboardHistoryEnabled,
                            onCheckedChange = onSetClipboardHistoryEnabled,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = clipboardHistoryTitle },
                        )
                    }
                    if (state.clipboardHistory.isNotEmpty()) {
                        OutlinedButton(onClick = onClearClipboardHistory) {
                            Text(stringResource(R.string.data_action_clear))
                        }
                    }
                    SettingRowDivider()

                    // 3. Timeline
                    Text(stringResource(R.string.data_timeline_title), style = MaterialTheme.typography.titleSmall)
                    Text(
                        stringResource(R.string.data_timeline_subtitle),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Choice(
                        listOf(
                            "30" to stringResource(R.string.timeline_retention_30d),
                            "90" to stringResource(R.string.timeline_retention_90d),
                            "365" to stringResource(R.string.timeline_retention_1y),
                            "0" to stringResource(R.string.timeline_retention_all),
                        ),
                        state.timelineRetentionDays.toString(),
                    ) { value -> value.toUIntOrNull()?.let(onSetTimelineRetentionDays) }
                    if (state.timeline.isNotEmpty() || state.timelineTotal > 0u) {
                        OutlinedButton(onClick = onClearTimeline) {
                            Text(stringResource(R.string.data_action_clear))
                        }
                    }
                    SettingRowDivider()

                    // 4. Message cache
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_messages_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_messages_subtitle),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.cachedMmsFiles > 0) {
                            OutlinedButton(onClick = onClearMessageCache) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 5. Photo thumbnails & camera cache
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_photos_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_photos_subtitle),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.cachedPhotoFiles > 0) {
                            OutlinedButton(onClick = onClearPhotoCache) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 6. Received-file history
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_received_files_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_received_files_subtitle),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.receivedFileRecords > 0 || state.transfers.isNotEmpty()) {
                            OutlinedButton(onClick = onClearReceivedFileHistory) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 7. Clear everything
                    Text(
                        stringResource(R.string.data_clear_everything_hint),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    OutlinedButton(onClick = { confirmClearEverything = true }) {
                        Text(stringResource(R.string.data_clear_everything_button))
                    }
                }
            }
        }

        if (showFeatures) {
            item(key = "about", contentType = "section") {
                Section(stringResource(R.string.settings_about)) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.guide_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_guide_card_body),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        FilledTonalButton(onClick = onOpenFeaturesGuide) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.guide_open_walkthrough), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_walkthrough_body),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        OutlinedButton(onClick = onOpenWalkthrough) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Text(stringResource(R.string.about_version, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodyMedium)
                    CheckForUpdates(updater)
                }
            }
            item(key = "support", contentType = "section") {
                val uriHandler = LocalUriHandler.current
                Section(stringResource(R.string.support_title)) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Text(
                            stringResource(R.string.support_body),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f).padding(end = 12.dp),
                        )
                        FilledTonalButton(onClick = { runCatching { uriHandler.openUri(StarNotification.SUPPORT) } }) {
                            Text(stringResource(R.string.support_action))
                        }
                    }
                }
            }
        }
    }

    confirmUnpair?.let { device ->
        AlertDialog(
            onDismissRequest = { confirmUnpair = null },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.unpair_title, device.name)) },
            text = { Text(stringResource(R.string.unpair_text)) },
            confirmButton = {
                TextButton(onClick = { onUnpair(device.id); confirmUnpair = null }) {
                    Text(stringResource(R.string.action_unpair))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmUnpair = null }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (pickLanguage && AppLanguage.supported) {
        val context = LocalContext.current
        val language = remember { AppLanguage.current(context) }
        AlertDialog(
            onDismissRequest = { pickLanguage = false },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.settings_language)) },
            text = {
                Column {
                    AppLanguage.languages.forEach { (tag, name) ->
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .selectable(selected = tag == language, role = Role.RadioButton) {
                                    pickLanguage = false
                                    AppLanguage.set(context, tag)
                                }
                                .padding(vertical = 12.dp),
                        ) {
                            RadioButton(selected = tag == language, onClick = null)
                            Spacer(Modifier.width(12.dp))
                            Text(name, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { pickLanguage = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (confirmClearEverything) {
        AlertDialog(
            onDismissRequest = { confirmClearEverything = false },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.data_clear_everything_dialog_title)) },
            text = { Text(stringResource(R.string.data_clear_everything_dialog_body)) },
            confirmButton = {
                TextButton(onClick = {
                    onClearEverything()
                    confirmClearEverything = false
                }) {
                    Text(stringResource(R.string.data_clear_everything_confirm))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmClearEverything = false }) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }
}

private tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is android.content.ContextWrapper -> baseContext.findActivity()
    else -> null
}

private fun openAppInfoSettings(context: Context) {
    runCatching {
        context.startActivity(
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

/** Shizuku's app, where it's started and where NectarLink is allowed. */
private fun openShizuku(context: Context) {
    context.packageManager.getLaunchIntentForPackage(ShizukuElevated.PACKAGE)
        ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        ?.let { runCatching { context.startActivity(it) } }
}

private fun openDeveloperOptionsSettings(context: Context) {
    val devIntent = Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(devIntent) }.isFailure) {
        runCatching {
            context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}

/**
 * Notification access, which mirroring needs. Android shows the switch in
 * its own settings; sideloaded apps on Android 13+ must first be allowed
 * "restricted settings" there.
 */
@Composable
private fun NotificationAccess(granted: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.notifications_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.notifications_on else R.string.notifications_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (!granted && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = { openAppInfoSettings(context) }) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

/** Unrestricted background execution and OEM-specific battery settings. */
@Composable
private fun BackgroundBatteryAccess(unrestricted: Boolean) {
    val context = LocalContext.current
    val oemGuide = BackgroundAccess.oemGuideResId()?.let { stringResource(it) }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.background_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(if (unrestricted) R.string.background_on else R.string.background_text),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (!unrestricted) {
                Button(onClick = {
                    runCatching { context.startActivity(BackgroundAccess.requestIntent(context)) }
                }) {
                    Text(stringResource(R.string.action_allow))
                }
            } else {
                OutlinedButton(onClick = {
                    runCatching { context.startActivity(BackgroundAccess.oemSettingsIntent(context)) }
                }) {
                    Text(stringResource(R.string.battery_open_oem_settings))
                }
            }
        }
        if (oemGuide != null) {
            Text(
                oemGuide,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (!unrestricted) {
            OutlinedButton(onClick = {
                runCatching { context.startActivity(BackgroundAccess.oemSettingsIntent(context)) }
            }) {
                Text(stringResource(R.string.battery_open_oem_settings))
            }
        }
    }
}

/** Companion Device Manager association for waking when a PC BLE beacon is nearby. */
@Composable
private fun CompanionWakeAccess(
    associated: Boolean,
    devices: List<Device>,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val chooserLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartIntentSenderForResult(),
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            PcCompanion.startObservingPresence(context)
        }
        onAccessChanged()
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.companion_wake_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (associated) R.string.companion_wake_on else R.string.companion_wake_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val primaryPc = devices.firstOrNull { it.online } ?: devices.firstOrNull()
        val triggerAssociate = {
            val activity = context.findActivity()
            if (activity != null) {
                PcCompanion.requestAssociation(
                    activity = activity,
                    pcName = primaryPc?.name ?: "PC",
                    pcDeviceId = primaryPc?.id,
                    onResult = { onAccessChanged() },
                    onLauncherFallback = { sender ->
                        runCatching {
                            chooserLauncher.launch(IntentSenderRequest.Builder(sender).build())
                        }
                    },
                )
            }
        }
        if (associated) {
            OutlinedButton(onClick = triggerAssociate) {
                Text(stringResource(R.string.action_manage))
            }
        } else {
            Button(onClick = triggerAssociate) {
                Text(stringResource(R.string.companion_wake_associate))
            }
        }
    }
}

/** Do Not Disturb and silent ringer control from a PC. */
@Composable
private fun DndAccess(granted: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.dnd_access_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.dnd_access_on else R.string.dnd_access_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneToggles.dndSettingsIntent()) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Screen brightness control from a PC (Modify system settings, or Elevated). */
@Composable
private fun WriteSettingsAccess(granted: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.write_settings_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.write_settings_on else R.string.write_settings_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneToggles.writeSettingsIntent(context)) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Control from the PC: NectarLink's accessibility service, turned on in Android's settings. */
@Composable
private fun ControlAccess(on: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.control_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (on) R.string.control_on else R.string.control_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(InputService.settingsIntent()) }.let {} }
        if (on) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (!on && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f).padding(end = 12.dp),
            )
            OutlinedButton(onClick = { openAppInfoSettings(context) }) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

/** Elevated through the phone's own wireless debugging: real touch and keys. */
@Composable
private fun ElevatedAccess() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by Elevated.state.collectAsStateWithLifecycle()
    var steps by remember { mutableStateOf(false) }
    val shizukuInstalled = remember { ShizukuElevated.installed(context) }
    val failure = (state as? Elevated.State.Failed)?.reason
    val shizukuFailure = failure == Elevated.Reason.ShizukuNotRunning ||
        failure == Elevated.Reason.ShizukuDenied || failure == Elevated.Reason.ShizukuRoot
    // Through Shizuku, the way back is in Shizuku's app, not Developer options.
    val viaShizuku = Elevated.usesShizuku() || shizukuFailure
    val methods = remember { ElevatedMethod.available(shizukuInstalled, Elevated.isRootAvailable()) }
    var method by remember {
        mutableStateOf(if (Elevated.usesShizuku()) ElevatedMethod.Shizuku else ElevatedMethod.WirelessDebugging)
    }
    fun setUp() {
        when (method) {
            ElevatedMethod.WirelessDebugging -> {
                Elevated.useWirelessDebugging()
                steps = true
            }
            ElevatedMethod.Shizuku -> scope.launch {
                if (!Elevated.setUpWithShizuku() && !ShizukuElevated.running()) openShizuku(context)
            }
            ElevatedMethod.Root -> scope.launch { Elevated.setUpWithRoot() }
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.elevated_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(
                        when (state) {
                            Elevated.State.NotSetUp -> R.string.elevated_not_set_up
                            Elevated.State.Waiting ->
                                if (Elevated.usesShizuku()) R.string.elevated_shizuku_waiting else R.string.elevated_waiting
                            Elevated.State.Starting -> R.string.elevated_starting
                            Elevated.State.Running -> when {
                                Elevated.rootActive -> R.string.elevated_running_root
                                Elevated.shizukuActive -> R.string.elevated_running_shizuku
                                else -> R.string.elevated_running
                            }
                            is Elevated.State.Failed -> when ((state as Elevated.State.Failed).reason) {
                                Elevated.Reason.NotPaired -> R.string.elevated_not_paired
                                Elevated.Reason.RootDenied -> R.string.elevated_root_failed
                                Elevated.Reason.ShizukuNotRunning -> R.string.elevated_shizuku_not_running
                                Elevated.Reason.ShizukuDenied -> R.string.elevated_shizuku_denied
                                Elevated.Reason.ShizukuRoot -> R.string.elevated_shizuku_root
                                else -> R.string.elevated_failed
                            }
                        },
                    ),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            when (state) {
                Elevated.State.NotSetUp, is Elevated.State.Failed -> Button(onClick = { setUp() }) {
                    Text(stringResource(R.string.elevated_set_up))
                }
                Elevated.State.Waiting -> Button(onClick = {
                    // Picked another way while waiting for this one.
                    val waitingFor =
                        if (Elevated.usesShizuku()) ElevatedMethod.Shizuku else ElevatedMethod.WirelessDebugging
                    if (method != waitingFor) {
                        setUp()
                        return@Button
                    }
                    scope.launch {
                        if (Elevated.usesShizuku()) {
                            if (!Elevated.start() && !ShizukuElevated.running()) openShizuku(context)
                            return@launch
                        }
                        PairingNotification.show(context)
                        if (!Elevated.start()) {
                            openDeveloperOptionsSettings(context)
                        }
                    }
                }) {
                    Text(stringResource(R.string.elevated_try_again))
                }
                Elevated.State.Running -> OutlinedButton(onClick = { scope.launch { Elevated.forget() } }) {
                    Text(stringResource(R.string.elevated_turn_off))
                }
                Elevated.State.Starting -> {}
            }
        }
        if (state == Elevated.State.Waiting || state is Elevated.State.Failed) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (viaShizuku && shizukuInstalled) {
                    OutlinedButton(onClick = { openShizuku(context) }) {
                        Text(stringResource(R.string.elevated_open_shizuku))
                    }
                } else {
                    OutlinedButton(onClick = {
                        PairingNotification.show(context)
                        openDeveloperOptionsSettings(context)
                    }) {
                        Text(stringResource(R.string.elevated_open_developer))
                    }
                }
            }
        }
        // Shizuku and root only show when the phone has them, and nothing asks for
        // either until the user picks it and taps Set up. To change the way it
        // started, turn it off first.
        if (methods.size > 1 && state != Elevated.State.Running && state != Elevated.State.Starting) {
            Text(stringResource(R.string.elevated_start_with), style = MaterialTheme.typography.labelMedium)
            Choice(
                options = methods.map { option ->
                    option.name to stringResource(
                        when (option) {
                            ElevatedMethod.WirelessDebugging -> R.string.elevated_method_wireless
                            ElevatedMethod.Shizuku -> R.string.elevated_method_shizuku
                            ElevatedMethod.Root -> R.string.elevated_method_root
                        },
                    )
                },
                selected = method.name,
                onSelect = { picked -> method = ElevatedMethod.valueOf(picked) },
            )
            val hint = when (method) {
                ElevatedMethod.WirelessDebugging -> null
                ElevatedMethod.Shizuku -> R.string.elevated_shizuku_hint
                ElevatedMethod.Root -> R.string.elevated_root_hint
            }
            hint?.let {
                Text(
                    stringResource(it),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
    if (steps) {
        AlertDialog(
            onDismissRequest = { steps = false },
            title = { Text(stringResource(R.string.elevated_steps_title)) },
            text = { Text(stringResource(R.string.elevated_steps)) },
            confirmButton = {
                TextButton(onClick = {
                    steps = false
                    PairingNotification.show(context)
                    openDeveloperOptionsSettings(context)
                }) { Text(stringResource(R.string.elevated_open_developer)) }
            },
            dismissButton = {
                TextButton(onClick = { steps = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@Composable
private fun RuntimePermissionAccess(
    title: String,
    textOn: String,
    textOff: String,
    granted: Boolean,
    permissions: Array<String>,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        onAccessChanged()
        if (grants.values.none { it }) {
            openAppInfoSettings(context)
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(
                if (granted) textOn else textOff,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (granted) {
            OutlinedButton(onClick = { openAppInfoSettings(context) }) {
                Text(stringResource(R.string.action_manage))
            }
        } else {
            Button(onClick = { ask.launch(permissions) }) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

@Composable
private fun PhotosAccess(
    granted: Boolean,
    partial: Boolean,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        onAccessChanged()
        if (grants.values.none { it }) {
            openAppInfoSettings(context)
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.photos_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(
                    when {
                        partial -> R.string.photos_partial
                        granted -> R.string.photos_on
                        else -> R.string.photos_off
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when {
            partial -> OutlinedButton(onClick = { ask.launch(RecentPhotos.permissions) }) {
                Text(stringResource(R.string.action_select_photos))
            }
            granted -> OutlinedButton(onClick = { openAppInfoSettings(context) }) {
                Text(stringResource(R.string.action_manage))
            }
            else -> Button(onClick = { ask.launch(RecentPhotos.permissions) }) {
                Text(stringResource(R.string.action_allow))
            }
        }
    }
}

@Composable
private fun StorageAccess(
    allFiles: Boolean,
    safFolders: List<String>,
    onAddSafFolder: (Uri) -> Unit,
    onRemoveSafFolder: (String) -> Unit,
) {
    val context = LocalContext.current
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) onAddSafFolder(uri)
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.storage_all_files_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (allFiles) R.string.storage_all_files_on else R.string.storage_all_files_off),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }.let {} }
        if (allFiles) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (!allFiles) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.storage_saf_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(R.string.storage_saf_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            OutlinedButton(onClick = { pickFolder.launch(null) }) {
                Text(stringResource(R.string.storage_add_folder))
            }
        }
        safFolders.forEach { name ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(name, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                TextButton(onClick = { onRemoveSafFolder(name) }) {
                    Text(stringResource(R.string.storage_remove_folder))
                }
            }
        }
    }
}

@Composable
private fun SettingRowDivider() {
    val isGlass = LocalGlassTheme.current
    HorizontalDivider(
        color = if (isGlass) {
            MaterialTheme.colorScheme.outlineVariant
        } else {
            MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.45f)
        },
    )
}

@Composable
private fun Section(
    title: String,
    highlighted: Boolean = false,
    content: @Composable () -> Unit,
) {
    val isGlass = LocalGlassTheme.current
    val scheme = MaterialTheme.colorScheme
    Column(
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.labelLarge,
            color = if (highlighted) scheme.primary else scheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp),
        )
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .liquidGlassCard(
                    cornerRadius = if (isGlass) 24.dp else 16.dp,
                    isGlass = isGlass,
                    baseColor = if (highlighted) scheme.secondaryContainer else scheme.surfaceContainer,
                    tintColor = scheme.primary,
                    outlineColor = when {
                        highlighted -> scheme.primary
                        isGlass -> scheme.outlineVariant
                        else -> androidx.compose.ui.graphics.Color.Transparent
                    },
                )
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            content()
        }
    }
}

@Composable
private fun Label(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall)
}

/** The third language choice when it isn't a picked language: opens the list. */
private const val MORE_LANGUAGES = "more"

/** A row of choices, one picked. */
@Composable
internal fun Choice(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { index, (value, label) ->
            SegmentedButton(
                selected = value == selected,
                onClick = { onSelect(value) },
                shape = SegmentedButtonDefaults.itemShape(index, options.size),
            ) { Text(label) }
        }
    }
}

