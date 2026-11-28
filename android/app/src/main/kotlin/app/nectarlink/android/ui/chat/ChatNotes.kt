// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.chat

import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import java.util.Calendar
import java.util.TimeZone

/**
 * What the chat with one paired device shows (#698): notes typed on one
 * device, and the files, photos and recordings sent between the two.
 */
@androidx.compose.runtime.Immutable
sealed interface ChatItem {
    /** The timeline entry's ID. */
    val entryId: Long

    /** Came from the other device. */
    val incoming: Boolean

    /** Unix seconds. */
    val timestamp: Long

    /** The list key: stays the same while the item is on screen. */
    val key: String
}

/**
 * A note: notes typed on one device show on the other. Each note is a
 * timeline entry of kind MESSAGE, its text in `detail` and `chat:<id>` in
 * `refId`.
 */
@androidx.compose.runtime.Immutable
data class ChatNote(
    override val entryId: Long,
    /** The ID both devices know the note by. */
    val noteId: String,
    val text: String,
    /** Written on the other device. */
    override val incoming: Boolean,
    override val timestamp: Long,
) : ChatItem {
    override val key: String get() = "note_$noteId"
}

/**
 * A file, photo or recording sent between the two devices: a timeline entry
 * of kind FILE, PHOTO or RECORDING. Only this phone's timeline has it; it
 * isn't shared like a note.
 */
@androidx.compose.runtime.Immutable
data class ChatAttachment(
    override val entryId: Long,
    val kind: TimelineKind,
    /** The file's name, or "a.jpg and 2 more" for several. */
    val name: String,
    /** Where it is on this phone: a path or a `content://` URI, one per line. */
    val target: String,
    val sizeBytes: Long,
    override val incoming: Boolean,
    override val timestamp: Long,
) : ChatItem {
    override val key: String get() = "file_$entryId"

    /** The one file it's about on this phone, or null for several or none. */
    val singleTarget: String?
        get() = target.trim().takeIf { it.isNotEmpty() && "\n" !in it }
}

/** The timeline kinds the chat shows. */
val CHAT_KINDS: List<TimelineKind> = listOf(
    TimelineKind.MESSAGE,
    TimelineKind.FILE,
    TimelineKind.PHOTO,
    TimelineKind.RECORDING,
)

/** The timeline kinds the chat shows as attachments. */
private val ATTACHMENT_KINDS = setOf(TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING)

/** What a tap on an attachment offers. */
enum class AttachmentAction { Open, Share, Remove }

/** What an attachment shows: a thumbnail for images and videos, else an icon. */
enum class AttachmentLook { Image, Video, Audio, File }

/** What a long press on a note offers. */
enum class NoteAction { Copy, Edit, Delete }

/** Why the composer can't send now. */
enum class ComposerBlock {
    /** The device isn't connected. */
    Offline,

    /** The device's app doesn't take notes yet. */
    TooOld,
}

/** The longest note, in characters (the protocol's `chat::TEXT_CHARS`). */
const val CHAT_MAX_CHARS = 8192

private const val REF_PREFIX = "chat:"

/** The note ID in a timeline entry's `refId`, or null when it isn't a note's. */
fun chatNoteId(refId: String?): String? =
    refId?.takeIf { it.startsWith(REF_PREFIX) }?.removePrefix(REF_PREFIX)?.takeIf { it.isNotEmpty() }

/**
 * The notes of the chat with [deviceId] among [entries] (the timeline, in any
 * order), oldest first: the order the chat shows them in, top to bottom.
 */
fun chatNotes(entries: List<TimelineEntry>, deviceId: String): List<ChatNote> =
    entries.asSequence()
        .filter { it.kind == TimelineKind.MESSAGE && it.deviceId == deviceId }
        .mapNotNull { entry ->
            val noteId = chatNoteId(entry.refId) ?: return@mapNotNull null
            ChatNote(
                entryId = entry.id,
                noteId = noteId,
                text = entry.detail,
                incoming = entry.incoming,
                timestamp = entry.timestamp,
            )
        }
        .distinctBy { it.noteId }
        .sortedWith(compareBy<ChatNote>({ it.timestamp }, { it.entryId }))
        .toList()

/**
 * The chat with [deviceId] among [entries] (the timeline, in any order),
 * oldest first: notes and attachments in the order the chat shows them, top
 * to bottom. Each entry shows once, and each note once.
 */
