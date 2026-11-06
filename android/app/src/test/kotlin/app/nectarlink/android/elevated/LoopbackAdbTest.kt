// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.DataInputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.concurrent.thread

class LoopbackAdbTest {
    private fun header(command: Int, magic: Int = command.inv()): ByteArray =
        ByteBuffer.allocate(24).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(command).putInt(0).putInt(0).putInt(0).putInt(0).putInt(magic)
            .array()

    @Test
    fun helloIsAnAdbConnectMessage() {
        val hello = LoopbackAdb.hello()
        val fields = ByteBuffer.wrap(hello).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals("CNXN", String(hello, 0, 4, Charsets.US_ASCII))
        assertEquals(fields.getInt(0).inv(), fields.getInt(20))
        assertEquals(hello.size - 24, fields.getInt(12))
        assertEquals("host::\u0000", String(hello, 24, hello.size - 24, Charsets.US_ASCII))
    }

    @Test
    fun onlyAdbRepliesCount() {
        assertTrue(LoopbackAdb.isAdbReply(header(0x534c5453))) // STLS: wireless debugging wants TLS
        assertTrue(LoopbackAdb.isAdbReply(header(0x48545541))) // AUTH
        assertFalse(LoopbackAdb.isAdbReply(header(0x534c5453, magic = 0)))
        assertFalse(LoopbackAdb.isAdbReply("HTTP/1.1 400 Bad Request\r\n".toByteArray()))
        assertFalse(LoopbackAdb.isAdbReply(ByteArray(3)))
    }

    /** A loopback server on a port in the range adbd uses, answering each client with [reply]. */
    private fun server(reply: ByteArray): ServerSocket {
        val socket = (40_000..50_000).asSequence()
            .mapNotNull { runCatching { ServerSocket(it, 4, InetAddress.getByName(LoopbackAdb.HOST)) }.getOrNull() }
            .first()
        thread(isDaemon = true) {
            while (!socket.isClosed) {
                val client = runCatching { socket.accept() }.getOrNull() ?: break
                client.use {
                    runCatching {
                        DataInputStream(it.getInputStream()).readFully(ByteArray(24))
                        it.getOutputStream().write(reply)
                        it.getOutputStream().flush()
                    }
                }
            }
        }
        return socket
    }

    @Test
    fun findsTheServerThatAnswersLikeAdbd() = runBlocking {
        server(header(0x534c5453)).use { adbd ->
            server("HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n".toByteArray()).use { other ->
                assertEquals(adbd.localPort, LoopbackAdb.connectPort(listOf(other.localPort, adbd.localPort)))
                assertNull(LoopbackAdb.connectPort(listOf(other.localPort)))
                assertTrue(LoopbackAdb.openPorts().containsAll(listOf(adbd.localPort, other.localPort)))
            }
        }
    }

    @Test
    fun `a port that never answers doesn't hide the one that pairs`() = runBlocking {
        val never = java.util.concurrent.CountDownLatch(1)
        val paired = LoopbackAdb.anySucceeds(listOf(1, 2), 5_000) { port ->
            if (port == 1) never.await()
            port == 2
        }
        never.countDown()
        assertTrue(paired)
    }

    @Test
    fun `pairing fails as soon as every port has said no`() = runBlocking {
        val started = System.currentTimeMillis()
        assertFalse(LoopbackAdb.anySucceeds(listOf(1, 2), 5_000) { false })
        assertTrue(System.currentTimeMillis() - started < 4_000)
    }

    @Test
    fun `pairing gives up when no port answers in time`() = runBlocking {
        val never = java.util.concurrent.CountDownLatch(1)
        assertFalse(LoopbackAdb.anySucceeds(listOf(1), 100) { never.await(); true })
        never.countDown()
    }
}
