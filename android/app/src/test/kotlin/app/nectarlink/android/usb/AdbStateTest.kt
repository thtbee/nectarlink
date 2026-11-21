// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.usb

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AdbStateTest {
    @Test
    fun usbDebuggingCountsWithACableIn() {
        assertTrue(AdbState.decide(usbDebugging = true, wirelessDebugging = false, cableIn = true))
        assertFalse(AdbState.decide(usbDebugging = true, wirelessDebugging = false, cableIn = false))
    }

    @Test
    fun anUnknownCableNeverStopsARealPc() {
        assertTrue(AdbState.decide(usbDebugging = true, wirelessDebugging = false, cableIn = null))
    }

    @Test
    fun wirelessDebuggingNeedsNoCable() {
        assertTrue(AdbState.decide(usbDebugging = false, wirelessDebugging = true, cableIn = false))
    }

    @Test
    fun noDebuggingMeansNoPc() {
        assertFalse(AdbState.decide(usbDebugging = false, wirelessDebugging = false, cableIn = true))
        assertFalse(AdbState.decide(usbDebugging = false, wirelessDebugging = false, cableIn = null))
    }
}
