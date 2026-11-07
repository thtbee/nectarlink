// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Test

class ElevatedMethodTest {
    @Test
    fun `only what the phone has is offered`() {
        assertEquals(
            listOf(ElevatedMethod.WirelessDebugging),
            ElevatedMethod.available(shizukuInstalled = false, rooted = false),
        )
        assertEquals(
            listOf(ElevatedMethod.WirelessDebugging, ElevatedMethod.Shizuku),
            ElevatedMethod.available(shizukuInstalled = true, rooted = false),
        )
        assertEquals(
            listOf(ElevatedMethod.WirelessDebugging, ElevatedMethod.Shizuku, ElevatedMethod.Root),
            ElevatedMethod.available(shizukuInstalled = true, rooted = true),
        )
    }
}
