// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services.client

import app.nectarlink.services.ServiceStatus
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NectarlinkServiceClientTest {
    @Test
    fun pinnedServiceCertificatesAreValidSha256Digests() {
        assertTrue(NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256.isNotEmpty())
        for (digestHex in NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256) {
            val bytes = NectarlinkServiceClient.decodeSha256Hex(digestHex)
            assertNotNull("Expected valid 32-byte SHA-256 hex digest for $digestHex", bytes)
            assertEquals(32, bytes!!.size)
        }
    }

    @Test
    fun decodeSha256HexNormalizesCaseAndColonsAndRejectsInvalid() {
        val raw = "E055B7E6A622168DA36E7B9F730369C8F140E5BA87DF1E16F31FD754434EEA0D"
        val colons = "e0:55:b7:e6:a6:22:16:8d:a3:6e:7b:9f:73:03:69:c8:f1:40:e5:ba:87:df:1e:16:f3:1f:d7:54:43:4e:ea:0d"
        val decodedRaw = NectarlinkServiceClient.decodeSha256Hex(raw)
        val decodedColons = NectarlinkServiceClient.decodeSha256Hex(colons)
        assertNotNull(decodedRaw)
        assertArrayEquals(decodedRaw, decodedColons)
        assertNull(NectarlinkServiceClient.decodeSha256Hex(""))
        assertNull(NectarlinkServiceClient.decodeSha256Hex("abcd"))
        assertNull(NectarlinkServiceClient.decodeSha256Hex("g".repeat(64)))
    }

    @Test
    fun matchesAnyDigestRequiresExactMatch() {
        val allowed = NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256.mapNotNull(NectarlinkServiceClient::decodeSha256Hex)
        val matching = NectarlinkServiceClient.decodeSha256Hex("e055b7e6a622168da36e7b9f730369c8f140e5ba87df1e16f31fd754434eea0d")!!
        val bogus = ByteArray(32) { 0x42 }
        assertTrue(NectarlinkServiceClient.matchesAnyDigest(matching, allowed))
        assertFalse(NectarlinkServiceClient.matchesAnyDigest(bogus, allowed))
        assertFalse(NectarlinkServiceClient.matchesAnyDigest(matching, emptyList()))
    }

    @Test
    fun uninstallPackageLimitsRejectNectarlinkAndMalformedPackages() {
        assertTrue(NectarlinkServiceContract.isAllowedUninstallPackage("com.example.app", "app.nectarlink"))
        assertTrue(NectarlinkServiceContract.isAllowedUninstallPackage("org.Some_App.v2", "app.nectarlink"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("app.nectarlink", "app.nectarlink"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("app.nectarlink.dev", "app.nectarlink"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("app.nectarlink.services", "app.nectarlink"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("com.custom.fork", "com.custom.fork"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("com.example.app; reboot"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("--user 0 com.example.app"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage("com"))
        assertFalse(NectarlinkServiceContract.isAllowedUninstallPackage(""))
    }

    @Test
    fun backgroundLaunchAndProvisioningRestrictPackageAndActivity() {
        val allowed = setOf("app.nectarlink", "app.nectarlink.dev")
        val pcStart = NectarlinkServiceContract.ALLOWED_BACKGROUND_ACTIVITY
        assertTrue(NectarlinkServiceContract.isAllowedBackgroundLaunch("app.nectarlink", pcStart, allowed))
        assertTrue(NectarlinkServiceContract.isAllowedBackgroundLaunch("app.nectarlink.dev", pcStart, allowed))
        assertFalse(NectarlinkServiceContract.isAllowedBackgroundLaunch("app.nectarlink", "app.nectarlink.android.MainActivity", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedBackgroundLaunch("com.other.app", pcStart, allowed))
        assertFalse(NectarlinkServiceContract.isAllowedBackgroundLaunch("app.nectarlink", "", allowed))

        assertTrue(NectarlinkServiceContract.isAllowedProvisionPackage("app.nectarlink", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedProvisionPackage("com.other.app", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedProvisionPackage("invalid", allowed))
    }

    @Test
    fun displayPointerBrightnessAndShellCommandsEnforceHelperLimits() {
        assertTrue(NectarlinkServiceContract.isAllowedBrightness(0))
        assertTrue(NectarlinkServiceContract.isAllowedBrightness(255))
        assertFalse(NectarlinkServiceContract.isAllowedBrightness(-1))
        assertFalse(NectarlinkServiceContract.isAllowedBrightness(256))

        assertTrue(NectarlinkServiceContract.isAllowedDisplayRequest(1080, 1920, 320, 8_000_000, 60, "h264"))
        assertFalse(NectarlinkServiceContract.isAllowedDisplayRequest(8, 1920, 320, 8_000_000, 60, "h264"))
        assertFalse(NectarlinkServiceContract.isAllowedDisplayRequest(1080, 1920, 50, 8_000_000, 60, "h264"))
        assertFalse(NectarlinkServiceContract.isAllowedDisplayRequest(1080, 1920, 320, 0, 60, "h264"))
        assertFalse(NectarlinkServiceContract.isAllowedDisplayRequest(1080, 1920, 320, 8_000_000, 60, "vp9"))

        assertTrue(NectarlinkServiceContract.isAllowedPointerStart("left", 0.5f))
        assertTrue(NectarlinkServiceContract.isAllowedPointerStart("right", 0.0f))
        assertFalse(NectarlinkServiceContract.isAllowedPointerStart("top", 0.5f))
        assertFalse(NectarlinkServiceContract.isAllowedPointerStart("left", Float.NaN))

        val allowed = setOf("app.nectarlink", "app.nectarlink.dev")
        assertTrue(NectarlinkServiceContract.isAllowedShellCommand("svc wifi enable", allowed))
        assertTrue(NectarlinkServiceContract.isAllowedShellCommand("settings put system screen_brightness 128", allowed))
        assertTrue(NectarlinkServiceContract.isAllowedShellCommand("pm uninstall com.example.app", allowed))
        assertTrue(
            NectarlinkServiceContract.isAllowedShellCommand(
                "am start -n app.nectarlink/${NectarlinkServiceContract.ALLOWED_BACKGROUND_ACTIVITY}",
                allowed,
            ),
        )
        assertFalse(NectarlinkServiceContract.isAllowedShellCommand("pm uninstall app.nectarlink", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedShellCommand("pm uninstall app.nectarlink.dev", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedShellCommand("am start -n app.nectarlink/.MainActivity", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedShellCommand("am start -n com.example/.MainActivity", allowed))
        assertFalse(NectarlinkServiceContract.isAllowedShellCommand("reboot", allowed))
    }

    @Test
    fun apiVersionHandshakeRejectsUnknownAndMismatchedVersions() {
        assertTrue(NectarlinkServiceContract.isSupportedApiVersion(NectarlinkServiceContract.API_VERSION))
        assertFalse(NectarlinkServiceContract.isSupportedApiVersion(0))
        assertFalse(NectarlinkServiceContract.isSupportedApiVersion(-1))
        assertFalse(NectarlinkServiceContract.isSupportedApiVersion(NectarlinkServiceContract.API_VERSION + 1))

        fun status(version: Int, enabled: Boolean = true, authorized: Boolean = true) = ServiceStatus(
            apiVersion = version,
            serviceEnabled = enabled,
            callerAuthorized = authorized,
            privilegedApp = true,
            capabilities = 0L,
            callAudioSupported = false,
            uhidAvailable = true,
            virtualDeviceSupported = true,
        )

        assertNull(
            NectarlinkServiceClient.handshakeRejectionReason(
                NectarlinkServiceContract.API_VERSION,
                status(NectarlinkServiceContract.API_VERSION),
            ),
        )
        assertNotNull(NectarlinkServiceClient.handshakeRejectionReason(0, status(1)))
        assertNotNull(NectarlinkServiceClient.handshakeRejectionReason(2, status(2)))
        assertNotNull(NectarlinkServiceClient.handshakeRejectionReason(1, status(2)))
        assertNotNull(NectarlinkServiceClient.handshakeRejectionReason(1, null))
        assertEquals(
            "service disabled",
            NectarlinkServiceClient.handshakeRejectionReason(1, status(1, enabled = false)),
        )
        assertEquals(
            "caller not authorized",
            NectarlinkServiceClient.handshakeRejectionReason(1, status(1, authorized = false)),
        )
    }
}
