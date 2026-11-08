// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RootManagersTest {
    @Test
    fun kernelSuAndAPatchManagersAreKnown() {
        val packages = RootManagers.PACKAGES
        for (name in listOf("me.weishu.kernelsu", "com.rifsxd.ksunext", "me.bmax.apatch")) {
            assertTrue(name, name in packages)
        }
        assertEquals(packages.size, packages.toSet().size)
    }

    @Test
    fun aRootManagerOffersRootAsAWayToStart() {
        assertEquals(
            listOf(ElevatedMethod.WirelessDebugging, ElevatedMethod.Root),
            ElevatedMethod.available(shizukuInstalled = false, rooted = true),
        )
    }
}
