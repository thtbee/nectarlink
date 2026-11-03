// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PcSpeakerPlayerTest {
    @Test
    fun invalidFramesDoNotSetPlaying() {
        var currentTime = 1_000L
        val window = ActivityWindow(windowMs = 3_000L, now = { currentTime })
        val player = PcSpeakerPlayer(window)

        assertFalse(player.playing.value)

        // Empty PCM
        player.playFrame(44_100, 2, ByteArray(0))
        assertFalse(player.playing.value)

        // Invalid sample rate
        player.playFrame(0, 2, ByteArray(16))
        assertFalse(player.playing.value)

        // Invalid channels
        player.playFrame(44_100, 0, ByteArray(16))
        assertFalse(player.playing.value)
    }

    @Test
    fun validFrameSetsPlayingAndStopResets() {
        var currentTime = 1_000L
        val window = ActivityWindow(windowMs = 3_000L, now = { currentTime })
        val player = PcSpeakerPlayer(window)

        assertFalse(player.playing.value)

        player.playFrame(44_100, 2, ByteArray(16), pcId = "pc-a")
        assertTrue(player.playing.value)

        // Another PC disconnecting does not stop playback for pc-a.
        player.stopForPc("pc-b")
        assertTrue(player.playing.value)

        player.stopForPc("pc-a")
        assertFalse(player.playing.value)
    }
}
