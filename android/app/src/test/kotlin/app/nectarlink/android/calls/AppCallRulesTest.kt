// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import app.nectarlink.core.CallPhase
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppCallRulesTest {
    private val own = "app.nectarlink"
    private val dialers = setOf("com.google.android.dialer")

    @Test
    fun `another app's call notification is an app call`() {
        assertTrue(AppCallRules.isAppCall("com.whatsapp", "call", 0, true, own, dialers))
        // CallStyle without the category still counts.
        assertTrue(AppCallRules.isAppCall("org.thoughtcrime.securesms", null, AppCallRules.CALL_TYPE_INCOMING, false, own, dialers))
        // A missed-call notice filed under "call": neither ongoing nor full-screen.
        assertFalse(AppCallRules.isAppCall("com.whatsapp", "call", 0, false, own, dialers))
    }

    @Test
    fun `the phone's own calls and other notifications are not`() {
        assertFalse(AppCallRules.isAppCall("com.google.android.dialer", "call", 1, true, own, dialers))
        assertFalse(AppCallRules.isAppCall("com.android.server.telecom", "call", 0, true, own, dialers))
        assertFalse(AppCallRules.isAppCall("com.samsung.android.incallui", "call", 0, true, own, dialers))
        assertFalse(AppCallRules.isAppCall(own, "call", 1, true, own, dialers))
        assertFalse(AppCallRules.isAppCall("com.whatsapp", "msg", 0, true, own, dialers))
    }

    @Test
    fun `ringing or answered`() {
        assertEquals(CallPhase.RINGING, AppCallRules.phase(AppCallRules.CALL_TYPE_INCOMING, showsTimer = false))
        assertEquals(CallPhase.RINGING, AppCallRules.phase(AppCallRules.CALL_TYPE_SCREENING, showsTimer = false))
        assertEquals(CallPhase.ACTIVE, AppCallRules.phase(AppCallRules.CALL_TYPE_ONGOING, showsTimer = false))
        // Without CallStyle: answered once a timer runs.
        assertEquals(CallPhase.RINGING, AppCallRules.phase(0, showsTimer = false))
        assertEquals(CallPhase.ACTIVE, AppCallRules.phase(0, showsTimer = true))
    }

    @Test
    fun `buttons are found by their titles`() {
        val titles = listOf("Decline", "Answer")
        assertEquals(1, AppCallRules.answerIndex(titles))
        assertEquals(0, AppCallRules.endIndex(titles, ringing = true))
        // In another language, or answer first.
        val spanish = listOf("Contestar", "Rechazar")
        assertEquals(0, AppCallRules.answerIndex(spanish))
        assertEquals(1, AppCallRules.endIndex(spanish, ringing = true))
        assertEquals(1, AppCallRules.endIndex(listOf("Mute", "Hang up"), ringing = false))
        // Whole words: "Send" isn't "end".
        assertNull(AppCallRules.endIndex(listOf("Send", "Speaker"), ringing = false))
        assertEquals(1, AppCallRules.endIndex(listOf("静音", "挂断"), ringing = false))
    }

    @Test
    fun `unknown titles fall back to where the buttons usually are`() {
        val titles = listOf("✕", "✓")
        assertEquals(1, AppCallRules.answerIndex(titles))
        assertEquals(0, AppCallRules.endIndex(titles, ringing = true))
        // An answered call's only button ends it; with more, nothing's guessed.
        assertEquals(0, AppCallRules.endIndex(listOf("◼"), ringing = false))
        assertNull(AppCallRules.endIndex(listOf("A", "B"), ringing = false))
        assertNull(AppCallRules.answerIndex(listOf("Open")))
    }
}
