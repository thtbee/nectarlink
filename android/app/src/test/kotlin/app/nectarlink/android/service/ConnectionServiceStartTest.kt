// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import org.junit.Assert.assertEquals
import org.junit.Test

class ConnectionServiceStartTest {
    @Test
    fun `foreground launch uses startService to avoid arming FGS timer`() {
        assertEquals(
            ConnectionService.StartStrategy.START_SERVICE,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = true,
                usbOnlyEnabled = false,
                usbConnected = false,
                fromForeground = true,
            ),
        )
    }

    @Test
    fun `background launch uses startForegroundService`() {
        assertEquals(
            ConnectionService.StartStrategy.START_FOREGROUND_SERVICE,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = true,
                usbOnlyEnabled = false,
                usbConnected = false,
                fromForeground = false,
            ),
        )
    }

    @Test
    fun `disabled background service skips starting`() {
        assertEquals(
            ConnectionService.StartStrategy.SKIP,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = false,
                usbOnlyEnabled = false,
                usbConnected = true,
                fromForeground = true,
            ),
        )
    }

    @Test
    fun `usb only without connected pc skips starting even in foreground`() {
        assertEquals(
            ConnectionService.StartStrategy.SKIP,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = true,
                usbOnlyEnabled = true,
                usbConnected = false,
                fromForeground = true,
            ),
        )
    }

    @Test
    fun `usb only with connected pc follows foreground vs background rule`() {
        assertEquals(
            ConnectionService.StartStrategy.START_SERVICE,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = true,
                usbOnlyEnabled = true,
                usbConnected = true,
                fromForeground = true,
            ),
        )
        assertEquals(
            ConnectionService.StartStrategy.START_FOREGROUND_SERVICE,
            ConnectionService.resolveStartStrategy(
                backgroundServiceEnabled = true,
                usbOnlyEnabled = true,
                usbConnected = true,
                fromForeground = false,
            ),
        )
    }
}
