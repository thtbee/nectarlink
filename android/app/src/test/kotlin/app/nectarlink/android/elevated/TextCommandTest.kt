// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import java.util.Base64
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class TextCommandTest {
    @Test
    fun russianLettersAndUnicodePastesStayWholeOnTheirDisplay() {
        for (text in listOf("я", "Ё", "Привет, мир!", "🙂", "first\nвторая\r\nтретья")) {
            val command = checkNotNull(TextCommand.make(text, 7))
            assertEquals("@7 I ", command.take(5))
            assertFalse(command.contains('\n'))
            assertFalse(command.contains('\r'))
            assertEquals(text, String(Base64.getDecoder().decode(command.substring(5)), Charsets.UTF_8))
        }
    }

    @Test
    fun ordinaryKeysAndEnterKeepTheirBehavior() {
        assertEquals("T a", TextCommand.make("a", null))
        assertEquals("@3 T  ", TextCommand.make(" ", 3))
        for (newline in listOf("\n", "\r", "\r\n")) assertEquals("K 66", TextCommand.make(newline, null))
        assertNull(TextCommand.make("", 3))
    }
}
