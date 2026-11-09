// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import io.github.muntashirakon.adb.AdbPairingRequiredException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Wireless debugging that mDNS can't find is looked for on 127.0.0.1 (#147). */
class LoopbackFallbackTest {
    @Test
    fun foundByMdnsSkipsTheLoopback() = runBlocking {
        var triedLoopback = false
        val connected = LoopbackAdb.foundOrLoopback(found = { true }, loopback = { triedLoopback = true; true })
        assertTrue(connected)
        assertFalse(triedLoopback)
    }

    @Test
    fun notFoundTriesTheLoopback() = runBlocking {
        assertTrue(LoopbackAdb.foundOrLoopback(found = { false }, loopback = { true }))
        assertFalse(LoopbackAdb.foundOrLoopback(found = { false }, loopback = { false }))
    }

    @Test
    fun aFailedLookupTriesTheLoopbackAndSaysWhy() = runBlocking {
        val missed = mutableListOf<Exception>()
        val connected = LoopbackAdb.foundOrLoopback(
            found = { throw java.io.IOException("timed out") },
            loopback = { true },
            onMissed = { missed += it },
        )
        assertTrue(connected)
        assertEquals("timed out", missed.single().message)
    }

    @Test(expected = AdbPairingRequiredException::class)
    fun aForgottenPairingIsNotHiddenByTheLoopback() {
        runBlocking {
            LoopbackAdb.foundOrLoopback(
                found = { throw AdbPairingRequiredException("pair again") },
                loopback = { true },
            )
        }
    }
}
