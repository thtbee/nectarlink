// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.ui.theme.glassModalContainerColor
import app.nectarlink.android.ui.theme.glassModalScrimColor
import app.nectarlink.core.whatsNewJson
import app.nectarlink.core.whatsNewMarkSeen
import app.nectarlink.core.whatsNewNoteStart
import app.nectarlink.core.whatsNewPending
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * What's new (#461), as on the PC: the changes since the last look, one short
 * line each, read from the changelog this build was made from (the core does
 * that, and keeps what was seen). It never opens by itself; a dot on Home's
 * sparkle button says an update brought something new, and opening it clears
 * the dot.
 */
internal object WhatsNew {
    private const val PREFS = "whats_new"
    private const val VERSION_CODE = "version_code"

    private val _pending = MutableStateFlow(false)
    val pending: StateFlow<Boolean> = _pending.asStateFlow()

    data class Group(val title: String, val items: List<String>, val more: Int)
    data class Summary(val version: String, val highlights: Boolean, val groups: List<Group>)

    private fun dir(context: Context): String =
        context.noBackupFilesDir.resolve("whats-new").apply { mkdirs() }.absolutePath

    /**
     * When the app opens: after an update, the dot shows if it brought
     * something not seen; on a fresh install, everything counts as seen.
     */
    suspend fun onAppOpened(context: Context) = withContext(Dispatchers.IO) {
        val app = context.applicationContext
        runCatching {
            val prefs = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val before = prefs.getLong(VERSION_CODE, 0L)
            val now = BuildConfig.VERSION_CODE.toLong()
            val updated = if (before == 0L) {
                // The first build with What's new: an update if the app was
                // here before it, by the install times.
                val info = app.packageManager.getPackageInfo(app.packageName, 0)
                info.lastUpdateTime > info.firstInstallTime
            } else {
                before != now
            }
            if (before != now) {
                whatsNewNoteStart(dir(app), updated)
                prefs.edit().putLong(VERSION_CODE, now).apply()
            }
            _pending.value = whatsNewPending(dir(app))
        }
    }

    /** Opened: what to show, and the dot goes. */
    suspend fun open(context: Context): Summary? = withContext(Dispatchers.IO) {
        runCatching {
            val dir = dir(context.applicationContext)
            val summary = parse(whatsNewJson(dir))
            whatsNewMarkSeen(dir)
            _pending.value = false
            summary
        }.getOrNull()
    }

    private fun parse(json: String): Summary {
        val root = JSONObject(json)
        val groups = root.optJSONArray("groups")
        return Summary(
            version = root.optString("version"),
            highlights = root.optString("mode") == "highlights",
            groups = List(groups?.length() ?: 0) { i ->
                val g = groups!!.getJSONObject(i)
                val items = g.optJSONArray("items")
                Group(
                    title = g.optString("title"),
                    items = List(items?.length() ?: 0) { items!!.getString(it) },
                    more = g.optInt("more"),
                )
            },
        )
    }
}

/** Home's What's new button: a sparkle, with a dot while something is new. */
@Composable
internal fun WhatsNewButton() {
    val context = LocalContext.current
    val pending by WhatsNew.pending.collectAsState()
    val scope = rememberCoroutineScope()
    var summary by remember { mutableStateOf<WhatsNew.Summary?>(null) }
    var open by remember { mutableStateOf(false) }
    val label = stringResource(R.string.update_whats_new)
    val dotLabel = stringResource(R.string.whats_new_dot)
    IconButton(
        onClick = {
            scope.launch {
                summary = WhatsNew.open(context)
                open = true
            }
        },
        modifier = Modifier.semantics { contentDescription = if (pending) "$label, $dotLabel" else label },
    ) {
        Box {
            Icon(painterResource(R.drawable.ic_sparkles), contentDescription = null)
            if (pending) {
                Box(
                    Modifier
                        .align(Alignment.TopEnd)
                        .size(7.dp)
                        .background(MaterialTheme.colorScheme.primary, CircleShape),
                )
            }
        }
    }
    if (open) {
        WhatsNewSheet(summary, onDismiss = { open = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WhatsNewSheet(summary: WhatsNew.Summary?, onDismiss: () -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = glassModalContainerColor(),
        scrimColor = glassModalScrimColor(),
    ) {
        LazyColumn(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            item(key = "title") {
                Row(verticalAlignment = Alignment.Bottom, modifier = Modifier.padding(bottom = 4.dp)) {
                    Text(stringResource(R.string.update_whats_new), style = MaterialTheme.typography.titleLarge)
                    if (!summary?.version.isNullOrEmpty()) {
                        Text(
                            summary!!.version,
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 8.dp, bottom = 3.dp),
                        )
                    }
                }
            }
            val groups = summary?.groups.orEmpty()
            if (groups.isEmpty()) {
                item(key = "empty") {
                    Text(
                        stringResource(R.string.whats_new_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            groups.forEachIndexed { index, group ->
                item(key = "head$index") {
                    Text(
                        if (summary!!.highlights) stringResource(R.string.whats_new_in_this_version) else groupTitle(group.title),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(top = if (index == 0) 4.dp else 12.dp),
                    )
                }
                items(group.items, key = { "item$index:$it" }) { line ->
                    Row(verticalAlignment = Alignment.Top) {
                        Box(
                            Modifier
                                .padding(top = 8.dp, end = 10.dp)
                                .size(4.dp)
                                .background(MaterialTheme.colorScheme.onSurfaceVariant, CircleShape),
                        )
                        Text(
                            line,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                if (group.more > 0) {
                    item(key = "more$index") {
                        Text(
                            stringResource(R.string.whats_new_more, group.more),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 14.dp),
                        )
                    }
                }
            }
            item(key = "link") {
                TextButton(
                    onClick = {
                        Link.open(context, Link.RELEASES)
                        scope.launch { sheetState.hide() }.invokeOnCompletion { onDismiss() }
                    },
                    modifier = Modifier.padding(top = 8.dp),
                ) {
                    Icon(
                        painterResource(R.drawable.ic_open_external),
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.size(8.dp))
                    Text(stringResource(R.string.whats_new_release_notes))
                }
                Spacer(Modifier.height(16.dp))
            }
        }
    }
}

@Composable
private fun groupTitle(title: String): String = when (title) {
    "New" -> stringResource(R.string.whats_new_group_new)
    "Changed" -> stringResource(R.string.whats_new_group_changed)
    "Fixed" -> stringResource(R.string.whats_new_group_fixed)
    else -> title
}
