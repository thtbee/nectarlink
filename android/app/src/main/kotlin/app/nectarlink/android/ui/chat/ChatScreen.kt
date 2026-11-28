// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.chat

import android.graphics.BitmapFactory
import android.media.ThumbnailUtils
import android.net.Uri
import android.os.Build
import android.text.format.Formatter
import android.util.LruCache
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
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
import app.nectarlink.android.ui.home.openTimelineFile
import app.nectarlink.android.ui.home.shareTimelineFile
import app.nectarlink.android.ui.home.timelineDayLabel
import app.nectarlink.android.ui.theme.glassModalContainerColor
import java.io.File
import java.io.InputStream
import java.util.Date
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** How many entries the chat reads at first, and how many more each time it's asked. */
private const val NOTES_STEP = 250

/**
 * The chat with one paired device (#698): notes typed here show there and
 * the other way round, oldest at the top, with the files, photos and
 * recordings sent between the two among them. What the other device sent
 * sits on the left, this phone's on the right; a tap on a note copies,
 * changes or deletes it, one on a file opens, shares or removes it.
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
    var items by remember(device.id) { mutableStateOf<List<ChatItem>>(emptyList()) }
    var hasMore by remember(device.id) { mutableStateOf(false) }
    var loaded by remember(device.id) { mutableStateOf(false) }

    LaunchedEffect(device.id, revision, limit) {
        val read = core.chatEntries(device.id, limit) ?: return@LaunchedEffect
        items = chatItems(read.first, device.id)
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
    LaunchedEffect(items, loaded) {
        val id = editingId ?: return@LaunchedEffect
        if (loaded && items.none { it is ChatNote && it.noteId == id }) {
            editingId = null
            draft = ""
        }
    }

    val listState = rememberLazyListState()
    // The newest item, at the bottom, stays in view as notes and files arrive.
    val newest = items.lastOrNull()?.key
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
            if (loaded && items.isEmpty()) {
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
                for (index in items.indices.reversed()) {
                    val chatItem = items[index]
                    if (chatItem is ChatAttachment) {
                        item(key = chatItem.key, contentType = "file") {
                            AttachmentBubble(
                                attachment = chatItem,
                                deviceName = device.name,
                                time = timeFormat.format(Date(chatItem.timestamp * 1000L)),
                                onAction = { action ->
                                    val target = chatItem.singleTarget
                                    when (action) {
                                        AttachmentAction.Open ->
                                            if (target != null) openTimelineFile(context, chatItem.kind, target)
                                        AttachmentAction.Share ->
                                            if (target != null) shareTimelineFile(context, chatItem.kind, target)
                                        AttachmentAction.Remove -> core.deleteTimelineEntry(chatItem.entryId)
                                    }
                                },
                            )
                        }
                    }
                    val note = chatItem as? ChatNote
                    if (note != null) item(key = note.key, contentType = "note") {
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
                    if (startsDay(items, index)) {
                        item(key = "day_${chatItem.key}", contentType = "day") {
                            Text(
                                timelineDayLabel(chatItem.timestamp, todayLabel, yesterdayLabel),
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
            canAttach = !device.online || device.has("pair.files"),
            onAttach = { uris -> core.sendFiles(device.id, uris) },
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

/**
 * A file, photo or recording in the chat: a thumbnail for an image or video
 * on this phone, else an icon, with its name and size. A tap opens, shares
 * or removes it.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AttachmentBubble(
    attachment: ChatAttachment,
    deviceName: String,
    time: String,
    onAction: (AttachmentAction) -> Unit,
) {
    val context = LocalContext.current
    val mine = !attachment.incoming
    val target = attachment.singleTarget
    val look = remember(attachment) { attachmentLook(attachment) }
    val onPhone by produceState(initialValue = target?.startsWith("content://") == true, target) {
        if (target != null && !target.startsWith("content://")) {
            value = withContext(Dispatchers.IO) { File(target).exists() }
        }
    }
    val thumbnail by produceState(
        initialValue = target?.let { thumbnailCache.get(it) },
        target,
        onPhone,
        look,
    ) {
        if (value != null || target == null || !onPhone) return@produceState
        if (look != AttachmentLook.Image && look != AttachmentLook.Video) return@produceState
        val loaded = loadThumbnail(context, target, look)
        if (loaded != null) thumbnailCache.put(target, loaded)
        value = loaded
    }
    var menuOpen by remember { mutableStateOf(false) }
    val size = if (attachment.sizeBytes > 0) Formatter.formatShortFileSize(context, attachment.sizeBytes) else null
    val missing = if (onPhone) null else stringResource(R.string.chat_file_missing)
    val meta = listOfNotNull(size, missing).joinToString(" · ")
    val summary = listOfNotNull(attachment.name, size, missing).joinToString(", ")
    val described = if (mine) {
        stringResource(R.string.chat_note_mine, time, summary)
    } else {
        stringResource(R.string.chat_note_from, deviceName, time, summary)
    }
    val optionsLabel = stringResource(R.string.chat_file_options)
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
                Row(
                    modifier = Modifier.padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val tile = Modifier.size(56.dp).clip(RoundedCornerShape(14.dp))
                    val image = thumbnail
                    if (image != null) {
                        Box(contentAlignment = Alignment.Center) {
                            Image(
                                bitmap = image,
                                contentDescription = null,
                                contentScale = ContentScale.Crop,
                                modifier = tile,
                            )
                            if (look == AttachmentLook.Video) {
                                Box(
                                    modifier = Modifier
                                        .size(24.dp)
                                        .clip(RoundedCornerShape(12.dp))
                                        .background(Color.Black.copy(alpha = 0.55f)),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Icon(PlayIcon, contentDescription = null, tint = Color.White, modifier = Modifier.size(14.dp))
                                }
                            }
                        }
                    } else {
                        Box(
                            modifier = tile.background(LocalContentColor.current.copy(alpha = 0.10f)),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                imageVector = if (look == AttachmentLook.Audio) AudioIcon else FileIcon,
                                contentDescription = null,
                                modifier = Modifier.size(26.dp),
                            )
                        }
                    }
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f, fill = false).padding(end = 6.dp)) {
                        Text(
                            attachment.name,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (meta.isNotEmpty()) {
                                Text(
                                    meta,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = LocalContentColor.current.copy(alpha = 0.72f),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f, fill = false),
                                )
                                Spacer(Modifier.width(8.dp))
                            }
                            Text(
                                time,
                                style = MaterialTheme.typography.labelSmall,
                                color = LocalContentColor.current.copy(alpha = 0.72f),
                            )
                        }
                    }
                }
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                attachmentActions(onPhone = onPhone && target != null).forEach { action ->
                    DropdownMenuItem(
                        text = {
                            Text(
                                stringResource(
                                    when (action) {
                                        AttachmentAction.Open -> R.string.timeline_action_open
                                        AttachmentAction.Share -> R.string.chat_action_share
                                        AttachmentAction.Remove -> R.string.chat_action_remove
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

/** The composer's attach button: photos and videos from the photo picker, or any files. */
@Composable
private fun AttachButton(enabled: Boolean, onAttach: (List<Uri>) -> Unit) {
    var menuOpen by remember { mutableStateOf(false) }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        if (uris.isNotEmpty()) onAttach(uris)
    }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) onAttach(uris)
    }
    Box {
        IconButton(onClick = { menuOpen = true }, enabled = enabled) {
            Icon(
                imageVector = AttachIcon,
                contentDescription = stringResource(R.string.chat_attach),
                modifier = Modifier.size(24.dp),
            )
        }
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.chat_attach_media)) },
                onClick = {
                    menuOpen = false
                    pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.chat_attach_files)) },
                onClick = {
                    menuOpen = false
                    pickFiles.launch(arrayOf("*/*"))
                },
            )
        }
    }
}

