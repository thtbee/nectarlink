// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.deck

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

class DefaultDeckLabelTest {
    @Test
    fun `every label of the PC's default Deck has a translation`() {
        listOf(
            "Play / pause", "Previous", "Next", "Volume down", "Volume up", "Mute audio",
            "Mic mute", "Show desktop", "Switch window", "Screenshot", "Lock PC",
        ).forEach { assertNotNull(it, defaultDeckLabel(it)) }
    }

    @Test
    fun `a name the user chose stays as it is`() {
        assertEquals(null, defaultDeckLabel("Open Spotify"))
    }
}
