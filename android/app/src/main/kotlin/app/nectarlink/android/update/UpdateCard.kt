// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import kotlinx.coroutines.launch

/** "Nectarlink 0.2.0 is available", while there's an update to talk about. */
@Composable
fun UpdateCard(updater: AppUpdater, modifier: Modifier = Modifier) {
    val state by updater.state.collectAsStateWithLifecycle()
    val update = when (val s = state) {
        is AppUpdater.State.Available -> s.update
        is AppUpdater.State.Downloading -> s.update
        is AppUpdater.State.Installing -> s.update
        is AppUpdater.State.Failed -> s.update
        else -> null
    } ?: return
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val failure = (state as? AppUpdater.State.Failed)?.reason
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = modifier,
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(stringResource(R.string.update_title, update.version), style = MaterialTheme.typography.titleMedium)
            Text(
                stringResource(
                    when {
                        state is AppUpdater.State.Downloading -> R.string.update_downloading
                        state is AppUpdater.State.Installing -> R.string.update_installing
                        failure == AppUpdater.Reason.NotAllowed -> R.string.update_not_allowed
                        failure == AppUpdater.Reason.Damaged -> R.string.update_damaged
                        failure == AppUpdater.Reason.Install -> R.string.update_install_failed
                        failure != null -> R.string.update_offline
                        else -> R.string.update_text
                    },
                ),
                style = MaterialTheme.typography.bodyMedium,
            )
            if (state is AppUpdater.State.Downloading || state is AppUpdater.State.Installing) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (failure == AppUpdater.Reason.NotAllowed && !updater.canInstall()) {
                    Button(onClick = { context.startActivity(updater.allowInstallsIntent()) }) {
                        Text(stringResource(R.string.action_allow))
                    }
                } else {
                    Button(
                        enabled = state is AppUpdater.State.Available || state is AppUpdater.State.Failed,
                        onClick = { scope.launch { updater.install(update) } },
                    ) {
                        Text(stringResource(R.string.action_update))
                    }
                }
                TextButton(onClick = {
                    context.startActivity(Intent(Intent.ACTION_VIEW, update.notesUrl.toUri()))
                }) {
                    Text(stringResource(R.string.update_whats_new))
                }
            }
        }
    }
}

/** "Check for updates" in Settings, with what it found. */
@Composable
fun CheckForUpdates(updater: AppUpdater) {
    if (!updater.enabled) return
    val scope = rememberCoroutineScope()
    val state by updater.state.collectAsStateWithLifecycle()
    var checked by remember { mutableStateOf(false) }
    OutlinedButton(
        enabled = state !is AppUpdater.State.Checking && state !is AppUpdater.State.Downloading,
        onClick = {
            scope.launch {
                updater.check()
                checked = true
            }
        },
    ) {
        Text(stringResource(R.string.update_check))
    }
    if (checked) {
        val text = when (state) {
            AppUpdater.State.Idle -> R.string.update_up_to_date
            is AppUpdater.State.Failed -> R.string.update_check_failed
            else -> null
        }
        text?.let { Text(stringResource(it), style = MaterialTheme.typography.bodySmall) }
    }
}
