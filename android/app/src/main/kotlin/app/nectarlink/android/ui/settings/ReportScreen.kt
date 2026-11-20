// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.diag.Reports
import app.nectarlink.core.ReportOutcome
import app.nectarlink.core.ReportPeer
import app.nectarlink.core.reportGuessArea
import app.nectarlink.core.reportIssueUrl
import app.nectarlink.core.reportLooksLikeContact
import app.nectarlink.core.reportOutgoing
import app.nectarlink.core.reportPrepare
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The report areas, as the core names them, with their labels. */
private val AREAS = listOf(
    "connection" to R.string.report_area_connection,
    "app-windows" to R.string.report_area_app_windows,
    "clipboard" to R.string.report_area_clipboard,
    "audio" to R.string.report_area_audio,
    "calls" to R.string.report_area_calls,
    "files" to R.string.report_area_files,
    "notifications" to R.string.report_area_notifications,
    "other" to R.string.report_area_other,
)

/** The parts a report can leave out: its bit in `report_prepare`, and its label. */
private val PARTS = listOf(
    1 to R.string.report_part_devices,
    2 to R.string.report_part_checks,
    4 to R.string.report_part_health,
    8 to R.string.report_part_crashes,
    16 to R.string.report_part_log,
)

private const val ALL_PARTS = 31
private const val MAX_TEXT = 4000

/** Where Send got to. */
private sealed interface SendState {
    data object Idle : SendState
    data object Sending : SendState
    data class Done(val outcome: ReportOutcome) : SendState
}

