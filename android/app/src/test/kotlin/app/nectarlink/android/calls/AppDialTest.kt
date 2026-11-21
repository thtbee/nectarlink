// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppDialTest {
    private fun same(row: String, number: String) = AppDial.sameNumber(row, AppDial.digits(number))

    @Test
    fun a_local_number_matches_its_international_row() {
        assertTrue(same("919876543210@s.whatsapp.net", "098765 43210"))
        assertTrue(same("919876543210@s.whatsapp.net", "+91 98765-43210"))
        assertTrue(same("4915112345678@s.whatsapp.net", "0151 1234 5678"))
    }

    @Test
    fun another_number_does_not() {
        assertFalse(same("919876543210@s.whatsapp.net", "98765 43211"))
        assertFalse(same("919876543210@s.whatsapp.net", "12345"))
        assertFalse("a short local number is not enough", same("4915555512345@s.whatsapp.net", "555 1234"))
        assertFalse(same("@s.whatsapp.net", "98765 43210"))
    }

    @Test
    fun the_exact_number_wins_and_two_possible_people_call_nobody() {
        val india = AppDial.Candidate(1, "919876543210", "Asha")
        val us = AppDial.Candidate(2, "19876543210", "Sam")
        assertEquals(1L, AppDial.pick(listOf(us, india), "919876543210")?.id)
        assertNull("the same national number in two countries", AppDial.pick(listOf(us, india), "9876543210"))
        assertEquals(2L, AppDial.pick(listOf(us), "9876543210")?.id)
        val twice = AppDial.Candidate(3, "919876543210", "Asha (work)")
        assertEquals("one number in two rows", 1L, AppDial.pick(listOf(india, twice), "09876543210")?.id)
    }

    @Test
    fun the_apps_have_their_own_ids() {
        val ids = AppDial.APPS.map { AppDial.PREFIX + it.pkg }
        assertTrue(ids.all { it.length <= 64 })
        assertTrue(ids.toSet().size == ids.size)
    }
}
