// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.diag.ReportNotices
import app.nectarlink.android.diag.Reports
import app.nectarlink.core.ReportAnswer
import app.nectarlink.core.ReportConversation
import app.nectarlink.core.ReportReplyOutcome
import java.text.DateFormat
import java.time.Instant
import java.util.Date
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Your reports: the problem reports this phone sent, what was answered on
 * them, and a field to reply, without a GitHub account. Answers are read
 * from the issue on GitHub; replies go through the report service.
 */
@Composable
fun YourReportsScreen(
    focusRid: String?,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    var conversations by remember { mutableStateOf<List<ReportConversation>>(emptyList()) }
    // Answers by report; null once GitHub couldn't be reached.
    val threads = remember { mutableStateMapOf<String, List<ReportAnswer>?>() }
    var loading by remember { mutableStateOf(false) }

    fun load() {
        if (loading) return
        loading = true
        scope.launch {
            val list = withContext(Dispatchers.IO) { Reports.conversations() }
            // The one a notification was about, first.
            conversations = list.sortedByDescending { it.rid == focusRid }
            list.forEach { conversation ->
                threads[conversation.rid] = withContext(Dispatchers.IO) { Reports.thread(conversation) }
            }
            loading = false
        }
    }
    LaunchedEffect(Unit) { load() }

    BackHandler { onBack() }

    Column(
        modifier = modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            TextButton(onClick = onBack, contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)) {
                Text(stringResource(R.string.report_back))
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(
                stringResource(R.string.your_reports_title),
                style = MaterialTheme.typography.headlineMedium,
                modifier = Modifier.weight(1f),
            )
            if (loading) {
                CircularProgressIndicator(strokeWidth = 2.dp, modifier = Modifier.padding(4.dp))
            } else {
                TextButton(onClick = ::load) { Text(stringResource(R.string.your_reports_check)) }
            }
        }
        Text(
            stringResource(R.string.your_reports_intro),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (!loading && conversations.isEmpty()) {
            Text(stringResource(R.string.your_reports_none), style = MaterialTheme.typography.bodyLarge)
        }
        conversations.forEach { conversation ->
            Conversation(
                conversation = conversation,
                answers = threads[conversation.rid],
                loaded = threads.containsKey(conversation.rid),
                focused = conversation.rid == focusRid,
                onSent = ::load,
            )
        }
    }
}

@Composable
private fun Conversation(
    conversation: ReportConversation,
    answers: List<ReportAnswer>?,
    loaded: Boolean,
    focused: Boolean,
    onSent: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val uriHandler = LocalUriHandler.current
    var reply by remember(conversation.rid) { mutableStateOf("") }
    var sending by remember(conversation.rid) { mutableStateOf(false) }
    var outcome by remember(conversation.rid) { mutableStateOf<ReportReplyOutcome?>(null) }
    val issue = conversation.issue
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        border = if (focused) androidx.compose.foundation.BorderStroke(2.dp, MaterialTheme.colorScheme.primary) else null,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        when {
                            issue == null -> stringResource(R.string.your_reports_waiting)
                            conversation.closed -> stringResource(R.string.your_reports_closed, issue.toLong())
                            else -> stringResource(R.string.your_reports_number, issue.toLong())
                        },
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        stringResource(
                            R.string.your_reports_sent,
                            DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(conversation.at.toLong() * 1000)),
                        ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (issue != null) {
                    TextButton(onClick = { runCatching { uriHandler.openUri("https://github.com/thtbee/nectarlink/issues/$issue") } }) {
                        Text(stringResource(R.string.your_reports_on_github))
                    }
                }
            }
            when {
                issue == null -> {}
                !loaded -> {}
                answers == null -> Text(
                    stringResource(R.string.your_reports_load_failed),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                answers.isEmpty() -> Text(
                    stringResource(R.string.your_reports_no_answers),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                else -> answers.forEach { answer -> Answer(answer, ReportNotices.who(context, answer)) }
            }
            if (issue != null && !conversation.canReply) {
                Text(
                    stringResource(R.string.your_reports_reply_on_github),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (conversation.canReply) {
                OutlinedTextField(
                    value = reply,
                    onValueChange = { reply = it.take(2000) },
                    label = { Text(stringResource(R.string.report_reply_hint)) },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 96.dp),
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    val status = when (outcome) {
                        ReportReplyOutcome.SENT -> R.string.your_reports_reply_sent
                        ReportReplyOutcome.LIMITED -> R.string.report_reply_limited
                        ReportReplyOutcome.LATER -> R.string.report_reply_later
                        ReportReplyOutcome.REFUSED -> R.string.report_reply_refused
                        null -> null
                    }
                    Text(
                        if (sending) stringResource(R.string.your_reports_sending) else status?.let { stringResource(it) }.orEmpty(),
                        style = MaterialTheme.typography.bodySmall,
                        color = if (outcome == null || outcome == ReportReplyOutcome.SENT) {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        } else {
                            MaterialTheme.colorScheme.error
                        },
                        modifier = Modifier.weight(1f),
                    )
                    Button(
                        enabled = reply.isNotBlank() && !sending,
                        onClick = {
                            sending = true
                            outcome = null
                            val text = reply
                            scope.launch {
                                val result = withContext(Dispatchers.IO) { Reports.reply(conversation.rid, text) }
                                sending = false
                                outcome = result
                                if (result == ReportReplyOutcome.SENT) {
                                    reply = ""
                                    onSent()
                                }
                            }
                        },
                    ) {
                        Text(stringResource(R.string.your_reports_send))
                    }
                }
            }
        }
    }
}

@Composable
private fun Answer(answer: ReportAnswer, who: String) {
    val mine = answer.who == "you"
    Surface(
        shape = MaterialTheme.shapes.medium,
        color = if (mine) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerHighest,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            val at = runCatching {
                DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date.from(Instant.parse(answer.at)))
            }.getOrDefault("")
            Text(
                if (at.isEmpty()) who else "$who · $at",
                style = MaterialTheme.typography.labelLarge,
                color = if (mine) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SelectionContainer {
                Text(
                    answer.text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (mine) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}
