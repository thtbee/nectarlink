// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.chat

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.chat.ChatNotifications
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.home.timelineDayLabel
import app.nectarlink.android.ui.theme.glassModalContainerColor
import java.util.Date
import kotlinx.coroutines.launch

/** How many notes the chat reads at first, and how many more each time it's asked. */
private const val NOTES_STEP = 250

/**
 * The chat with one paired device (#698): notes typed here show there and
 * the other way round, oldest at the top. The other device's notes sit on
 * the left, this phone's on the right; a long press copies, changes or
 * deletes one.
 */
@Composable
fun ChatScreen(
    device: Device,
    core: Core,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val revision by core.timelineRevision.collectAsStateWithLifecycle()
    val unsupported by core.chatUnsupported.collectAsStateWithLifecycle()
    var limit by remember(device.id) { mutableIntStateOf(NOTES_STEP) }
    var notes by remember(device.id) { mutableStateOf<List<ChatNote>>(emptyList()) }
    var hasMore by remember(device.id) { mutableStateOf(false) }
    var loaded by remember(device.id) { mutableStateOf(false) }

    LaunchedEffect(device.id, revision, limit) {
        val read = core.chatEntries(device.id, limit) ?: return@LaunchedEffect
        notes = chatNotes(read.first, device.id)
        hasMore = read.second
        loaded = true
    }

    // On screen while the app is in front: no notification for this chat, and none left over.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, device.id) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> {
                    ChatNotifications.onScreen = device.id
                    ChatNotifications.cancel(context, device.id)
                }
                Lifecycle.Event.ON_PAUSE -> if (ChatNotifications.onScreen == device.id) ChatNotifications.onScreen = null
                else -> Unit
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        if (lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) {
            ChatNotifications.onScreen = device.id
            ChatNotifications.cancel(context, device.id)
        }
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            if (ChatNotifications.onScreen == device.id) ChatNotifications.onScreen = null
        }
    }

    // Known from what it offered when it connected, or from a note it turned down.
    val tooOld = device.id in unsupported || (device.online && !core.chatOffered(device.id))
    val block = composerBlock(online = device.online, tooOld = tooOld)
    var draft by rememberSaveable(device.id) { mutableStateOf("") }
    var editingId by rememberSaveable(device.id) { mutableStateOf<String?>(null) }
    var sending by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<ChatNote?>(null) }

    // The note being changed is gone (deleted on the other device): nothing left to change.
    LaunchedEffect(notes, loaded) {
        val id = editingId ?: return@LaunchedEffect
        if (loaded && notes.none { it.noteId == id }) {
            editingId = null
            draft = ""
        }
    }

    val listState = rememberLazyListState()
    // The newest note, at the bottom, stays in view as notes arrive.
    val newest = notes.lastOrNull()?.noteId
    LaunchedEffect(newest) {
        if (newest != null) listState.animateScrollToItem(0)
    }

    fun send() {
        val text = noteText(draft)
        if (!sendable(text) || sending) return
        val editing = editingId
        sending = true
        scope.launch {
            val ok = if (editing != null) {
                core.editChat(device.id, editing, text)
            } else {
                core.sendChat(device.id, text)
            }
            sending = false
            if (ok) {
                draft = ""
                editingId = null
            }
        }
    }

    Column(modifier = modifier.fillMaxSize()) {
        ChatHeader(device = device, onBack = onBack)
        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (loaded && notes.isEmpty()) {
                Text(
                    stringResource(R.string.chat_empty, device.name),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.align(Alignment.Center).padding(horizontal = 32.dp),
                )
            }
            val todayLabel = stringResource(R.string.timeline_today)
            val yesterdayLabel = stringResource(R.string.timeline_yesterday)
            val timeFormat = remember(context) { android.text.format.DateFormat.getTimeFormat(context) }
            // Bottom up: the newest note is the first item, so the list opens at it
            // and stays there while the keyboard opens.
            LazyColumn(
                state = listState,
                reverseLayout = true,
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                for (index in notes.indices.reversed()) {
                    val note = notes[index]
                    item(key = "note_${note.noteId}", contentType = "note") {
                        NoteBubble(
                            note = note,
                            deviceName = device.name,
                            time = timeFormat.format(Date(note.timestamp * 1000L)),
                            actions = noteActions(note, canSend = block == null),
                            onAction = { action ->
                                when (action) {
                                    NoteAction.Copy -> PhoneClipboard.write(context, note.text)
                                    NoteAction.Edit -> {
                                        editingId = note.noteId
                                        draft = note.text
                                    }
                                    NoteAction.Delete -> deleting = note
                                }
                            },
                        )
                    }
                    if (startsDay(notes, index)) {
                        item(key = "day_${note.noteId}", contentType = "day") {
                            Text(
                                timelineDayLabel(note.timestamp, todayLabel, yesterdayLabel),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(top = 10.dp, bottom = 4.dp)
                                    .semantics { heading() },
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                            )
                        }
                    }
                }
                if (hasMore) {
                    item(key = "earlier", contentType = "earlier") {
                        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                            TextButton(onClick = { limit += NOTES_STEP }) {
                                Text(stringResource(R.string.chat_earlier))
                            }
                        }
                    }
                }
            }
        }
        Composer(
            device = device,
            draft = draft,
            onDraft = { draft = it },
            editing = editingId != null,
            onCancelEdit = {
                editingId = null
                draft = ""
            },
            block = block,
            sending = sending,
            onSend = ::send,
        )
    }

    deleting?.let { note ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.chat_delete_title)) },
            text = {
                Text(
                    stringResource(
                        if (device.online) R.string.chat_delete_text else R.string.chat_delete_text_offline,
                        device.name,
                    ),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    deleting = null
                    if (editingId == note.noteId) {
                        editingId = null
                        draft = ""
                    }
                    core.deleteChat(device.id, note.noteId)
                }) { Text(stringResource(R.string.chat_action_delete)) }
            },
            dismissButton = {
                TextButton(onClick = { deleting = null }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@Composable
private fun ChatHeader(device: Device, onBack: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onBack) {
            Icon(
                painter = painterResource(R.drawable.ic_back),
                contentDescription = stringResource(R.string.action_back),
            )
        }
        Spacer(Modifier.width(4.dp))
        Column(Modifier.weight(1f).semantics(mergeDescendants = true) { heading() }) {
            Text(device.name, style = MaterialTheme.typography.titleLarge, maxLines = 1)
            Text(
                stringResource(if (device.online) R.string.link_online else R.string.widget_pc_offline),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val dot = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
        Spacer(
            modifier = Modifier
                .padding(end = 16.dp)
                .size(10.dp)
                .drawBehind { drawCircle(dot) },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun NoteBubble(
    note: ChatNote,
    deviceName: String,
    time: String,
    actions: List<NoteAction>,
    onAction: (NoteAction) -> Unit,
) {
    val mine = !note.incoming
    var menuOpen by remember { mutableStateOf(false) }
    val described = if (mine) {
        stringResource(R.string.chat_note_mine, time, note.text)
    } else {
        stringResource(R.string.chat_note_from, deviceName, time, note.text)
    }
    val optionsLabel = stringResource(R.string.chat_note_options)
    val shape = if (mine) {
        RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp, bottomStart = 20.dp, bottomEnd = 6.dp)
    } else {
        RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp, bottomStart = 6.dp, bottomEnd = 20.dp)
    }
    Box(Modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth(0.82f)
                .align(if (mine) Alignment.CenterEnd else Alignment.CenterStart),
            contentAlignment = if (mine) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            Surface(
                shape = shape,
                color = if (mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
                contentColor = if (mine) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
                modifier = Modifier
                    .combinedClickable(
                        onClickLabel = optionsLabel,
                        onLongClickLabel = optionsLabel,
                        onLongClick = { menuOpen = true },
                        onClick = { menuOpen = true },
                    )
                    .clearAndSetSemantics { contentDescription = described },
            ) {
                Column(Modifier.padding(start = 14.dp, end = 14.dp, top = 9.dp, bottom = 7.dp)) {
                    Text(note.text, style = MaterialTheme.typography.bodyLarge)
                    Text(
                        time,
                        style = MaterialTheme.typography.labelSmall,
                        color = LocalContentColor.current.copy(alpha = 0.72f),
                        modifier = Modifier
                            .align(Alignment.End)
                            .padding(top = 2.dp),
                    )
                }
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                actions.forEach { action ->
                    DropdownMenuItem(
                        text = {
                            Text(
                                stringResource(
                                    when (action) {
                                        NoteAction.Copy -> R.string.chat_action_copy
                                        NoteAction.Edit -> R.string.chat_action_edit
                                        NoteAction.Delete -> R.string.chat_action_delete
                                    },
                                ),
                            )
                        },
                        onClick = {
                            menuOpen = false
                            onAction(action)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun Composer(
    device: Device,
    draft: String,
    onDraft: (String) -> Unit,
    editing: Boolean,
    onCancelEdit: () -> Unit,
    block: ComposerBlock?,
    sending: Boolean,
    onSend: () -> Unit,
) {
    val over = charsOver(noteText(draft))
    Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, top = 8.dp, bottom = 10.dp)) {
            if (editing) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        stringResource(R.string.chat_editing),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = onCancelEdit) { Text(stringResource(R.string.action_cancel)) }
                }
            }
            val reason = when (block) {
                ComposerBlock.Offline -> stringResource(R.string.chat_offline, device.name)
                ComposerBlock.TooOld -> stringResource(R.string.chat_too_old, device.name)
                null -> null
            }
            if (reason != null) {
                Text(
                    reason,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(bottom = 6.dp),
                )
            }
            if (over > 0) {
                Text(
                    stringResource(R.string.chat_too_long, CHAT_MAX_CHARS),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(bottom = 6.dp),
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                val label = stringResource(R.string.chat_composer_label, device.name)
                OutlinedTextField(
                    value = draft,
                    onValueChange = onDraft,
                    enabled = block == null,
                    placeholder = { Text(stringResource(R.string.chat_composer_hint)) },
                    maxLines = 6,
                    isError = over > 0,
                    shape = RoundedCornerShape(24.dp),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = 52.dp)
                        .semantics { contentDescription = label },
                )
                Spacer(Modifier.width(8.dp))
                FilledIconButton(
                    onClick = onSend,
                    enabled = block == null && !sending && sendable(draft),
                    modifier = Modifier.size(48.dp),
                ) {
                    Icon(
                        imageVector = SendIcon,
                        contentDescription = stringResource(if (editing) R.string.chat_save else R.string.chat_send),
                        modifier = Modifier.size(22.dp),
                    )
                }
            }
        }
    }
}

private val SendIcon: ImageVector by lazy {
    ImageVector.Builder(
        name = "ChatSend",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).addPath(
        pathData = PathParser().parsePathString("M12,19V5 M6,11L12,5L18,11").toNodes(),
        fill = null,
        stroke = SolidColor(Color.White),
        strokeLineWidth = 2.2f,
        strokeLineCap = StrokeCap.Round,
        strokeLineJoin = StrokeJoin.Round,
    ).build()
}