/**
 * Report a problem: what went wrong in the user's words, then the exact
 * report to read before it goes anywhere. Send hands it to the report
 * service, which files it as an issue; Open on GitHub fills in a new issue
 * and copies the report to paste into it.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ReportScreen(
    state: CoreState,
    onAsk: suspend (id: String, log: Boolean, justNow: Boolean) -> ReportPeer?,
    reportLogs: Flow<String>,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val uriHandler = LocalUriHandler.current
    var step by rememberSaveable { mutableIntStateOf(0) }
    var area by rememberSaveable { mutableStateOf<String?>(null) }
    var otherArea by rememberSaveable { mutableStateOf("") }
    var text by rememberSaveable { mutableStateOf("") }
    var justNow by rememberSaveable { mutableStateOf(true) }
    var parts by rememberSaveable { mutableIntStateOf(ALL_PARTS) }
    var preview by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf(false) }
    var send by remember { mutableStateOf<SendState>(SendState.Idle) }
    var peers by remember { mutableStateOf<Map<String, ReportPeer>>(emptyMap()) }

    LaunchedEffect(Unit) {
        if (area == null) area = withContext(Dispatchers.IO) { runCatching { reportGuessArea() }.getOrDefault("other") }
    }

    /** Makes the report; with [ask], asks each connected device for its part first. */
    fun makeReport(ask: Boolean = false) {
        busy = true
        send = SendState.Idle
        val others = state.devices.map(Reports::other)
        val chosen = area ?: "other"
        scope.launch {
            if (ask) {
                val log = parts and 16 != 0
                peers = state.devices.associate { device ->
                    device.id to if (device.online) onAsk(device.id, log, justNow) ?: ReportPeer.OFFLINE else ReportPeer.OFFLINE
                }
            }
            preview = withContext(Dispatchers.IO) {
                runCatching {
                    reportPrepare(chosen, otherArea, text, justNow, parts, Reports.phone(context), others)
                }.getOrDefault("")
            }
            busy = false
        }
    }

    // A device's log arrived (or its user said no): make the report again with it.
    LaunchedEffect(reportLogs) {
        reportLogs.collect { id ->
            peers = peers - id
            if (step == 1) makeReport()
        }
    }

    BackHandler { if (step == 1) step = 0 else onBack() }

    Column(
        modifier = modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            TextButton(
                onClick = { if (step == 1) step = 0 else onBack() },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            ) {
                Text(stringResource(R.string.report_back))
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(
                stringResource(if (step == 0) R.string.report_title else R.string.report_your_report),
                style = MaterialTheme.typography.headlineMedium,
                modifier = Modifier.weight(1f),
            )
            if (busy) CircularProgressIndicator(strokeWidth = 2.dp, modifier = Modifier.padding(4.dp))
        }

        if (step == 0) {
            Text(stringResource(R.string.report_about), style = MaterialTheme.typography.titleSmall)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AREAS.forEach { (id, label) ->
                    FilterChip(selected = area == id, onClick = { area = id }, label = { Text(stringResource(label)) })
                }
            }
            if (area == "other") {
                OutlinedTextField(
                    value = otherArea,
                    onValueChange = { otherArea = it.take(60) },
                    label = { Text(stringResource(R.string.report_other_hint)) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            Text(stringResource(R.string.report_what), style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(
                value = text,
                onValueChange = { text = it.take(MAX_TEXT) },
                placeholder = { Text(stringResource(R.string.report_what_hint)) },
                modifier = Modifier.fillMaxWidth().heightIn(min = 140.dp),
            )
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.report_just_now), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                Switch(checked = justNow, onCheckedChange = { justNow = it })
            }
            Row(horizontalArrangement = Arrangement.End, modifier = Modifier.fillMaxWidth()) {
                Button(
                    enabled = text.isNotBlank() || area != "other",
                    onClick = {
                        step = 1
                        copied = false
                        makeReport(ask = true)
                    },
                ) {
                    Text(stringResource(R.string.report_next))
                }
            }
        } else {
            val ready = !busy && preview.isNotEmpty()
            Text(stringResource(R.string.report_public), style = MaterialTheme.typography.bodyMedium)
            if (reportLooksLikeContact(text) || reportLooksLikeContact(otherArea)) {
                Text(
                    stringResource(R.string.report_contact_warning),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.horizontalScroll(rememberScrollState())) {
                PARTS.forEach { (bit, label) ->
                    FilterChip(
                        selected = parts and bit != 0,
                        onClick = {
                            parts = parts xor bit
                            makeReport()
                        },
                        label = { Text(stringResource(label)) },
                    )
                }
            }
            Surface(
                color = MaterialTheme.colorScheme.surfaceContainerHighest,
                shape = MaterialTheme.shapes.medium,
                modifier = Modifier.fillMaxWidth().heightIn(min = 200.dp),
            ) {
                SelectionContainer {
                    Text(
                        preview,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 11.sp,
                        lineHeight = 15.sp,
                        modifier = Modifier.padding(12.dp),
                    )
                }
            }
            peers.forEach { (id, peer) ->
                val name = state.nameOf(id).orEmpty().ifEmpty { stringResource(R.string.your_pc) }
                val note = when (peer) {
                    ReportPeer.ASKING -> R.string.report_peer_asking
                    ReportPeer.BUSY -> R.string.report_peer_busy
                    ReportPeer.OFFLINE -> R.string.report_peer_offline
                    ReportPeer.UNSUPPORTED -> R.string.report_peer_old
                    ReportPeer.INCLUDED -> null
                }
                if (note != null) {
                    Text(
                        stringResource(note, name),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            if (copied) {
                Text(
                    stringResource(R.string.report_copied),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            SendStatus(send, onView = { issue -> runCatching { uriHandler.openUri(issueUrl(issue)) } })
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                modifier = Modifier.fillMaxWidth(),
            ) {
                TextButton(enabled = ready, onClick = {
                    copyReport(context, preview)
                    copied = true
                }) {
                    Text(stringResource(R.string.report_copy))
                }
                FilledTonalButton(enabled = ready, onClick = {
                    copyReport(context, preview)
                    copied = true
                    reportIssueUrl()?.let { url -> runCatching { uriHandler.openUri(url) } }
                }) {
                    Text(stringResource(R.string.report_open_github))
                }
                val canSend = when (val s = send) {
                    SendState.Idle -> true
                    SendState.Sending -> false
                    is SendState.Done -> s.outcome is ReportOutcome.Limited || s.outcome is ReportOutcome.Failed
                }
                Button(enabled = ready && canSend, onClick = {
                    send = SendState.Sending
                    scope.launch {
                        val outcome = withContext(Dispatchers.IO) {
                            runCatching { reportOutgoing(null)?.let(Reports::deliver) }.getOrNull()
                        } ?: ReportOutcome.Failed
                        send = SendState.Done(outcome)
                    }
                }) {
                    Text(stringResource(R.string.report_send))
                }
            }
        }
    }
}

@Composable
private fun SendStatus(send: SendState, onView: (ULong) -> Unit) {
    val (message, warn) = when (send) {
        SendState.Idle -> return
        SendState.Sending -> stringResource(R.string.report_sending) to false
        is SendState.Done -> when (val o = send.outcome) {
            is ReportOutcome.Filed -> stringResource(R.string.report_filed, o.issue.toLong()) to false
            ReportOutcome.Queued -> stringResource(R.string.report_queued) to false
            ReportOutcome.Later -> stringResource(R.string.report_later) to false
            ReportOutcome.Limited -> stringResource(R.string.report_limited) to true
            ReportOutcome.Failed -> stringResource(R.string.report_failed) to true
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Text(
            message,
            style = MaterialTheme.typography.bodySmall,
            color = if (warn) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f),
        )
        val filed = (send as? SendState.Done)?.outcome as? ReportOutcome.Filed
        if (filed != null) {
            TextButton(onClick = { onView(filed.issue) }) { Text(stringResource(R.string.report_view)) }
        }
    }
}

private fun issueUrl(issue: ULong) = "https://github.com/thtbee/nectarlink/issues/$issue"

/**
 * Puts the report on the clipboard, marked sensitive so it isn't shown in
 * the copy preview, and so NectarLink doesn't send it to the PC as a copy.
 */
private fun copyReport(context: Context, text: String) {
    val clip = ClipData.newPlainText("NectarLink report", text).apply {
        description.extras = PersistableBundle().apply {
            // ClipDescription.EXTRA_IS_SENSITIVE, from Android 13; older ones ignore it.
            putBoolean("android.content.extra.IS_SENSITIVE", true)
        }
    }
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
}

/** Where the crash card's Send got to. */
enum class CrashSend { Idle, Sending, Sent, Failed }

/**
 * After a crash: a card on Home offering to send a crash report (where in
 * NectarLink it happened, versions, checks and feature health, never the
 * log). Once per crash per version; "Don't ask about crashes" turns it off.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun CrashCardView(
    send: CrashSend,
    fixedIn: String?,
    onUpdate: () -> Unit,
    onSend: () -> Unit,
    onLookFirst: () -> Unit,
    onNotNow: () -> Unit,
    onOff: () -> Unit,
) {
    Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.secondaryContainer) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                stringResource(if (send == CrashSend.Sent) R.string.crash_card_sent else R.string.crash_card_title),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            if (send != CrashSend.Sent) {
                Text(
                    when {
                        fixedIn != null -> stringResource(R.string.crash_card_fixed, fixedIn)
                        send == CrashSend.Failed -> stringResource(R.string.crash_card_failed)
                        else -> stringResource(R.string.crash_card_body)
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                )
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (fixedIn != null) {
                        Button(onClick = onUpdate) { Text(stringResource(R.string.crash_card_update)) }
                    } else {
                        Button(enabled = send == CrashSend.Idle, onClick = onSend) { Text(stringResource(R.string.report_send)) }
                        FilledTonalButton(enabled = send != CrashSend.Sending, onClick = onLookFirst) {
                            Text(stringResource(R.string.crash_card_look))
                        }
                    }
                    TextButton(enabled = send != CrashSend.Sending, onClick = onNotNow) {
                        Text(stringResource(R.string.action_not_now))
                    }
                }
                TextButton(onClick = onOff, contentPadding = PaddingValues(0.dp)) {
                    Text(stringResource(R.string.crash_card_off))
                }
            }
        }
    }
}
