// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the helper runs for the app: the app's own commands and nothing else. */
class InputServerCommandsTest {
    @Test
    fun theAppsOwnCommandsRun() {
        assertTrue(InputServer.isAllowedCommand("svc wifi enable"))
        assertTrue(InputServer.isAllowedCommand("hotspot on"))
        assertTrue(InputServer.isAllowedCommand("settings put system screen_brightness 128"))
        assertTrue(InputServer.isAllowedCommand("settings put system accelerometer_rotation 0"))
        assertTrue(InputServer.isAllowedCommand("settings put system user_rotation 1"))
        assertFalse(InputServer.isAllowedCommand("settings put system user_rotation 4"))
        assertFalse(InputServer.isAllowedCommand("settings put system accelerometer_rotation 1; reboot"))
        assertFalse(InputServer.isAllowedCommand("settings put system user_rotation 1 && reboot"))
        assertTrue(InputServer.isAllowedCommand(COMPILE_APP_COMMAND))
    }

    @Test
    fun aPcMayUninstallOtherAppsButNeverNectarLink() {
        assertTrue(InputServer.isAllowedCommand("pm uninstall com.example.app"))
        assertTrue(InputServer.isAllowedCommand("pm uninstall org.Some_App.v2"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall app.nectarlink"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall app.nectarlink.dev"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall sn.nectarlink.services"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall com.example.app; reboot"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall --user 0 com.example.app"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall com"))
        assertFalse(InputServer.isAllowedCommand("pm uninstall "))
    }

