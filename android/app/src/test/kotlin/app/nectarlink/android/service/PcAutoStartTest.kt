// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PcAutoStartTest {
    private val request = PcAutoStart.Pending(PcAutoStart.Kind.WEBCAM, "pc", at = 100_000L)

    @Test
    fun `a request that just came in is taken once`() {
        PcAutoStart.offer(request)
        assertEquals(request, PcAutoStart.take(now = 105_000L))
        assertNull(PcAutoStart.take(now = 105_000L))
    }

    @Test
    fun `a stale request starts nothing`() {
        PcAutoStart.offer(request)
        assertNull(PcAutoStart.take(now = 116_000L))
        // Nothing waits after it either.
        assertNull(PcAutoStart.take(now = 105_000L))
    }

    @Test
    fun `nothing is taken without a request`() {
        assertNull(PcAutoStart.take(now = 0L))
    }
}
