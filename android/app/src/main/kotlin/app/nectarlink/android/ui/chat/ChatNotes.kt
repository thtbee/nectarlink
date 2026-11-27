// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.chat

import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import java.util.Calendar
import java.util.TimeZone

/**
 * The chat between this phone and one paired device (#698): notes typed on
 * one show on the other. Each note is a timeline entry of kind MESSAGE, its
 * text in `detail` and `chat:<id>` in `refId`.
 */
@androidx.compose.runtime.Immutable
data class ChatNote(
    /** The timeline entry's ID, for list keys. */
    val entryId: Long,
    /** The ID both devices know the note by. */
    val noteId: String,
    val text: String,
    /** Written on the other device. */
    val incoming: Boolean,
    /** Unix seconds. */
    val timestamp: Long,
)

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

/** Whether the note at [index] starts a new day, so a day label goes above it. */
fun startsDay(notes: List<ChatNote>, index: Int, zone: TimeZone = TimeZone.getDefault()): Boolean {
    if (index == 0) return true
    val prev = Calendar.getInstance(zone).apply { timeInMillis = notes[index - 1].timestamp * 1000L }
    val cur = Calendar.getInstance(zone).apply { timeInMillis = notes[index].timestamp * 1000L }
    return prev.get(Calendar.YEAR) != cur.get(Calendar.YEAR) ||
        prev.get(Calendar.DAY_OF_YEAR) != cur.get(Calendar.DAY_OF_YEAR)
}
