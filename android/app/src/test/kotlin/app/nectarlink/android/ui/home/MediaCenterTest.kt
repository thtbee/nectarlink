// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import app.nectarlink.android.media.PcMedia
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MediaPlayer
import org.junit.Assert.assertEquals
import org.junit.Test

class MediaCenterTest {
    private fun player(id: String, playing: Boolean) = MediaPlayer(
        id = id,
        app = "App",
        title = id,
        artist = null,
        album = null,
        playing = playing,
        duration = null,
        position = null,
        actions = listOf(MediaAction.PLAY, MediaAction.PAUSE),
        artKey = null,
        art = null,
    )

    @Test
    fun playingPlayersComeFirstFromThePcsAndThePhone() {
        val pc = listOf(
            PcMedia.Player("pc", "Desk", player("paused-pc", false), null, 0),
            PcMedia.Player("pc", "Desk", player("playing-pc", true), null, 0),
        )
        val phone = PhoneMedia.Players(listOf(player("playing-phone", true), player("paused-phone", false)), 0)
        val entries = mediaEntries(pc, phone, mapOf("pc" to null))
        assertEquals(
            listOf("playing-pc", "playing-phone", "paused-pc", "paused-phone"),
            entries.map { it.player.id },
        )
        assertEquals(listOf("pc/pc/playing-pc", "phone/playing-phone"), entries.take(2).map { it.key })
    }

    @Test
    fun nothingPlayingIsNothingToShow() {
        assertEquals(emptyList<MediaEntry>(), mediaEntries(emptyList(), PhoneMedia.Players(emptyList(), 0), emptyMap()))
    }
}
