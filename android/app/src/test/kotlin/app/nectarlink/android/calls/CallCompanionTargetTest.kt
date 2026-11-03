// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.telecom.Call
import org.junit.Assert.assertEquals
import org.junit.Test

class CallCompanionTargetTest {
    @Test
    fun waitingCallDoesNotTakeOverTheActiveOne() {
        assertEquals(0, controlledIndex(listOf(Call.STATE_ACTIVE, Call.STATE_RINGING)))
        assertEquals(0, controlledIndex(listOf(Call.STATE_HOLDING, Call.STATE_RINGING)))
        assertEquals(0, controlledIndex(listOf(Call.STATE_RINGING)))
    }

    @Test
    fun noCallToControlWhenAllAreGone() {
        assertEquals(-1, controlledIndex(emptyList()))
        assertEquals(-1, controlledIndex(listOf(Call.STATE_DISCONNECTED)))
    }

    @Test
    fun truncateUtf8RespectsByteLimitAndCodePointBoundaries() {
        val ascii = "a".repeat(80)
        assertEquals(64, PhoneCalls.truncateUtf8(ascii, PhoneCalls.MAX_NUMBER_BYTES).toByteArray(Charsets.UTF_8).size)

        // Each emoji is 4 UTF-8 bytes; 16 fit in 64 bytes, and a 65-byte limit still keeps 16 without splitting a surrogate pair.
        val emojis = "\uD83D\uDE00".repeat(20)
        val truncated = PhoneCalls.truncateUtf8(emojis, 65)
        assertEquals(64, truncated.toByteArray(Charsets.UTF_8).size)
        assertEquals("\uD83D\uDE00".repeat(16), truncated)
    }
}
