// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import app.nectarlink.android.banking.PauseDecider.Action
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PauseDeciderTest {
    private val bank = "com.uco.bank"
    private val upi = "com.upi.pay"
    private val apps = setOf(bank, upi)

    @Test
    fun pausesAsSoonAsABankingAppIsInFront() {
        val d = PauseDecider()
        assertNull(d.decide("com.launcher", apps, paused = false, nowMs = 0))
        assertEquals(Action.PAUSE, d.decide(bank, apps, paused = false, nowMs = 10))
        // Already paused: nothing more to do while it stays in front.
        assertNull(d.decide(bank, apps, paused = true, nowMs = 1_000))
    }

    @Test
    fun turnsBackOnOnlyAfterAnotherAppStaysInFront() {
        val d = PauseDecider(resumeAfterMs = 3_000)
        assertNull(d.decide("com.launcher", apps, paused = true, nowMs = 0))
        assertNull(d.decide("com.launcher", apps, paused = true, nowMs = 2_999))
        assertEquals(Action.RESUME, d.decide("com.launcher", apps, paused = true, nowMs = 3_000))
    }

    @Test
    fun aQuickHopToAnotherAppAndBackKeepsThePause() {
        val d = PauseDecider(resumeAfterMs = 3_000)
        assertNull(d.decide("com.camera.qr", apps, paused = true, nowMs = 0))
        // Back in the banking app before the wait is over.
        assertNull(d.decide(bank, apps, paused = true, nowMs = 1_500))
        // Leaving again starts the wait over.
        assertNull(d.decide("com.camera.qr", apps, paused = true, nowMs = 2_000))
        assertNull(d.decide("com.camera.qr", apps, paused = true, nowMs = 4_500))
        assertEquals(Action.RESUME, d.decide("com.camera.qr", apps, paused = true, nowMs = 5_000))
    }

    @Test
    fun movingBetweenChosenAppsKeepsThePause() {
        val d = PauseDecider()
        assertEquals(Action.PAUSE, d.decide(bank, apps, paused = false, nowMs = 0))
        assertNull(d.decide(upi, apps, paused = true, nowMs = 10_000))
    }

    @Test
    fun systemScreensOverABankingAppChangeNothing() {
        val d = PauseDecider(resumeAfterMs = 3_000)
        assertNull(d.decide("com.google.android.permissioncontroller", apps, paused = true, nowMs = 0))
        assertNull(d.decide("com.android.systemui", apps, paused = true, nowMs = 60_000))
        assertNull(d.decide(null, apps, paused = true, nowMs = 120_000))
    }

    @Test
    fun nothingHappensWhenNotPausedAndNoBankingAppIsInFront() {
        val d = PauseDecider()
        assertNull(d.decide("com.launcher", apps, paused = false, nowMs = 0))
        assertNull(d.decide("com.launcher", emptySet(), paused = false, nowMs = 60_000))
    }

    @Test
    fun lockedOrScreenOffPackageResumesAfterCountdown() {
        val d = PauseDecider(resumeAfterMs = 3_000)
        assertNull(d.decide(PauseDecider.LOCKED_PACKAGE, apps, paused = true, nowMs = 0))
        assertNull(d.decide(PauseDecider.LOCKED_PACKAGE, apps, paused = true, nowMs = 2_999))
        assertEquals(Action.RESUME, d.decide(PauseDecider.LOCKED_PACKAGE, apps, paused = true, nowMs = 3_000))
    }
}
