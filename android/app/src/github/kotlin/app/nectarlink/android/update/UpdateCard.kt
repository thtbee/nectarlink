// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.content.Intent
import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.ui.theme.LocalReducedMotion

/** The update a state is about, if any. */
private val AppUpdater.State.update: AppUpdater.Update?
    get() = when (this) {
        is AppUpdater.State.Available -> update
        is AppUpdater.State.NeedsPermission -> update
        is AppUpdater.State.Downloading -> update
        is AppUpdater.State.Installing -> update
        is AppUpdater.State.Failed -> update
        else -> null
    }

/** "NectarLink 0.2.0 is available" on Home, while there's an update to talk about. */
@Composable
fun UpdateCard(updater: AppUpdater, modifier: Modifier = Modifier) {
    val state by updater.state.collectAsStateWithLifecycle()
    val update = state.update ?: return
    UpdatePanel(updater, state, update, modifier)
}

/** One update's status, progress and actions, for the Home card and Settings. */
@Composable
private fun UpdatePanel(updater: AppUpdater, state: AppUpdater.State, update: AppUpdater.Update, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val failure = (state as? AppUpdater.State.Failed)?.reason
    Surface(
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = modifier,
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(stringResource(R.string.update_title, update.title(context)), style = MaterialTheme.typography.titleMedium)
            val status = when {
                state is AppUpdater.State.Downloading && state.waitingForNetwork -> stringResource(R.string.update_waiting_network)
                state is AppUpdater.State.Downloading && state.total > 0 -> stringResource(
                    R.string.update_downloading_progress,
                    Formatter.formatShortFileSize(context, state.done),
                    Formatter.formatShortFileSize(context, state.total),
                )
                state is AppUpdater.State.Downloading -> stringResource(R.string.update_downloading)
                state is AppUpdater.State.Installing -> stringResource(R.string.update_installing)
                state is AppUpdater.State.NeedsPermission -> stringResource(R.string.update_not_allowed)
                failure == AppUpdater.Reason.Damaged -> stringResource(R.string.update_damaged)
                failure == AppUpdater.Reason.Install -> stringResource(R.string.update_install_failed)
                failure != null -> stringResource(R.string.update_offline)
                else -> stringResource(R.string.update_text)
            }
            Text(status, style = MaterialTheme.typography.bodyMedium)
            when {
                state is AppUpdater.State.Downloading && state.total > 0 && !state.waitingForNetwork -> LinearProgressIndicator(
                    progress = { (state.done.toFloat() / state.total).coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth(),
                )
                state is AppUpdater.State.Downloading || state is AppUpdater.State.Installing ->
                    if (LocalReducedMotion.current) {
                        LinearProgressIndicator(progress = { 0.5f }, modifier = Modifier.fillMaxWidth())
                    } else {
                        LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                when (state) {
                    is AppUpdater.State.NeedsPermission -> Button(onClick = { context.startActivity(updater.allowInstallsIntent()) }) {
                        Text(stringResource(R.string.action_allow))
                    }
                    is AppUpdater.State.Available, is AppUpdater.State.Failed -> Button(onClick = {
                        updater.requestInstall(update)?.let(context::startActivity)
                    }) {
                        Text(stringResource(if (failure != null) R.string.action_try_again else R.string.action_update))
                    }
                    else -> Unit
                }
                TextButton(onClick = {
                    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, update.notesUrl.toUri())) }
                }) {
                    Text(stringResource(R.string.update_whats_new))
                }
            }
        }
    }
}

/** Updates in Settings: automatic installs on or off, the channel, a check, and what it found. */
@Composable
fun CheckForUpdates(updater: AppUpdater) {
    if (!updater.enabled) return
    val state by updater.state.collectAsStateWithLifecycle()
    val auto by updater.autoInstall.collectAsStateWithLifecycle()
    val channel by updater.channel.collectAsStateWithLifecycle()
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.update_auto_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(R.string.update_auto_text),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(
            checked = auto,
            onCheckedChange = updater::setAutoInstall,
            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
        )
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(stringResource(R.string.update_channel_title), style = MaterialTheme.typography.titleSmall)
        app.nectarlink.android.ui.settings.Choice(
            listOf(
                AppUpdater.Channel.Stable.key to stringResource(R.string.update_channel_stable),
                AppUpdater.Channel.Nightly.key to stringResource(R.string.update_channel_nightly),
            ),
            channel.key,
        ) { key -> AppUpdater.Channel.parse(key)?.let(updater::setChannel) }
        Text(
            stringResource(if (channel == AppUpdater.Channel.Nightly) R.string.update_channel_nightly_text else R.string.update_channel_stable_text),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    val update = state.update
    if (update != null) {
        UpdatePanel(updater, state, update, Modifier.fillMaxWidth())
        return
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedButton(enabled = state !is AppUpdater.State.Checking, onClick = updater::checkNow) {
            Text(stringResource(R.string.update_check))
        }
        if (state is AppUpdater.State.Checking) {
            CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
        }
    }
    val result = when (state) {
        AppUpdater.State.Checking -> stringResource(R.string.update_checking)
        AppUpdater.State.UpToDate -> stringResource(R.string.update_up_to_date_version, BuildConfig.VERSION_NAME)
        is AppUpdater.State.Failed -> stringResource(R.string.update_check_failed)
        else -> stringResource(R.string.update_current_version, BuildConfig.VERSION_NAME)
    }
    Text(result, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
