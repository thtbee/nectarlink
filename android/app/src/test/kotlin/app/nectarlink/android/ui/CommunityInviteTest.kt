// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CommunityInviteTest {
    private val month = CommunityInvite.EVERY_MS

    private fun due(done: Boolean = false, armed: Boolean = false, count: Int = 0, lastAt: Long = 0, now: Long = month) =
        CommunityInvite.due(done, armed, count, lastAt, now)

    @Test
    fun the_first_update_brings_it() = assertTrue(due())

    @Test
    fun at_most_once_a_month() {
        assertFalse(due(count = 1, lastAt = month, now = month + 1000))
        assertTrue(due(count = 1, lastAt = month, now = 2 * month))
    }

    @Test
    fun not_after_joining_or_three_answers_or_while_shown() {
        assertFalse(due(done = true))
        assertFalse(due(count = CommunityInvite.TIMES))
        assertFalse(due(armed = true))
    }
}
