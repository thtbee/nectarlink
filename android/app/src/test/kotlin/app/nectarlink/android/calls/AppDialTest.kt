// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import org.junit.Assert.assertFalse
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
        assertFalse(same("@s.whatsapp.net", "98765 43210"))
    }

    @Test
    fun the_apps_have_their_own_ids() {
        val ids = AppDial.APPS.map { AppDial.PREFIX + it.pkg }
        assertTrue(ids.all { it.length <= 64 })
        assertTrue(ids.toSet().size == ids.size)
    }
}
