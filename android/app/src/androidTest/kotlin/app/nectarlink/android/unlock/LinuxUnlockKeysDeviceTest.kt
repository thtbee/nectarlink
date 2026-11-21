// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import android.os.Build
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import app.nectarlink.core.UnlockRefusal
import java.security.KeyStore
import java.util.UUID
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

@SdkSuppress(minSdkVersion = Build.VERSION_CODES.R)
class LinuxUnlockKeysDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val pc = UUID.randomUUID().toString()
    private val uid = 1000u

    @Test
    fun missingEnrollmentDoesNotOpenPrompt() {
        assertThrows(UnlockRefusal.KeyGone::class.java) {
            LinuxUnlockKeys.sign(context, pc, uid, byteArrayOf(1), 0)
        }
    }

    @Test
    fun timedOutEnrollmentDiscardsItsUnconfirmedKey() {
        assumeTrue(UnlockKeys.supported(context))
        try {
            assertThrows(UnlockRefusal.Declined::class.java) {
                LinuxUnlockKeys.enroll(context, pc, "Test PC", "testuser", uid, 0)
            }
            assertThrows(UnlockRefusal.KeyGone::class.java) {
                LinuxUnlockKeys.sign(context, pc, uid, byteArrayOf(1), 0)
            }
            val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            assertTrue(store.aliases().toList().none { it.startsWith("nl-linux-unlock-$pc-$uid-") })
        } finally {
            LinuxUnlockKeys.forget(context, pc, uid)
        }
    }
}
