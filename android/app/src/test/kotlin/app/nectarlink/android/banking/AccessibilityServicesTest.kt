// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AccessibilityServicesTest {
    private val long = "app.nectarlink/app.nectarlink.android.mirror.InputService"
    private val short = "app.nectarlink/.android.mirror.InputService"
    private val launcher = "com.microsoft.launcher/.LauncherAccessibilityService"

    @Test
    fun `finds the service written either way`() {
        assertTrue(AccessibilityServices.contains(long, long))
        assertTrue(AccessibilityServices.contains(short, long))
        assertTrue(AccessibilityServices.contains("$launcher:$short", long))
    }

    @Test
    fun `does not find what isn't there`() {
        assertFalse(AccessibilityServices.contains(null, long))
        assertFalse(AccessibilityServices.contains("", long))
        assertFalse(AccessibilityServices.contains(launcher, long))
        assertFalse(AccessibilityServices.contains("app.nectarlink/.other.InputService", long))
        assertFalse(AccessibilityServices.contains("app.nectarlink.dev/.android.mirror.InputService", long))
    }

    @Test
    fun `takes the service out however it was written and keeps the others`() {
        assertEquals(listOf(launcher), AccessibilityServices.without("$short:$launcher", long))
        assertEquals(listOf(launcher), AccessibilityServices.without("$launcher:$long", long))
        assertEquals(emptyList<String>(), AccessibilityServices.without(short, long))
    }

    @Test
    fun `leaves another app's short name as it was written`() {
        assertEquals(launcher, AccessibilityServices.join(AccessibilityServices.without(launcher, long)))
    }

    @Test
    fun `a stray separator doesn't leave blank entries`() {
        assertEquals(listOf(launcher), AccessibilityServices.parse(":$launcher:"))
        assertEquals(emptyList<String>(), AccessibilityServices.parse(null))
    }
}
