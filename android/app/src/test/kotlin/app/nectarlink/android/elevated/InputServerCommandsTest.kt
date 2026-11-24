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
