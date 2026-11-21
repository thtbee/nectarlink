// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import java.util.concurrent.CompletableFuture
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UnlockPromptsTest {
    @Test
    fun cancellationEndsTheWaitForBiometricApproval() = runBlocking {
        val biometric = CompletableFuture<String?>()
        val cancel = CompletableDeferred<Unit>()
        val answer = async { awaitUnlockResult(biometric) { cancel.await() } }

        cancel.complete(Unit)

        assertNull(withTimeout(1_000L) { answer.await() })
    }

    @Test
    fun biometricApprovalEndsTheCancellationWait() = runBlocking {
        val biometric = CompletableFuture<String?>()
        val listening = CompletableDeferred<Unit>()
        val stopped = CompletableDeferred<Unit>()
        val answer = async {
            awaitUnlockResult(biometric) {
                try {
                    listening.complete(Unit)
                    awaitCancellation()
                } finally {
                    stopped.complete(Unit)
                }
            }
        }

        withTimeout(1_000L) { listening.await() }
        biometric.complete("approved")

        assertEquals("approved", withTimeout(1_000L) { answer.await() })
        assertTrue(stopped.isCompleted)
    }
}