    @Test
    fun markingTextsReadAllowsTheWriteOnlyForTheChange() {
        val commands = InputServer.smsReadCommands(42, "ignore").map { it.toList() }
        assertEquals(
            listOf(
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "allow"),
                listOf(
                    "content", "update", "--uri", "content://sms/inbox",
                    "--bind", "read:i:1", "--bind", "seen:i:1",
                    "--where", "thread_id=42 AND read=0",
                ),
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "ignore"),
            ),
            commands,
        )
    }

    @Test
    fun markingATextUnreadChangesThatOneMessageAndPutsTheModeBack() {
        val allow = listOf("appops", "set", "com.android.shell", "WRITE_SMS", "allow")
        val back = listOf("appops", "set", "com.android.shell", "WRITE_SMS", "ignore")
        assertEquals(
            listOf(
                allow,
                listOf("content", "update", "--uri", "content://sms/42", "--bind", "read:i:0"),
                back,
            ),
            InputServer.smsUnreadCommands(false, 42, "ignore").map { it.toList() },
        )
        assertEquals(
            "content://mms/7",
            InputServer.smsUnreadCommands(true, 7, "ignore")[1][3],
        )
        assertEquals(
            listOf("content", "query", "--uri", "content://sms/42", "--projection", "read"),
            InputServer.smsUnreadCheck(false, 42).toList(),
        )
        assertEquals(0, InputServer.parseReadFlag("Row: 0 read=0\n"))
        assertEquals(1, InputServer.parseReadFlag("Row: 0 read=1\n"))
        assertNull(InputServer.parseReadFlag("No result found.\n"))
        assertNull(InputServer.parseReadFlag("Error while accessing provider:sms\n"))
    }

    @Test
    fun deletingAConversationIsScopedToItsNumberInBothStores() {
        val commands = InputServer.smsDeleteCommands(42, null, "ignore").map { it.toList() }
        assertEquals(
            listOf(
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "allow"),
                listOf("content", "delete", "--uri", "content://sms", "--where", "thread_id=42"),
                listOf("content", "delete", "--uri", "content://mms", "--where", "thread_id=42"),
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "ignore"),
            ),
            commands,
        )
        assertEquals(
            listOf(
                listOf("content", "query", "--uri", "content://sms", "--projection", "_id", "--where", "thread_id=42"),
                listOf("content", "query", "--uri", "content://mms", "--projection", "_id", "--where", "thread_id=42"),
            ),
            InputServer.smsDeleteChecks(42, null).map { it.toList() },
        )
    }

    @Test
    fun deletingMessagesNamesOnlyThoseNumbersAndOnlyTheStoresTheyAreIn() {
        val ids = InputServer.parseSmsIds("sms:5,sms:6,mms:7")!!
        assertEquals(
            listOf(
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "allow"),
                listOf("content", "delete", "--uri", "content://sms", "--where", "thread_id=42 AND _id IN (5,6)"),
                listOf("content", "delete", "--uri", "content://mms", "--where", "thread_id=42 AND _id IN (7)"),
                listOf("appops", "set", "com.android.shell", "WRITE_SMS", "default"),
            ),
            InputServer.smsDeleteCommands(42, ids, "default").map { it.toList() },
        )
        val onlyPictures = InputServer.parseSmsIds("mms:7")!!
        assertEquals(
            listOf(listOf("content", "delete", "--uri", "content://mms", "--where", "thread_id=42 AND _id IN (7)")),
            InputServer.smsDeleteCommands(42, onlyPictures, "default").map { it.toList() }.drop(1).dropLast(1),
        )
        assertEquals(
            listOf(listOf("content", "query", "--uri", "content://mms", "--projection", "_id", "--where", "thread_id=42 AND _id IN (7)")),
            InputServer.smsDeleteChecks(42, onlyPictures).map { it.toList() },
        )
    }

    @Test
    fun anythingButNumbersIsRefusedBeforeAShellSeesIt() {
        for (bad in listOf("", "5", "sms:", "sms:x", "sms:5,", "sms:5;reboot", "sms:5 OR 1=1", "sms:-1", "sms:0", "wap:5", "sms:1234567890123456789")) {
            assertNull(bad, InputServer.parseSmsIds(bad))
        }
        val unread = InputServer.parseSmsUnreadRequest("3 mms 9")!!
        assertEquals(Triple(3L, true, 9L), Triple(unread.seq, unread.mms, unread.id))
        for (bad in listOf("", "3 mms", "3 sms 9 9", "x sms 9", "3 wap 9", "3 sms 0", "3 sms 9;reboot", "3 sms -9")) {
            assertNull(bad, InputServer.parseSmsUnreadRequest(bad))
        }
        val whole = InputServer.parseSmsDeleteRequest("4 42")!!
        assertEquals(Pair(4L, 42L), Pair(whole.seq, whole.thread))
        assertNull(whole.ids)
        val some = InputServer.parseSmsDeleteRequest("4 42 sms:5,mms:6")!!
        assertEquals(listOf(5L), some.ids!!.sms)
        assertEquals(listOf(6L), some.ids!!.mms)
        // A list that doesn't parse deletes nothing, not the conversation.
        for (bad in listOf("", "4", "4 0", "4 x", "4 42 sms:5;reboot", "4 42 5", "4 42 sms:5 sms:6")) {
            assertNull(bad, InputServer.parseSmsDeleteRequest(bad))
        }
    }

    @Test
    fun noRowsAreOnlyCountedFromARealResult() {
        assertEquals(0, InputServer.parseRowCount("No result found.\n"))
        assertEquals(2, InputServer.parseRowCount("Row: 0 _id=5\nRow: 1 _id=6\n"))
        assertNull(InputServer.parseRowCount(""))
        assertNull(InputServer.parseRowCount("Error while accessing provider:sms\n"))
    }

    @Test
    fun theWriteModeIsPutBackToWhatItWas() {
        assertEquals("ignore", InputServer.parseAppOpMode("WRITE_SMS: ignore\n"))
        assertEquals("allow", InputServer.parseAppOpMode("WRITE_SMS: allow; time=+3d2h ago\n"))
        assertEquals("default", InputServer.parseAppOpMode("No operations.\n"))
        assertNull(InputServer.parseAppOpMode(""))
        assertNull(InputServer.parseAppOpMode("WRITE_SMS: reboot"))
    }

    @Test
    fun anythingElseIsRefused() {
        assertFalse(InputServer.isAllowedCommand("settings put system screen_brightness 256"))
        assertFalse(InputServer.isAllowedCommand("settings put global adb_enabled 1"))
        assertFalse(InputServer.isAllowedCommand("reboot"))
        assertFalse(InputServer.isAllowedCommand(""))
    }
}
