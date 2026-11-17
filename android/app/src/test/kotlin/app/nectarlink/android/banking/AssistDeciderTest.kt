// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import app.nectarlink.android.banking.AssistDecider.Action
import org.junit.Assert.assertEquals
import org.junit.Test

class AssistDeciderTest {
    private fun decide(
        paused: Boolean = false,
        inUse: Boolean = false,
        elevatedSetUp: Boolean = true,
        on: Boolean = true,
        offByUs: Boolean = false,
        now: Boolean = false,
    ) = AssistDecider.decide(paused, inUse, elevatedSetUp, on, offByUs, now)

    @Test
    fun turnsOffAfterADelayWhenNoPcUsesThePhone() {
        assertEquals(Action.SCHEDULE_OFF, decide())
        assertEquals(Action.TURN_OFF, decide(now = true))
    }

    @Test
    fun comesBackWhenAPcUsesThePhone() {
        assertEquals(Action.TURN_ON, decide(inUse = true, on = false, offByUs = true))
        // Already on: stays, and a pending off is dropped.
        assertEquals(Action.NONE, decide(inUse = true, on = true))
        assertEquals(Action.NONE, decide(inUse = true, on = true, now = true))
    }

    @Test
    fun neverTurnsOnWhatTheUserTurnedOff() {
        assertEquals(Action.NONE, decide(inUse = true, on = false, offByUs = false))
        assertEquals(Action.NONE, decide(elevatedSetUp = false, on = false, offByUs = false))
        assertEquals(Action.NONE, decide(on = false, offByUs = false))
    }

    @Test
    fun staysOnWithoutElevated() {
        assertEquals(Action.TURN_ON, decide(elevatedSetUp = false, on = false, offByUs = true))
        assertEquals(Action.NONE, decide(elevatedSetUp = false, on = true, now = true))
    }

    @Test
    fun leavesItToPauseForBankingApps() {
        assertEquals(Action.NONE, decide(paused = true, now = true))
        assertEquals(Action.NONE, decide(paused = true, inUse = true, on = false, offByUs = true))
    }
}