/** Thumbnails already made, by target. */
private val thumbnailCache = LruCache<String, ImageBitmap>(48)

/** The longest side of a thumbnail, in pixels. */
private const val THUMB_PX = 256

/** A small picture of the image or video at [target] (a path or a `content://` URI), or null. */
private suspend fun loadThumbnail(
    context: android.content.Context,
    target: String,
    look: AttachmentLook,
): ImageBitmap? = withContext(Dispatchers.IO) {
    runCatching {
        val size = Size(THUMB_PX, THUMB_PX)
        val bitmap = if (target.startsWith("content://")) {
            val uri = Uri.parse(target)
            when {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q -> context.contentResolver.loadThumbnail(uri, size, null)
                look == AttachmentLook.Image -> decodeSampled { context.contentResolver.openInputStream(uri) }
                else -> null
            }
        } else {
            val file = File(target)
            when {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q -> if (look == AttachmentLook.Image) {
                    ThumbnailUtils.createImageThumbnail(file, size, null)
                } else {
                    ThumbnailUtils.createVideoThumbnail(file, size, null)
                }
                look == AttachmentLook.Image -> decodeSampled { file.inputStream() }
                else -> @Suppress("DEPRECATION") ThumbnailUtils.createVideoThumbnail(
                    file.path,
                    android.provider.MediaStore.Images.Thumbnails.MINI_KIND,
                )
            }
        }
        bitmap?.asImageBitmap()
    }.getOrNull()
}

