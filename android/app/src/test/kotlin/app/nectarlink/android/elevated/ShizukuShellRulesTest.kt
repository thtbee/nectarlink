// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import app.nectarlink.android.BuildConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ShizukuShellRulesTest {
    private val own = BuildConfig.APPLICATION_ID

    @Test
    fun takesOnlyTheTokensElevatedMakes() {
        assertTrue(ShizukuShellRules.isToken("0123456789abcdef".repeat(3)))
        assertFalse(ShizukuShellRules.isToken("0123456789ABCDEF".repeat(3)))
        assertFalse(ShizukuShellRules.isToken("abc"))
        assertFalse(ShizukuShellRules.isToken("0123456789abcdef".repeat(3) + " ; reboot"))
    }

    @Test
    fun runsTheGrantsElevatedAsksForItsOwnPackage() {
        listOf(
            "pm grant $own android.permission.WRITE_SECURE_SETTINGS",
            "appops set $own GET_USAGE_STATS allow",
            "appops set $own WRITE_SETTINGS allow",
            "cmd notification allow_dnd $own",
            "cmd notification allow_listener $own/app.nectarlink.android.notifications.NotificationListener",
        ).forEach { assertTrue(it, ShizukuShellRules.isGrant(it)) }
    }

    @Test
    fun refusesAnythingElse() {
        listOf(
            "pm grant com.example.other android.permission.WRITE_SECURE_SETTINGS",
            "pm grant $own android.permission.WRITE_SECURE_SETTINGS; reboot",
            "appops set $own GET_USAGE_STATS deny",
            "appops set com.example.other GET_USAGE_STATS allow",
            "cmd notification allow_listener com.example.other/$own.Listener",
            "cmd notification allow_dnd com.example.other",
            "pm uninstall $own",
            "reboot",
            "",
        ).forEach { assertFalse(it, ShizukuShellRules.isGrant(it)) }
    }

    @Test
    fun readsThePortTheHelperListensOn() {
        assertEquals(43567, ShizukuShellRules.readyPort("ready 43567"))
        assertNull(ShizukuShellRules.readyPort("can't inject input: java.lang.SecurityException"))
        assertNull(ShizukuShellRules.readyPort("ready 0"))
        assertNull(ShizukuShellRules.readyPort("ready 70000"))
        assertNull(ShizukuShellRules.readyPort(""))
    }

    @Test
    fun attachSenderUidAndBoundedReaderRejectUntrustedOrOversizedInput() {
        assertTrue(Elevated.isAllowedAttachSenderUid(android.os.Process.SHELL_UID))
        assertTrue(Elevated.isAllowedAttachSenderUid(android.os.Process.ROOT_UID))
        assertFalse(Elevated.isAllowedAttachSenderUid(10234))

        val okReader = java.io.StringReader("hello\r\nworld\n")
        with(InputServer) {
            assertEquals("hello", okReader.nextLine(16))
            assertEquals("world", okReader.nextLine(16))
            assertNull(okReader.nextLine(16))
        }

        val oversized = java.io.StringReader("a".repeat(200))
        val threw = runCatching {
            with(InputServer) { oversized.nextLine(InputServer.MAX_TOKEN_CHARS) }
        }.isFailure
        assertTrue("expected IOException on line exceeding maxChars", threw)

        assertTrue(app.nectarlink.android.service.PcAutoStart.OPEN_COMMAND.contains("$own/"))
    }
}
