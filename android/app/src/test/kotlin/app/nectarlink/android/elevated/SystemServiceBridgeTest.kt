// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import app.nectarlink.services.ServiceStatus
import app.nectarlink.services.client.NectarlinkServiceContract
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SystemServiceBridgeTest {
    @Test
    fun parsesAllSupportedElevatedCommands() {
        val own = "app.nectarlink"
        assertEquals(
            SystemServiceCommand.ExpandNotifications,
            SystemServiceBridge.parseCommand("cmd statusbar expand-notifications", own),
        )
        assertEquals(SystemServiceCommand.SetWifi(true), SystemServiceBridge.parseCommand("svc wifi enable", own))
        assertEquals(SystemServiceCommand.SetWifi(false), SystemServiceBridge.parseCommand("svc wifi disable", own))
        assertEquals(SystemServiceCommand.SetBluetooth(true), SystemServiceBridge.parseCommand("svc bluetooth enable", own))
        assertEquals(SystemServiceCommand.SetBluetooth(false), SystemServiceBridge.parseCommand("svc bluetooth disable", own))
        assertEquals(SystemServiceCommand.SetHotspot(true), SystemServiceBridge.parseCommand("hotspot on", own))
        assertEquals(SystemServiceCommand.SetHotspot(false), SystemServiceBridge.parseCommand("hotspot off", own))
        assertEquals(SystemServiceCommand.SetHotspot(true), SystemServiceBridge.parseCommand("cmd wifi start-softap", own))
        assertEquals(SystemServiceCommand.SetHotspot(false), SystemServiceBridge.parseCommand("cmd wifi stop-softap", own))
        assertEquals(
            SystemServiceCommand.ProvisionPermissions,
            SystemServiceBridge.parseCommand("appops set app.nectarlink GET_USAGE_STATS allow", own),
        )
        assertEquals(
            SystemServiceCommand.LaunchPcStartActivity,
            SystemServiceBridge.parseCommand("am start -n app.nectarlink/app.nectarlink.android.service.PcStartActivity", own),
        )
        assertEquals(
            SystemServiceCommand.SetBrightness(128),
            SystemServiceBridge.parseCommand("settings put system screen_brightness 128", own),
        )
        assertEquals(
            SystemServiceCommand.UninstallPackage("com.example.app"),
            SystemServiceBridge.parseCommand("pm uninstall com.example.app", own),
        )
    }

    @Test
    fun rejectsDisallowedCommandsAndOwnPackageUninstall() {
        val own = "app.nectarlink"
        assertNull(SystemServiceBridge.parseCommand("pm uninstall app.nectarlink", own))
        assertNull(SystemServiceBridge.parseCommand("pm uninstall app.nectarlink.dev", own))
        assertNull(SystemServiceBridge.parseCommand("pm uninstall com.example; rm -rf /", own))
        assertNull(SystemServiceBridge.parseCommand("settings put system screen_brightness 300", own))
        assertNull(SystemServiceBridge.parseCommand("settings put system screen_brightness -1", own))
        assertNull(SystemServiceBridge.parseCommand("rm -rf /", own))
    }

    @Test
    fun serviceStatusCapabilityBitmaskMatchesContractFlags() {
        val status = ServiceStatus(
            apiVersion = NectarlinkServiceContract.API_VERSION,
            serviceEnabled = true,
            callerAuthorized = true,
            privilegedApp = true,
            capabilities = NectarlinkServiceContract.CAP_INPUT_INJECTION or
                NectarlinkServiceContract.CAP_SCREEN_MIRROR or
                NectarlinkServiceContract.CAP_CLIPBOARD_SYNC,
            callAudioSupported = false,
            uhidAvailable = true,
            virtualDeviceSupported = true,
        )
        assertTrue(status.hasCapability(NectarlinkServiceContract.CAP_INPUT_INJECTION))
        assertTrue(status.hasCapability(NectarlinkServiceContract.CAP_SCREEN_MIRROR))
        assertTrue(status.hasCapability(NectarlinkServiceContract.CAP_CLIPBOARD_SYNC))
        assertFalse(status.hasCapability(NectarlinkServiceContract.CAP_CALL_AUDIO))
        assertFalse(status.hasCapability(NectarlinkServiceContract.CAP_SYSTEM_TOGGLES))
    }
}
