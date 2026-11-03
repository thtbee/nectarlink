// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import java.util.concurrent.atomic.AtomicInteger

/**
 * Tracks stops so that in-flight starts can detect if a stop occurred
 * after they began. Multiple start calls do not cancel each other.
 */
internal class StartGuard {
    private val seq = AtomicInteger(0)
    @Volatile private var latestStoppedSeq = 0

    class Token internal constructor(internal val startSeq: Int)

    fun beginStart(): Token = Token(seq.incrementAndGet())

    fun recordStop(): Int {
        val s = seq.incrementAndGet()
        latestStoppedSeq = s
        return s
    }

    fun stop() {
        recordStop()
    }

    fun isCancelled(token: Token): Boolean = token.startSeq < latestStoppedSeq
}
