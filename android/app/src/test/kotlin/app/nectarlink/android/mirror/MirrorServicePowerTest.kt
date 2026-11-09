// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MirrorServicePowerTest {
    @Test
    fun `the phone sound alone keeps the phone on the network`() {
        assertTrue(MirrorService.streaming(screen = false, elevatedScreen = false, sound = true))
    }

    @Test
    fun `a screen stream keeps the phone on the network with or without sound`() {
        assertTrue(MirrorService.streaming(screen = true, elevatedScreen = false, sound = false))
        assertTrue(MirrorService.streaming(screen = false, elevatedScreen = true, sound = false))
        assertTrue(MirrorService.streaming(screen = true, elevatedScreen = false, sound = true))
    }

    @Test
    fun `nothing streaming lets the phone rest`() {
        assertFalse(MirrorService.streaming(screen = false, elevatedScreen = false, sound = false))
    }
}
