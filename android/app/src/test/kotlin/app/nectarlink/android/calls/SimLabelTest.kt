// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import org.junit.Assert.assertEquals
import org.junit.Test

class SimLabelTest {
    private fun label(slot: Int?, name: String?, carrier: String?) =
        PhoneCalls.simLabelOf(slot, name, carrier, "fallback")

    @Test
    fun a_named_sim_shows_its_name() {
        assertEquals("SIM 1 · Orange", label(1, "Orange", "Orange Mali"))
    }

    @Test
    fun a_default_name_gives_way_to_the_carrier() {
        assertEquals("SIM 1 · Airtel", label(1, "SIM 1", "Airtel"))
        assertEquals("SIM 2 · Jio", label(2, "sim2", "Jio"))
    }

    @Test
    fun the_slot_is_never_said_twice() {
        assertEquals("SIM 1", label(1, "SIM 1", null))
        assertEquals("SIM 2", label(2, "SIM 2", "SIM 2"))
        assertEquals("SIM 1", label(1, "  ", ""))
    }

    @Test
    fun without_a_slot_the_name_or_the_fallback() {
        assertEquals("Google Voice", label(null, null, "Google Voice"))
        assertEquals("fallback", label(null, null, null))
    }
}
