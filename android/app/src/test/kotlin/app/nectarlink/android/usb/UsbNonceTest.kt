// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.usb

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UsbNonceTest {
    @Test
    fun aPcsNonceIsPassedOnAsLowercaseHex() {
        assertEquals("00112233445566778899aabbccddeeff", UsbNonce.parse("00112233445566778899aabbccddeeff"))
        assertEquals("00112233445566778899aabbccddeeff", UsbNonce.parse(" 00112233445566778899AABBCCDDEEFF\n"))
    }

    @Test
    fun anythingElseIsDropped() {
        assertNull(UsbNonce.parse(null))
        assertNull(UsbNonce.parse(""))
        assertNull(UsbNonce.parse("0011"))
        assertNull(UsbNonce.parse("00112233445566778899aabbccddeeff00"))
        assertNull(UsbNonce.parse("00112233445566778899aabbccddeefg"))
        assertNull(UsbNonce.parse("+0112233445566778899aabbccddeeff"))
    }
}
