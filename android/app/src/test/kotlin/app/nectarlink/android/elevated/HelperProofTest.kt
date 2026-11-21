// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

class HelperProofTest {
    private val token = "0123456789abcdef".repeat(3)

    @Test
    fun answerChecksOutOnlyForTheSameToken() {
        val nonce = HelperProof.nonce()
        val answer = HelperProof.answer(token, nonce)!!
        assertTrue(HelperProof.verify(token, nonce, answer))
        assertFalse(HelperProof.verify("f".repeat(48), nonce, answer))
        assertFalse(HelperProof.verify(token, HelperProof.nonce(), answer))
        assertFalse(HelperProof.verify(token, nonce, ""))
    }

    @Test
    fun malformedChallengesGetNoAnswer() {
        assertNull(HelperProof.answer(token, "short"))
        assertNull(HelperProof.answer(token, "Z".repeat(64)))
        assertNull(HelperProof.answer(token, "a".repeat(200)))
    }

    @Test
    fun theTokenLineIsNotAChallenge() {
        val out = ByteArrayOutputStream()
        assertFalse(HelperProof.answerIfChallenge(token, token, out))
        assertEquals(0, out.size())
        try {
            HelperProof.answerIfChallenge("${HelperProof.CHALLENGE}nope", token, out)
            fail("a bad challenge should end the connection")
        } catch (_: IOException) {
        }
    }

    /** A listener that serves like the helper does with [serverToken], and reports the lines it got after the challenge. */
    private fun helper(serverToken: String, received: LinkedBlockingQueue<String>): ServerSocket {
        val server = ServerSocket(0, 4, InetAddress.getLoopbackAddress())
        thread(isDaemon = true) {
            server.accept().use { client ->
                val reader = client.getInputStream().bufferedReader()
                var line = with(InputServer) { reader.nextLine(InputServer.MAX_TOKEN_CHARS) } ?: return@use
                if (HelperProof.answerIfChallenge(line, serverToken, client.getOutputStream())) {
                    line = with(InputServer) { reader.nextLine(InputServer.MAX_TOKEN_CHARS) } ?: return@use
                }
                received.add(line)
                received.add(with(InputServer) { reader.nextLine() }.orEmpty())
            }
        }
        return server
    }

    @Test
    fun theRealHelperGetsTheTokenAndRequest() {
        val received = LinkedBlockingQueue<String>()
        helper(token, received).use { server ->
            HelperProof.open(server.localPort, token, "H", 2_000).use { socket ->
                assertNotNull(socket)
                assertEquals(token, received.poll(2, TimeUnit.SECONDS))
                assertEquals("H", received.poll(2, TimeUnit.SECONDS))
            }
        }
    }

    @Test
    fun somethingElseOnThePortGetsNothing() {
        val received = LinkedBlockingQueue<String>()
        helper("f".repeat(48), received).use { server ->
            try {
                HelperProof.open(server.localPort, token, "H", 2_000).close()
                fail("an impostor should not be trusted")
            } catch (_: IOException) {
            }
            assertNull(received.poll(500, TimeUnit.MILLISECONDS))
        }
    }

    @Test
    fun aListenerThatNeverAnswersTimesOut() {
        ServerSocket(0, 4, InetAddress.getLoopbackAddress()).use { server ->
            thread(isDaemon = true) { runCatching { server.accept().getInputStream().read() } }
            try {
                HelperProof.open(server.localPort, token, "H", 300).close()
                fail("a silent listener should not be trusted")
            } catch (_: IOException) {
            }
        }
    }
}
