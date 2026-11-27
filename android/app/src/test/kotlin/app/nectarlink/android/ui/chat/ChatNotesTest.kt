// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.chat

import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.TimeZone

class ChatNotesTest {
    private fun entry(
        id: Long,
        timestamp: Long,
        refId: String? = "chat:n$id",
        kind: TimelineKind = TimelineKind.MESSAGE,
        device: String = "pc",
        incoming: Boolean = true,
        detail: String = "note $id",
    ) = TimelineEntry(
        id = id,
        kind = kind,
        deviceId = device,
        deviceName = "Desk",
        incoming = incoming,
        timestamp = timestamp,
        title = "",
        detail = detail,
        target = "",
        sizeBytes = 0uL,
        durationSecs = 0uL,
        refId = refId,
        clipAvailable = false,
        imageDataUrl = null,
    )

    private fun note(incoming: Boolean, timestamp: Long = 0L, id: String = "a") =
        ChatNote(entryId = 1, noteId = id, text = "hi", incoming = incoming, timestamp = timestamp)

    @Test
    fun `a note's id is what follows chat colon in its ref`() {
        assertEquals("abc123", chatNoteId("chat:abc123"))
        assertNull(chatNoteId("chat:"))
        assertNull(chatNoteId("clip-1"))
        assertNull(chatNoteId(null))
    }

    @Test
    fun `the chat shows only that device's notes, oldest first`() {
        val timeline = listOf(
            entry(5, timestamp = 300),
            entry(4, timestamp = 200, kind = TimelineKind.CLIP, refId = "clip"),
            entry(3, timestamp = 200, device = "tablet"),
            entry(2, timestamp = 100, incoming = false, detail = "mine"),
            entry(1, timestamp = 100),
        )
        val notes = chatNotes(timeline, "pc")
        assertEquals(listOf("n1", "n2", "n5"), notes.map { it.noteId })
        assertEquals("mine", notes[1].text)
        assertFalse(notes[1].incoming)
        assertTrue(notes[0].incoming)
    }

    @Test
    fun `an entry without a note id is left out, and a note shows once`() {
        val timeline = listOf(
            entry(1, timestamp = 10, refId = null),
            entry(2, timestamp = 20, refId = "chat:same"),
            entry(3, timestamp = 30, refId = "chat:same"),
        )
        assertEquals(listOf(2L), chatNotes(timeline, "pc").map { it.entryId })
    }

    @Test
    fun `only this phone's notes can be changed, and only while it can reach the device`() {
        assertEquals(listOf(NoteAction.Copy, NoteAction.Edit, NoteAction.Delete), noteActions(note(incoming = false), canSend = true))
        assertEquals(listOf(NoteAction.Copy, NoteAction.Delete), noteActions(note(incoming = false), canSend = false))
        assertEquals(listOf(NoteAction.Copy, NoteAction.Delete), noteActions(note(incoming = true), canSend = true))
    }

    @Test
    fun `the composer says why it can't send`() {
        assertNull(composerBlock(online = true, tooOld = false))
        assertEquals(ComposerBlock.Offline, composerBlock(online = false, tooOld = false))
        assertEquals(ComposerBlock.Offline, composerBlock(online = false, tooOld = true))
        assertEquals(ComposerBlock.TooOld, composerBlock(online = true, tooOld = true))
    }

    @Test
    fun `a note is sent without blank lines around it, and must have text within the limit`() {
        assertEquals("  two\nlines", noteText("\n\n  two\nlines \n\n"))
        assertFalse(sendable(""))
        assertFalse(sendable(" \n\t"))
        assertTrue(sendable("hi"))
        assertTrue(sendable("x".repeat(CHAT_MAX_CHARS)))
        assertFalse(sendable("x".repeat(CHAT_MAX_CHARS + 1)))
        assertEquals(1, charsOver("x".repeat(CHAT_MAX_CHARS + 1)))
        // Counted in characters, as the core counts them, not UTF-16 units.
        val emoji = "😀"
        assertTrue(sendable(emoji.repeat(CHAT_MAX_CHARS)))
        assertEquals(0, charsOver(emoji.repeat(CHAT_MAX_CHARS)))
    }

    @Test
    fun `a day label goes above the first note of each day`() {
        val utc = TimeZone.getTimeZone("UTC")
        val day = 86_400L
        val notes = listOf(
            note(incoming = true, timestamp = 10 * day + 100, id = "a"),
            note(incoming = false, timestamp = 10 * day + 5_000, id = "b"),
            note(incoming = true, timestamp = 11 * day + 1, id = "c"),
        )
        assertTrue(startsDay(notes, 0, utc))
        assertFalse(startsDay(notes, 1, utc))
        assertTrue(startsDay(notes, 2, utc))
    }
}