fun chatItems(entries: List<TimelineEntry>, deviceId: String): List<ChatItem> {
    val seen = HashSet<Long>()
    val notes = chatNotes(entries, deviceId)
    notes.forEach { seen += it.entryId }
    val attachments = entries.asSequence()
        .filter { it.kind in ATTACHMENT_KINDS && it.deviceId == deviceId }
        .filter { seen.add(it.id) }
        .map { entry ->
            ChatAttachment(
                entryId = entry.id,
                kind = entry.kind,
                name = entry.title,
                target = entry.target,
                sizeBytes = entry.sizeBytes.toLong(),
                incoming = entry.incoming,
                timestamp = entry.timestamp,
            )
        }
        .toList()
    return (notes + attachments).sortedWith(compareBy<ChatItem>({ it.timestamp }, { it.entryId }))
}

/**
 * The newest [limit] entries of several timeline reads (each newest first,
 * with whether it has more), newest first, and whether there are more. Each
 * read must hold its kind's newest [limit] entries, or all of them.
 */
fun newestOf(reads: List<Pair<List<TimelineEntry>, Boolean>>, limit: Int): Pair<List<TimelineEntry>, Boolean> {
    val all = reads.asSequence()
        .flatMap { it.first.asSequence() }
        .distinctBy { it.id }
        .sortedWith(compareByDescending<TimelineEntry> { it.timestamp }.thenByDescending { it.id })
        .toList()
    val more = all.size > limit || reads.any { it.second }
    return all.take(limit) to more
}

/**
 * What an attachment offers: Open and Share while its file is on this phone
 * ([onPhone]), and Remove, which takes it off this phone's timeline.
 */
fun attachmentActions(onPhone: Boolean): List<AttachmentAction> = buildList {
    if (onPhone) {
        add(AttachmentAction.Open)
        add(AttachmentAction.Share)
    }
    add(AttachmentAction.Remove)
}

private val IMAGE_EXTENSIONS = setOf("jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "avif")
private val VIDEO_EXTENSIONS = setOf("mp4", "mkv", "webm", "mov", "3gp", "avi", "m4v")
private val AUDIO_EXTENSIONS = setOf("mp3", "m4a", "aac", "ogg", "opus", "wav", "flac", "amr")

/** How an attachment shows, from its kind and its file's name. */
fun attachmentLook(attachment: ChatAttachment): AttachmentLook {
    val file = attachment.singleTarget
        ?.substringAfterLast('/')
        ?.takeIf { '.' in it }
        ?: attachment.name
    val ext = file.substringAfterLast('.', "").lowercase()
    return when {
        ext in IMAGE_EXTENSIONS -> AttachmentLook.Image
        ext in VIDEO_EXTENSIONS -> AttachmentLook.Video
        ext in AUDIO_EXTENSIONS -> AttachmentLook.Audio
        attachment.kind == TimelineKind.PHOTO -> AttachmentLook.Image
        attachment.kind == TimelineKind.RECORDING -> AttachmentLook.Audio
        else -> AttachmentLook.File
    }
}

/**
 * What a note offers: anyone's can be copied and deleted (on both devices);
 * only this phone's can be edited, and only while a change can reach the
 * other device ([canSend]).
 */
fun noteActions(note: ChatNote, canSend: Boolean): List<NoteAction> = buildList {
    add(NoteAction.Copy)
    if (!note.incoming && canSend) add(NoteAction.Edit)
    add(NoteAction.Delete)
}

/** Why nothing can be sent to the device now, or null when it can. */
fun composerBlock(online: Boolean, tooOld: Boolean): ComposerBlock? = when {
    !online -> ComposerBlock.Offline
    tooOld -> ComposerBlock.TooOld
    else -> null
}

/** The draft as it's sent: without blank lines around it or trailing spaces. */
fun noteText(draft: String): String = draft.trimEnd().trimStart('\n', '\r')

/** How many characters [text] is over the limit, or 0. Counted as the core counts them. */
fun charsOver(text: String): Int = (text.codePointCount(0, text.length) - CHAT_MAX_CHARS).coerceAtLeast(0)

/** Whether the draft can be sent: some text, and not too long. */
fun sendable(draft: String): Boolean {
    val text = noteText(draft)
    return text.isNotBlank() && charsOver(text) == 0
}

/** Whether the item at [index] starts a new day, so a day label goes above it. */
fun startsDay(notes: List<ChatItem>, index: Int, zone: TimeZone = TimeZone.getDefault()): Boolean {
    if (index == 0) return true
    val prev = Calendar.getInstance(zone).apply { timeInMillis = notes[index - 1].timestamp * 1000L }
    val cur = Calendar.getInstance(zone).apply { timeInMillis = notes[index].timestamp * 1000L }
    return prev.get(Calendar.YEAR) != cur.get(Calendar.YEAR) ||
        prev.get(Calendar.DAY_OF_YEAR) != cur.get(Calendar.DAY_OF_YEAR)
}
