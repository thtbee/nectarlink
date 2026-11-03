// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StartGuardTest {
    @Test
    fun multipleStartsWithoutStopAreNotCancelled() {
        val guard = StartGuard()
        val token1 = guard.beginStart()
        val token2 = guard.beginStart()

        assertFalse(guard.isCancelled(token1))
        assertFalse(guard.isCancelled(token2))
    }

    @Test
    fun stopCancelsInFlightStart() {
        val guard = StartGuard()
        val token = guard.beginStart()

        guard.stop()

        assertTrue(guard.isCancelled(token))
    }

    @Test
    fun startAfterStopIsNotCancelled() {
        val guard = StartGuard()
        val firstToken = guard.beginStart()
        val stopSeq = guard.recordStop()

        val nextToken = guard.beginStart()

        assertTrue(guard.isCancelled(firstToken))
        assertFalse(guard.isCancelled(nextToken))
        assertTrue(nextToken.startSeq > stopSeq)
    }
}
