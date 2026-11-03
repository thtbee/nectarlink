// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.banking.OpenWithPauseActivity
import app.nectarlink.android.elevated.Elevated
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** An app the user can choose: package, name and a small icon. */
private data class Choosable(val pkg: String, val label: String, val icon: ImageBitmap?)

/** Settings › Control › Pause for banking apps. */
@Composable
internal fun BankingPauseAccess() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var chosen by remember { mutableStateOf(BankingPause.apps(context)) }
    var canChange by remember { mutableStateOf(BankingPause.canChangeSettings(context)) }
    var usage by remember { mutableStateOf(BankingPause.hasUsageAccess(context)) }
    var picking by remember { mutableStateOf(false) }
    val paused by BankingPause.paused.collectAsStateWithLifecycle()

    fun refreshUsageAccess() {
        usage = BankingPause.hasUsageAccess(context)
        if (!usage) {
            scope.launch {
                repeat(10) {
                    delay(100)
                    if (BankingPause.hasUsageAccess(context)) {
                        usage = true
                        return@launch
                    }
                }
            }
        }
    }
    // Permissions can change in Settings while NectarLink is in the background.
    LifecycleResumeEffect(Unit) {
        canChange = BankingPause.canChangeSettings(context)
        usage = BankingPause.hasUsageAccess(context)
        onPauseOrDispose {}
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.banking_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    if (chosen.isEmpty()) {
                        stringResource(R.string.banking_text)
                    } else {
                        pluralStringResource(R.plurals.banking_chosen, chosen.size, chosen.size)
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            OutlinedButton(onClick = { picking = true }) { Text(stringResource(R.string.banking_choose)) }
        }

        if (chosen.isNotEmpty()) {
            when {
                !canChange -> Note(stringResource(R.string.banking_needs_elevated))
                !usage -> Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Note(stringResource(R.string.banking_needs_usage), Modifier.weight(1f).padding(end = 12.dp))
                    Button(onClick = {
                        // Through Elevated when it's running, else Android's own page.
                        if (!Elevated.grantUsageAccess()) openUsageAccessSettings(context)
                        refreshUsageAccess()
                    }) { Text(stringResource(R.string.action_allow)) }
                }
                paused -> Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Note(stringResource(R.string.banking_paused_now), Modifier.weight(1f).padding(end = 12.dp))
                    TextButton(onClick = { BankingPause.resume(context) }) {
                        Text(stringResource(R.string.banking_turn_back_on))
                    }
                }
            }
            val canPin = remember { OpenWithPauseActivity.canPin(context) }
            for (app in remember(chosen) { describe(context, chosen) }) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    AppIcon(app.icon)
                    Text(
                        app.label,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
                    )
                    if (canPin) {
                        TextButton(onClick = { OpenWithPauseActivity.pin(context, app.pkg, app.label) }) {
                            Text(stringResource(R.string.banking_shortcut))
                        }
                    }
                }
            }
            if (canPin) Note(stringResource(R.string.banking_shortcut_hint))
        }
    }

    if (picking) {
        AppPicker(
            initial = chosen,
            onDone = { picked ->
                picking = false
                BankingPause.setApps(context, picked)
                chosen = picked
                // Elevated can let NectarLink see the app in front right away.
                if (picked.isNotEmpty() && !BankingPause.hasUsageAccess(context)) {
                    Elevated.grantUsageAccess()
                    refreshUsageAccess()
                }
            },
            onCancel = { picking = false },
        )
    }
}

@Composable
private fun AppPicker(initial: Set<String>, onDone: (Set<String>) -> Unit, onCancel: () -> Unit) {
    val context = LocalContext.current
    // Every app's name and icon: read away from the main thread, which they
    // held for the whole time the dialog took to open.
    val apps by produceState<List<Choosable>?>(initialValue = null) {
        value = withContext(Dispatchers.Default) { launchableApps(context) }
    }
    var selected by remember { mutableStateOf(initial) }
    var query by remember { mutableStateOf("") }
    val shown = apps.orEmpty().filter { query.isBlank() || it.label.contains(query.trim(), ignoreCase = true) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(stringResource(R.string.banking_pick_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    label = { Text(stringResource(R.string.banking_search)) },
                    modifier = Modifier.fillMaxWidth(),
                )
                if (apps == null) {
                    Row(
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.fillMaxWidth().height(120.dp),
                    ) { CircularProgressIndicator() }
                }
                LazyColumn(Modifier.heightIn(max = 360.dp)) {
                    items(shown, key = { it.pkg }) { app ->
                        val on = app.pkg in selected
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { selected = if (on) selected - app.pkg else selected + app.pkg }
                                .padding(vertical = 4.dp),
                        ) {
                            Checkbox(checked = on, onCheckedChange = null)
                            AppIcon(app.icon)
                            Text(app.label, modifier = Modifier.padding(start = 12.dp))
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onDone(selected) }) { Text(stringResource(R.string.action_done)) } },
        dismissButton = { TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) } },
    )
}

@Composable
private fun AppIcon(icon: ImageBitmap?) {
    if (icon != null) Image(icon, contentDescription = null, modifier = Modifier.size(28.dp))
}

@Composable
private fun Note(text: String, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = modifier)
}

/** Apps with a launcher icon, by name, NectarLink left out. */
private fun launchableApps(context: Context): List<Choosable> {
    val manager = context.packageManager
    val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
    @Suppress("DEPRECATION")
    val found = manager.queryIntentActivities(launcher, 0)
    return found.map { it.activityInfo }
        .filter { it.packageName != context.packageName }
        .distinctBy { it.packageName }
        .map { info -> Choosable(info.packageName, info.loadLabel(manager).toString().trim(), iconOf(manager, info.packageName)) }
        .sortedBy { it.label.lowercase() }
}

/** The chosen apps that are still installed, by name. */
private fun describe(context: Context, packages: Set<String>): List<Choosable> {
    val manager = context.packageManager
    return packages.mapNotNull { pkg ->
        runCatching {
            val info = manager.getApplicationInfo(pkg, 0)
            Choosable(pkg, manager.getApplicationLabel(info).toString(), iconOf(manager, pkg))
        }.getOrNull()
    }.sortedBy { it.label.lowercase() }
}

private fun iconOf(manager: PackageManager, pkg: String): ImageBitmap? =
    runCatching { manager.getApplicationIcon(pkg).toBitmap(96, 96).asImageBitmap() }.getOrNull()

private fun openUsageAccessSettings(context: Context) {
    runCatching {
        context.startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
