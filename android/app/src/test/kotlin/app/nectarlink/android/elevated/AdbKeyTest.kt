// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.security.KeyPairGenerator
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate

class AdbKeyTest {
    @get:Rule val folder = TemporaryFolder()

    @Test
    fun theCertificateIsValidAndSelfSigned() {
        val pair = KeyPairGenerator.getInstance("RSA").apply { initialize(2048) }.generateKeyPair()
        val der = AdbKey.selfSignedCertificate(pair, "CN=Nectarlink")
        val cert = CertificateFactory.getInstance("X.509").generateCertificate(der.inputStream()) as X509Certificate
        assertEquals(3, cert.version)
        assertEquals("CN=Nectarlink", cert.subjectX500Principal.name)
        assertEquals(cert.subjectX500Principal, cert.issuerX500Principal)
        assertArrayEquals(pair.public.encoded, cert.publicKey.encoded)
        cert.checkValidity()
        cert.verify(pair.public) // Signed by its own key.
        assertEquals("SHA256withRSA", cert.sigAlgName)
    }

    @Test
    fun theKeyIsKept() {
        val dir = folder.newFolder()
        val first = AdbKey.load(dir)
        val again = AdbKey.load(dir)
        assertArrayEquals(first.privateKey.encoded, again.privateKey.encoded)
        assertArrayEquals(first.certificate.encoded, again.certificate.encoded)
        AdbKey.delete(dir)
        val fresh = AdbKey.load(dir)
        assert(!fresh.privateKey.encoded.contentEquals(first.privateKey.encoded))
    }
}
