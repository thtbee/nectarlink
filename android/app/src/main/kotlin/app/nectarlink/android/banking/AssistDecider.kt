// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

/** What [AssistOnDemand] does with the accessibility service; kept apart from Android so it can be tested. */
internal object AssistDecider {
    enum class Action { NONE, TURN_ON, SCHEDULE_OFF, TURN_OFF }

    /**
     * What to do with the service, with this switched on. [inUse]: a PC shows
     * the phone or types on it; [on]: the service is enabled now; [offByUs]:
     * this turned it off; [now]: the off delay has passed.
     */
    fun decide(paused: Boolean, inUse: Boolean, elevatedSetUp: Boolean, on: Boolean, offByUs: Boolean, now: Boolean): Action = when {
        // Pause for banking apps turns it off and back on as it found it.
        paused -> Action.NONE
        // Without Elevated, Assist is how a PC controls the phone at all.
        inUse || !elevatedSetUp -> if (!on && offByUs) Action.TURN_ON else Action.NONE
        !on -> Action.NONE
        now -> Action.TURN_OFF
        else -> Action.SCHEDULE_OFF
    }
}
