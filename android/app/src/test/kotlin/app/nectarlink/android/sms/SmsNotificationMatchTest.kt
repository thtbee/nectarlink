// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.sms

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SmsNotificationMatchTest {
    private val people = listOf("Martin Guillon", "+1 555-0100")

    @Test
    fun aTitleThatIsANameOrANumberOfThePeopleMatches() {
        assertTrue(SmsNotificationMatch.titleMatches("martin guillon", people))
        assertTrue(SmsNotificationMatch.titleMatches("+1 555-0100", people))
        // The number as the phone's notification writes it.
        assertTrue(SmsNotificationMatch.titleMatches("(555) 0100", people))
        assertTrue(SmsNotificationMatch.titleMatches("5550100", people))
    }

    @Test
    fun otherTitlesDont() {
        assertFalse(SmsNotificationMatch.titleMatches("", people))
        assertFalse(SmsNotificationMatch.titleMatches("Alice", people))
        assertFalse(SmsNotificationMatch.titleMatches("+1 555-0199", people))
        // Too few digits to tell numbers apart.
        assertFalse(SmsNotificationMatch.titleMatches("0100", people))
        // Words with digits in them are not numbers.
        assertFalse(SmsNotificationMatch.titleMatches("Order 5550100", people))
    }

    @Test
    fun theMarkAsReadButtonIsFoundByItsKindOrItsLabel() {
        val semantic = 2
        assertTrue(SmsNotificationMatch.marksRead(2, "Whatever", semantic))
        assertTrue(SmsNotificationMatch.marksRead(0, " Mark as read ", semantic))
        assertFalse(SmsNotificationMatch.marksRead(0, "Reply via SMS", semantic))
        assertFalse(SmsNotificationMatch.marksRead(0, null, semantic))
    }
}
