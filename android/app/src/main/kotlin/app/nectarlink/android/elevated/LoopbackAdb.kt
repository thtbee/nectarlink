// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.os.Build
import io.github.muntashirakon.adb.AdbPairingRequiredException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.DataInputStream
import java.net.BindException
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

/**
 * Finds wireless debugging's ports on this phone without searching the
 * network. adbd listens on random ports in Linux's ephemeral range, which
 * the usual mDNS lookup finds; from Android 17 that lookup makes the system
 * ask the user to choose a device each time (local network protection), even
 * though the ports are on this phone. So the ports on 127.0.0.1 are tried
 * instead, which isn't the local network and asks nobody.
 */
internal object LoopbackAdb {
    const val HOST = "127.0.0.1"

    /** Where Linux picks listening ports nobody asked a number for. */
    private val EPHEMERAL = 32768..60999

    /** NectarLink's own fixed ports (LocalSend, USB), never adbd's. */
    private val OWN = 53317..53322

    private const val OPEN_MS = 50
    private const val ANSWER_MS = 500
    private const val SCANNERS = 32

    /** Whether to use this instead of mDNS: Android 17 and later (API 37). */
    fun preferred(): Boolean = Build.VERSION.SDK_INT >= 37

    /** The ports listening on 127.0.0.1 in the ephemeral range, except NectarLink's own. */
    suspend fun openPorts(): List<Int> = withContext(Dispatchers.IO) {
        val ports = EPHEMERAL.filterNot { it in OWN }
        coroutineScope {
            ports.chunked(ports.size / SCANNERS + 1)
                .map { chunk -> async { chunk.filter(::isTaken) } }
                .awaitAll()
                .flatten()
                .filter(::isOpen)
        }
    }

    /**
     * Runs [attempt] on every port at once, each on its own thread, and tells
     * if one succeeded within [timeoutMs]. Attempts that never return are left behind.
     */
    suspend fun anySucceeds(ports: List<Int>, timeoutMs: Long, attempt: (Int) -> Boolean): Boolean {
        val result = CompletableDeferred<Boolean>()
        val left = AtomicInteger(ports.size)
        for (port in ports) {
            thread(isDaemon = true, name = "adb-pair") {
                if (attempt(port)) result.complete(true)
                else if (left.decrementAndGet() == 0) result.complete(false)
            }
        }
        return withTimeoutOrNull(timeoutMs) { result.await() } ?: false
    }

    /**
     * Connects the usual way ([found], through mDNS), or else on 127.0.0.1
     * ([loopback]). adbd saying it no longer knows this phone isn't a miss:
     * it goes to the caller, so the user is asked to pair again.
     */
    suspend fun foundOrLoopback(
        found: suspend () -> Boolean,
        loopback: suspend () -> Boolean,
        onMissed: (Exception) -> Unit = {},
    ): Boolean {
        val connected = try {
            found()
        } catch (e: AdbPairingRequiredException) {
            throw e
        } catch (e: Exception) {
            onMissed(e)
            false
        }
        return connected || loopback()
    }

    /** adbd's port for connecting (it answers an adb hello), among [open]. */
    suspend fun connectPort(open: List<Int>): Int? = withContext(Dispatchers.IO) {
        open.firstOrNull(::answersLikeAdbd)
    }

    /**
     * Whether something holds [port], by trying to take it. Android reports
     * every connect() to the system, so connecting to the whole range floods
     * its binder and other apps start failing. Binding reports nothing.
     */
    private fun isTaken(port: Int): Boolean = try {
        ServerSocket().use {
            it.reuseAddress = false
            it.bind(InetSocketAddress(HOST, port), 1)
        }
        false
    } catch (_: BindException) {
        true
    } catch (_: java.io.IOException) {
        false
    }

    private fun isOpen(port: Int): Boolean = runCatching {
        Socket().use { it.connect(InetSocketAddress(HOST, port), OPEN_MS) }
        true
    }.getOrDefault(false)

    /** Sends adb's opening CNXN message and checks the reply is adb's. */
    private fun answersLikeAdbd(port: Int): Boolean = runCatching {
        Socket().use { socket ->
            socket.connect(InetSocketAddress(HOST, port), ANSWER_MS)
            socket.soTimeout = ANSWER_MS
            socket.getOutputStream().apply {
                write(hello())
                flush()
            }
            val header = ByteArray(HEADER_BYTES)
            DataInputStream(socket.getInputStream()).readFully(header)
            isAdbReply(header)
        }
    }.getOrDefault(false)

    private const val HEADER_BYTES = 24
    private const val A_CNXN = 0x4e584e43
    private const val A_AUTH = 0x48545541
    private const val A_STLS = 0x534c5453
    private const val VERSION = 0x01000001
    private const val MAX_DATA = 256 * 1024

    /** adb's CNXN message (`host::`), as a client sends it first. */
    internal fun hello(): ByteArray {
        val payload = "host::\u0000".toByteArray(Charsets.US_ASCII)
        val checksum = payload.sumOf { it.toInt() and 0xff }
        return ByteBuffer.allocate(HEADER_BYTES + payload.size).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(A_CNXN)
            .putInt(VERSION)
            .putInt(MAX_DATA)
            .putInt(payload.size)
            .putInt(checksum)
            .putInt(A_CNXN.inv())
            .put(payload)
            .array()
    }

    /** Whether a 24-byte message header is adbd answering a hello. */
    internal fun isAdbReply(header: ByteArray): Boolean {
        if (header.size < HEADER_BYTES) return false
        val fields = ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN)
        val command = fields.getInt(0)
        val magic = fields.getInt(20)
        return command in setOf(A_STLS, A_AUTH, A_CNXN) && magic == command.inv()
    }
}
