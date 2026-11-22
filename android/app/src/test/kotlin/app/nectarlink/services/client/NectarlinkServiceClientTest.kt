// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services.client

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
}
