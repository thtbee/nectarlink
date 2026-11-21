// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ReconnectPolicyTest {
    @Test
    fun aNetworkKeepsItsIdAndAnotherGetsItsOwn() {
        val home = ReconnectPolicy.idOf("192.168.1.0/24", "192.168.1.1", "")
        assertEquals(home, ReconnectPolicy.idOf("192.168.1.0/24", "192.168.1.1", ""))
        assertEquals(16, home.length)
        assertNotEquals(home, ReconnectPolicy.idOf("10.0.0.0/8", "10.0.0.1", "corp.example"))
        assertNotEquals(home, ReconnectPolicy.idOf("192.168.1.0/24", "192.168.1.254", ""))
    }

    @Test
    fun givingUpWaitsForTheChosenTimeAndNeverWhenOff() {
        assertFalse(ReconnectPolicy.overdue(since = 0, now = 14 * 60_000L, minutes = 15))
        assertTrue(ReconnectPolicy.overdue(since = 0, now = 15 * 60_000L, minutes = 15))
        assertFalse(ReconnectPolicy.overdue(since = 0, now = Long.MAX_VALUE / 2, minutes = 0))
        assertEquals(listOf(0, 15, 60, 240), ReconnectPolicy.GIVE_UP_CHOICES)
    }
}
