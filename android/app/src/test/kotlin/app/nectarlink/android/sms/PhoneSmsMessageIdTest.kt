// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.sms

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PhoneSmsMessageIdTest {
    @Test
    fun aMessageIdIsTheKindAndTheNumberTheListGave() {
        assertEquals(false to 12L, PhoneSms.parseMessageId("sms:12"))
        assertEquals(true to 7L, PhoneSms.parseMessageId("mms:7"))
    }

    @Test
    fun anythingElseIsNotAMessageId() {
        for (bad in listOf("", "12", "sms:", "sms:0", "sms:-1", "sms:1 OR 1=1", "mms:7;reboot", "wap:3", "sms:1234567890123456789")) {
            assertNull(bad, PhoneSms.parseMessageId(bad))
        }
    }
}
