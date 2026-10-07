// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
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
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.PairingNotification
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.CheckForUpdates
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.Device
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.LocalAppFonts
import app.nectarlink.android.ui.theme.Tokens

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
    onAddSafFolder: (Uri) -> Unit = {},
    onRemoveSafFolder: (String) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var confirmUnpair by remember { mutableStateOf<Device?>(null) }

    Column(
        modifier.verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            stringResource(R.string.settings_title),
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
        )

        Section(stringResource(R.string.settings_notifications)) {
            NotificationAccess(state.notificationAccess)
        }

        Section(stringResource(R.string.settings_calls_and_contacts)) {
            RuntimePermissionAccess(
                title = stringResource(R.string.calls_title),
                textOn = stringResource(R.string.calls_on),
                textOff = stringResource(R.string.calls_off),
                granted = state.callAccess,
                permissions = PhoneCalls.permissions,
                onAccessChanged = onAccessChanged,
            )
            RuntimePermissionAccess(
                title = stringResource(R.string.contacts_title),
                textOn = stringResource(R.string.contacts_on),
                textOff = stringResource(R.string.contacts_off),
                granted = state.contactsAccess,
                permissions = PhoneContacts.permissions,
                onAccessChanged = onAccessChanged,
            )
            RuntimePermissionAccess(
                title = stringResource(R.string.sms_title),
                textOn = stringResource(R.string.sms_on),
                textOff = stringResource(R.string.sms_off),
                granted = state.smsAccess,
                permissions = PhoneSms.permissions,
                onAccessChanged = onAccessChanged,
            )
        }

        Section(stringResource(R.string.settings_photos)) {
            PhotosAccess(
                granted = state.photoAccess,
                partial = state.photoPartialAccess,
                onAccessChanged = onAccessChanged,
            )
        }

        Section(stringResource(R.string.settings_storage)) {
            StorageAccess(
                allFiles = state.storageAllFilesAccess,
                safFolders = state.storageSafFolders,
                onAddSafFolder = onAddSafFolder,
                onRemoveSafFolder = onRemoveSafFolder,
            )
        }

        Section(stringResource(R.string.settings_control)) {
            DndAccess(state.dndAccess)
            WriteSettingsAccess(state.writeSettingsAccess || state.elevated)
            ControlAccess(state.inputAccess)
            ElevatedAccess()
        }

        Section(stringResource(R.string.settings_remote)) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(
                    stringResource(R.string.settings_touchpad_sensitivity),
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
            )
        }

        Section(stringResource(R.string.settings_appearance)) {
            Label(stringResource(R.string.settings_theme))
            Choice(
                listOf("bloom" to stringResource(R.string.theme_bloom), "graphite" to stringResource(R.string.theme_graphite)),
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
            if (appearance.theme == "bloom") {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f)) {
                            Text(stringResource(R.string.settings_dynamic_color), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_dynamic_color_hint),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = appearance.dynamicColor,
                            onCheckedChange = { on -> onAppearance { it.copy(dynamicColor = on) } },
                        )
                    }
                }
                if (!appearance.dynamicColor || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Tokens.bloomSeeds.forEach { (name, seed) ->
                            val selected = appearance.seed == name
                            Surface(
                                shape = CircleShape,
                                color = seed.color,
                                modifier = Modifier
                                    .size(32.dp)
                                    .then(if (selected) Modifier.border(2.dp, MaterialTheme.colorScheme.onSurface, CircleShape) else Modifier)
                                    .clickable { onAppearance { it.copy(seed = name) } }
                                    .semantics { contentDescription = name },
                            ) {}
                        }
                    }
                }
            }
        }

        Section(stringResource(R.string.settings_devices)) {
            val context = LocalContext.current
            state.devices.forEach { device ->
                Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f)) {
                            Text(device.name, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(if (device.online) R.string.connected else R.string.not_connected),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        OutlinedButton(onClick = { confirmUnpair = device }) { Text(stringResource(R.string.action_unpair)) }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.storage_pc_toggle), style = MaterialTheme.typography.bodyMedium)
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
                                    context.startActivity(PhoneStorage.allFilesAccessIntent(context))
                                }
                            },
                        )
                    }
                }
            }
            FilledTonalButton(onClick = onPairNew) { Text(stringResource(R.string.action_pair_new)) }
        }

        Section(stringResource(R.string.settings_about)) {
            Text(stringResource(R.string.about_version, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodyMedium)
            CheckForUpdates(updater)
            (state.status as? CoreStatus.Ready)?.let {
                Text(
                    it.deviceId,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = LocalAppFonts.current.mono,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }

    confirmUnpair?.let { device ->
        AlertDialog(
            onDismissRequest = { confirmUnpair = null },
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
        val open = { context.startActivity(NotificationListener.settingsIntent(context)) }
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
            TextButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text(stringResource(R.string.action_app_info)) }
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
        val open = { context.startActivity(PhoneToggles.dndSettingsIntent()) }
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
        val open = { context.startActivity(PhoneToggles.writeSettingsIntent(context)) }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Control from the PC: Nectarlink's accessibility service, turned on in Android's settings. */
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
        val open = { context.startActivity(InputService.settingsIntent()) }
        if (on) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Elevated through the phone's own wireless debugging: real touch and keys. */
@Composable
private fun ElevatedAccess() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by Elevated.state.collectAsState()
    var steps by remember { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.elevated_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(
                    when (state) {
                        Elevated.State.NotSetUp -> R.string.elevated_not_set_up
                        Elevated.State.Waiting -> R.string.elevated_waiting
                        Elevated.State.Starting -> R.string.elevated_starting
                        Elevated.State.Running -> R.string.elevated_running
                        is Elevated.State.Failed ->
                            if ((state as Elevated.State.Failed).reason == Elevated.Reason.NotPaired) R.string.elevated_not_paired
                            else R.string.elevated_failed
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when (state) {
            Elevated.State.NotSetUp, is Elevated.State.Failed -> Button(onClick = { steps = true }) {
                Text(stringResource(R.string.elevated_set_up))
            }
            Elevated.State.Waiting -> OutlinedButton(onClick = { scope.launch { Elevated.start() } }) {
                Text(stringResource(R.string.elevated_try_again))
            }
            Elevated.State.Running -> OutlinedButton(onClick = { scope.launch { Elevated.forget() } }) {
                Text(stringResource(R.string.elevated_turn_off))
            }
            Elevated.State.Starting -> {}
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
                    context.startActivity(
                        Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
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
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        onAccessChanged()
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
            OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text(stringResource(R.string.action_manage)) }
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
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        onAccessChanged()
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
            granted -> OutlinedButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }) { Text(stringResource(R.string.action_manage)) }
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
        val open = { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }
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
private fun Section(title: String, content: @Composable () -> Unit) {
    Text(
        title,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 8.dp),
    )
    Surface(shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) { content() }
    }
}

@Composable
private fun Label(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall)
}

@Composable
private fun Choice(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
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
