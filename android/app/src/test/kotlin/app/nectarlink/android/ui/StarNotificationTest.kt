// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class StarNotificationTest {
    private val installed = 1_000_000L
    private val hour = 60 * 60 * 1000L

    @Test
    fun `a fresh install is asked an hour after installing`() {
        assertEquals(installed + hour, StarNotification.askAt(installed, installed, installed + 5))
    }

    @Test
    fun `a fresh install opened much later is asked at that same time, already past`() {
        val now = installed + 5 * hour
        assertEquals(installed + hour, StarNotification.askAt(installed, installed, now))
    }

    @Test
    fun `someone who updated is asked right away`() {
        val now = installed + 1000 * hour
        assertEquals(now, StarNotification.askAt(installed, installed + 999 * hour, now))
    }
}