/** An image decoded at about [THUMB_PX] on its longest side; [open] gives its bytes, once per read. */
private fun decodeSampled(open: () -> InputStream?): android.graphics.Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    open()?.use { BitmapFactory.decodeStream(it, null, bounds) } ?: return null
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= THUMB_PX) sample *= 2
    return open()?.use { BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sample }) }
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
    canAttach: Boolean,
    onAttach: (List<Uri>) -> Unit,
) {
    val over = charsOver(noteText(draft))
    Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(start = 8.dp, end = 12.dp, top = 8.dp, bottom = 10.dp)) {
            if (editing) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        stringResource(R.string.chat_editing),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.weight(1f).padding(start = 8.dp),
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
                    modifier = Modifier.padding(start = 8.dp, bottom = 6.dp),
                )
            }
            if (over > 0) {
                Text(
                    stringResource(R.string.chat_too_long, CHAT_MAX_CHARS),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(start = 8.dp, bottom = 6.dp),
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                AttachButton(enabled = canAttach && !editing, onAttach = onAttach)
                Spacer(Modifier.width(4.dp))
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

/** A stroked 24 by 24 icon from [path]. */
private fun strokeIcon(name: String, path: String): ImageVector =
    ImageVector.Builder(
        name = name,
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).addPath(
        pathData = PathParser().parsePathString(path).toNodes(),
        fill = null,
        stroke = SolidColor(Color.White),
        strokeLineWidth = 1.8f,
        strokeLineCap = StrokeCap.Round,
        strokeLineJoin = StrokeJoin.Round,
    ).build()

private val AttachIcon: ImageVector by lazy { strokeIcon("ChatAttach", "M12,5V19 M5,12H19") }

private val FileIcon: ImageVector by lazy {
    strokeIcon("ChatFile", "M6,3H14L19,8V20A1,1 0 0,1 18,21H6A1,1 0 0,1 5,20V4A1,1 0 0,1 6,3Z M14,3V8H19")
}

private val AudioIcon: ImageVector by lazy {
    strokeIcon(
        "ChatAudio",
        "M9,18V6L19,4V16 M9,18A2.5,2.5 0 1,1 4,18A2.5,2.5 0 1,1 9,18Z M19,16A2.5,2.5 0 1,1 14,16A2.5,2.5 0 1,1 19,16Z",
    )
}

private val PlayIcon: ImageVector by lazy {
    ImageVector.Builder(
        name = "ChatPlay",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).addPath(
        pathData = PathParser().parsePathString("M8,5L19,12L8,19Z").toNodes(),
        fill = SolidColor(Color.White),
    ).build()
}
