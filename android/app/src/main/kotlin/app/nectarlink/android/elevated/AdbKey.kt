// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import java.io.ByteArrayOutputStream
import java.io.File
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.cert.Certificate
import java.security.cert.CertificateFactory
import java.security.spec.PKCS8EncodedKeySpec
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * This app's key for the phone's own wireless debugging: an RSA key and a
 * self-signed certificate (what adbd remembers after pairing). Kept in the
 * app's private storage; whoever holds it can open a shell on this phone,
 * so it never leaves it.
 */
internal class AdbKey private constructor(val privateKey: PrivateKey, val certificate: Certificate) {
    companion object {
        private const val KEY_FILE = "adb.key"
        private const val CERT_FILE = "adb.crt"

        /** The stored key, or a new one. */
        fun load(dir: File): AdbKey {
            val keyFile = File(dir, KEY_FILE)
            val certFile = File(dir, CERT_FILE)
            if (keyFile.exists() && certFile.exists()) {
                runCatching {
                    val key = KeyFactory.getInstance("RSA").generatePrivate(PKCS8EncodedKeySpec(keyFile.readBytes()))
                    val cert = CertificateFactory.getInstance("X.509").generateCertificate(certFile.inputStream())
                    return AdbKey(key, cert)
                }
            }
            val pair = KeyPairGenerator.getInstance("RSA").apply { initialize(2048, SecureRandom()) }.generateKeyPair()
            val der = selfSignedCertificate(pair, "CN=Nectarlink")
            dir.mkdirs()
            certFile.writeBytes(der)
            keyFile.writeBytes(pair.private.encoded)
            val cert = CertificateFactory.getInstance("X.509").generateCertificate(der.inputStream())
            return AdbKey(pair.private, cert)
        }

        /** Forgets the key (unpaired: a new one is made next time). */
        fun delete(dir: File) {
            File(dir, KEY_FILE).delete()
            File(dir, CERT_FILE).delete()
        }

        /**
         * A minimal X.509 v3 certificate for `pair`, signed by its own key
         * (SHA-256 with RSA), valid for 30 years.
         */
        internal fun selfSignedCertificate(pair: KeyPair, name: String): ByteArray {
            val sha256WithRsa = sequence(oid(byteArrayOf(0x2a, 0x86.toByte(), 0x48, 0x86.toByte(), 0xf7.toByte(), 0x0d, 0x01, 0x01, 0x0b)), byteArrayOf(0x05, 0x00))
            val subject = name(name)
            val now = System.currentTimeMillis()
            val tbs = sequence(
                tlv(0xa0, integer(BigInteger.valueOf(2))), // v3
                integer(BigInteger(64, SecureRandom()).add(BigInteger.ONE)),
                sha256WithRsa,
                subject,
                sequence(time(Date(now - DAY)), time(Date(now + 30 * 365 * DAY))),
                subject,
                pair.public.encoded, // SubjectPublicKeyInfo, already DER
            )
            val signature = Signature.getInstance("SHA256withRSA").run {
                initSign(pair.private)
                update(tbs)
                sign()
            }
            return sequence(tbs, sha256WithRsa, tlv(0x03, byteArrayOf(0) + signature))
        }

        private const val DAY = 24L * 3600 * 1000

        // ---- DER ----

        private fun tlv(tag: Int, content: ByteArray): ByteArray {
            val out = ByteArrayOutputStream()
            out.write(tag)
            val n = content.size
            when {
                n < 0x80 -> out.write(n)
                n < 0x100 -> { out.write(0x81); out.write(n) }
                n < 0x10000 -> { out.write(0x82); out.write(n shr 8); out.write(n and 0xff) }
                else -> { out.write(0x83); out.write(n shr 16); out.write((n shr 8) and 0xff); out.write(n and 0xff) }
            }
            out.write(content)
            return out.toByteArray()
        }

        private fun sequence(vararg parts: ByteArray) = tlv(0x30, parts.fold(ByteArray(0)) { all, part -> all + part })

        private fun integer(value: BigInteger) = tlv(0x02, value.toByteArray())

        private fun oid(encoded: ByteArray) = tlv(0x06, encoded)

        /** `CN=...` as a Name with one UTF8String attribute. */
        private fun name(dn: String): ByteArray {
            val commonName = oid(byteArrayOf(0x55, 0x04, 0x03))
            val value = tlv(0x0c, dn.removePrefix("CN=").toByteArray(Charsets.UTF_8))
            return sequence(tlv(0x31, sequence(commonName, value)))
        }

        /** UTCTime (years before 2050) or GeneralizedTime. */
        private fun time(date: Date): ByteArray {
            val utc = TimeZone.getTimeZone("UTC")
            val year = java.util.Calendar.getInstance(utc).apply { time = date }.get(java.util.Calendar.YEAR)
            return if (year < 2050) {
                tlv(0x17, SimpleDateFormat("yyMMddHHmmss'Z'", Locale.ROOT).apply { timeZone = utc }.format(date).toByteArray())
            } else {
                tlv(0x18, SimpleDateFormat("yyyyMMddHHmmss'Z'", Locale.ROOT).apply { timeZone = utc }.format(date).toByteArray())
            }
        }
    }
}
