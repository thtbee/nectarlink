// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import java.math.BigInteger
import java.security.spec.ECPoint
import org.junit.Assert.assertArrayEquals
import org.junit.Test

class LinuxUnlockKeysTest {
    @Test
    fun sec1PointUsesUnsignedFixedWidthCoordinates() {
        val x = BigInteger("80" + "12".repeat(31), 16)
        val y = BigInteger("34", 16)
        val expected = byteArrayOf(0x04, 0x80.toByte()) +
            ByteArray(31) { 0x12 } + ByteArray(31) + byteArrayOf(0x34)

        assertArrayEquals(expected, encodeLinuxUnlockPublicKey(ECPoint(x, y)))
    }
}
