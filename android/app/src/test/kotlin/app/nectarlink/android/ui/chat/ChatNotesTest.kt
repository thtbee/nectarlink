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

    private fun file(
        id: Long,
        timestamp: Long,
        kind: TimelineKind = TimelineKind.FILE,
        device: String = "pc",
        incoming: Boolean = true,
        title: String = "file$id.pdf",
        target: String = "/storage/emulated/0/Download/file$id.pdf",
    ) = entry(id, timestamp, refId = "t$id", kind = kind, device = device, incoming = incoming, detail = "")
        .copy(title = title, target = target, sizeBytes = 1_000uL * id.toULong())

    @Test
    fun `files photos and recordings sit among the notes in time order, on the sender's side`() {
        val timeline = listOf(
            file(6, timestamp = 250, kind = TimelineKind.RECORDING, incoming = false, title = "Rec.m4a"),
            entry(5, timestamp = 300),
            file(4, timestamp = 150, kind = TimelineKind.PHOTO, title = "IMG_1.jpg"),
            entry(3, timestamp = 200, kind = TimelineKind.CLIP, refId = "clip"),
            entry(2, timestamp = 100, incoming = false),
            file(1, timestamp = 100, device = "tablet"),
            file(7, timestamp = 50, kind = TimelineKind.LINK),
            file(8, timestamp = 50, kind = TimelineKind.SESSION),
        )
        val items = chatItems(timeline, "pc")
        assertEquals(listOf(2L, 4L, 6L, 5L), items.map { it.entryId })
        val photo = items[1] as ChatAttachment
        assertEquals(TimelineKind.PHOTO, photo.kind)
        assertEquals("IMG_1.jpg", photo.name)
        assertEquals(4_000L, photo.sizeBytes)
        assertTrue(photo.incoming)
        val recording = items[2] as ChatAttachment
        assertFalse(recording.incoming)
        assertTrue(items[0] is ChatNote)
        assertTrue(items[3] is ChatNote)
    }

    @Test
    fun `at the same second the older entry comes first, and each entry shows once`() {
        val timeline = listOf(
            file(3, timestamp = 100),
            entry(2, timestamp = 100),
            file(3, timestamp = 100),
            entry(1, timestamp = 100, refId = "chat:n2"),
        )
        val items = chatItems(timeline, "pc")
        assertEquals(listOf(2L, 3L), items.map { it.entryId })
        assertEquals(listOf("note_n2", "file_3"), items.map { it.key })
    }

    @Test
    fun `the newest entries of several reads, and whether there are more`() {
        val notes = listOf(entry(5, timestamp = 500), entry(2, timestamp = 200), entry(1, timestamp = 100))
        val files = listOf(file(4, timestamp = 400), file(3, timestamp = 300))
        val (top, more) = newestOf(listOf(notes to false, files to false), limit = 3)
        assertEquals(listOf(5L, 4L, 3L), top.map { it.id })
        assertTrue(more)

        val (all, noMore) = newestOf(listOf(notes to false, files to false, emptyList<TimelineEntry>() to false), limit = 10)
        assertEquals(listOf(5L, 4L, 3L, 2L, 1L), all.map { it.id })
        assertFalse(noMore)

        // A read that has more means there's more, even when everything fits.
        assertTrue(newestOf(listOf(notes to true), limit = 10).second)
        // The same entry read twice counts once.
        val (once, onceMore) = newestOf(listOf(notes to false, notes to false), limit = 3)
        assertEquals(listOf(5L, 2L, 1L), once.map { it.id })
        assertFalse(onceMore)
    }

    @Test
    fun `an attachment opens and shares only while its file is here, and can always be removed`() {
        assertEquals(
            listOf(AttachmentAction.Open, AttachmentAction.Share, AttachmentAction.Remove),
            attachmentActions(onPhone = true),
        )
        assertEquals(listOf(AttachmentAction.Remove), attachmentActions(onPhone = false))
    }

    @Test
    fun `an attachment about several files has no single target`() {
        val one = chatItems(listOf(file(1, timestamp = 1, target = "/a/b.jpg")), "pc").single() as ChatAttachment
        assertEquals("/a/b.jpg", one.singleTarget)
        val many = chatItems(listOf(file(2, timestamp = 1, target = "/a/b.jpg\n/a/c.jpg")), "pc").single() as ChatAttachment
        assertNull(many.singleTarget)
        val none = chatItems(listOf(file(3, timestamp = 1, target = "")), "pc").single() as ChatAttachment
        assertNull(none.singleTarget)
    }

    @Test
    fun `an attachment looks like what its file is`() {
        fun look(kind: TimelineKind, title: String, target: String = "") =
            attachmentLook(chatItems(listOf(file(1, timestamp = 1, kind = kind, title = title, target = target)), "pc").single() as ChatAttachment)
        assertEquals(AttachmentLook.Image, look(TimelineKind.FILE, "Cat.JPG"))
        assertEquals(AttachmentLook.Video, look(TimelineKind.FILE, "clip.mp4"))
        assertEquals(AttachmentLook.Audio, look(TimelineKind.FILE, "song.mp3"))
        assertEquals(AttachmentLook.File, look(TimelineKind.FILE, "report.pdf"))
        assertEquals(AttachmentLook.Image, look(TimelineKind.PHOTO, "Photo"))
        assertEquals(AttachmentLook.Audio, look(TimelineKind.RECORDING, "Recording"))
        // The file on this phone tells more than the title.
        assertEquals(AttachmentLook.Image, look(TimelineKind.FILE, "a.jpg and 2 more", target = "/x/a.png"))
        assertEquals(AttachmentLook.Image, look(TimelineKind.FILE, "a.png", target = "content://media/external/images/media/12"))
    }

    @Test
    fun `a day label goes above the first item of each day, notes and files alike`() {
        val utc = TimeZone.getTimeZone("UTC")
        val day = 86_400L
        val items = chatItems(
            listOf(
                entry(1, timestamp = 10 * day + 100),
                file(2, timestamp = 11 * day + 5),
                entry(3, timestamp = 11 * day + 50),
            ),
            "pc",
        )
        assertTrue(startsDay(items, 0, utc))
        assertTrue(startsDay(items, 1, utc))
        assertFalse(startsDay(items, 2, utc))
    }
}
