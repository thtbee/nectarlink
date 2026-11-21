// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * How the app knows it reached the helper it started, before it sends the
 * helper's token or anything else. The app sends a random challenge; the
 * helper answers with an HMAC of it keyed by the token it was started with.
 * Something else listening on that port (say, after the helper stopped and
 * another app took the port) can't answer, so the app sends it nothing.
 *
 * After the answer the connection goes on as before: the token, then the
 * first line. A client that sends the token first is still served.
 */
internal object HelperProof {
    const val CHALLENGE = "PROVE "
    const val ANSWER = "PROOF "
    private const val NONCE_BYTES = 32
    private const val MAX_ANSWER_CHARS = 256
    private const val CONTEXT = "nectarlink-helper-proof\n"

    fun nonce(): String = ByteArray(NONCE_BYTES).also(SecureRandom()::nextBytes).toHex()

    /** The answer to [nonce] for [token]; null when [nonce] isn't one the app could have sent. */
    fun answer(token: String, nonce: String): String? {
        if (nonce.length !in 32..128 || !nonce.all { it in '0'..'9' || it in 'a'..'f' }) return null
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(token.toByteArray(Charsets.UTF_8), "HmacSHA256"))
        return mac.doFinal((CONTEXT + nonce).toByteArray(Charsets.UTF_8)).toHex()
    }

    /** Whether [given] is the right answer to [nonce] for [token], compared in constant time. */
    fun verify(token: String, nonce: String, given: String): Boolean {
        val expected = answer(token, nonce) ?: return false
        return MessageDigest.isEqual(expected.toByteArray(), given.toByteArray())
    }

    /**
     * The helper's side: answers [line] if it's a challenge. True when it was
     * one (and answered), false when it's something else, such as the token.
     */
    fun answerIfChallenge(line: String, token: String, out: OutputStream): Boolean {
        if (!line.startsWith(CHALLENGE)) return false
        val answer = answer(token, line.removePrefix(CHALLENGE)) ?: throw IOException("bad challenge")
        out.write("$ANSWER$answer\n".toByteArray(Charsets.UTF_8))
        out.flush()
        return true
    }

    /**
     * The app's side: connects to the helper on [port], checks it holds
     * [token], then sends the token and [request]. Throws when it doesn't
     * answer in [timeoutMs] or answers wrong; nothing was sent to it then.
     */
    fun open(port: Int, token: String, request: String, timeoutMs: Int): Socket {
        val socket = Socket(InetAddress.getLoopbackAddress(), port)
        try {
            socket.tcpNoDelay = true
            socket.soTimeout = timeoutMs
            val out = socket.getOutputStream()
            val nonce = nonce()
            out.write("$CHALLENGE$nonce\n".toByteArray(Charsets.UTF_8))
            out.flush()
            val reply = readLine(socket.getInputStream())
            if (!reply.startsWith(ANSWER) || !verify(token, nonce, reply.removePrefix(ANSWER))) {
                throw IOException("the helper on port $port didn't prove it's ours")
            }
            socket.soTimeout = 0
            out.write("$token\n$request\n".toByteArray(Charsets.UTF_8))
            out.flush()
            return socket
        } catch (e: Exception) {
            runCatching { socket.close() }
            throw e
        }
    }

    /** One line, read byte by byte so nothing after it is taken from the stream. */
    private fun readLine(input: InputStream): String {
        val line = StringBuilder()
        while (true) {
            val b = input.read()
            if (b < 0 || b == '\n'.code) return line.toString()
            if (line.length >= MAX_ANSWER_CHARS) throw IOException("answer too long")
            line.append(b.toChar())
        }
    }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
}
