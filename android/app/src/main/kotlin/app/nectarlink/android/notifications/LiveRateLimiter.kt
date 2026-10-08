// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.os.Handler
import android.os.Looper
import app.nectarlink.core.Notification as Mirrored

/**
 * Coalesces rapid updates to live/ongoing notifications to at most one update
 * per [windowMs] (1 second) per notification `key`.
 *
 * - The first post of a key goes out immediately.
 * - Updates arriving within [windowMs] of the last sent post for that key
 *   replace the pending state and schedule a single runnable for the end of
 *   the window so the final state is always delivered.
 * - Non-live notifications (or transitions from live to non-live) cancel any
 *   pending runnable for that key and go out immediately.
 * - [cancel] and [clear] immediately unschedule pending runnables so no timers
 *   run when idle or after a notification is removed.
 */
internal class LiveRateLimiter(
    private val windowMs: Long = DEFAULT_WINDOW_MS,
    private val nowMs: () -> Long = System::currentTimeMillis,
    private val schedule: (Runnable, Long) -> Unit,
    private val unschedule: (Runnable) -> Unit,
    private val deliver: (Mirrored) -> Unit,
) {
    constructor(
        handler: Handler = Handler(Looper.getMainLooper()),
        deliver: (Mirrored) -> Unit,
    ) : this(
        windowMs = DEFAULT_WINDOW_MS,
        nowMs = System::currentTimeMillis,
        schedule = { r, delayMs -> handler.postDelayed(r, delayMs) },
        unschedule = { r -> handler.removeCallbacks(r) },
        deliver = deliver,
    )

    private class Entry(
        var lastSentAtMs: Long,
        var pending: Mirrored? = null,
        var runnable: Runnable? = null,
    )

    private val entries = HashMap<String, Entry>()

    @Synchronized
    fun onPosted(notification: Mirrored) {
        val now = nowMs()
        pruneExpired(now)
        val key = notification.key

        val live = notification.live
        if (live == null) {
            cancel(key)
            deliver(notification)
            return
        }

        val progress = live.progress
        val max = live.max
        val isComplete = !live.indeterminate && progress != null && max != null && max > 0u && progress >= max
        val existing = entries[key]
        if (existing == null || isComplete || now - existing.lastSentAtMs >= windowMs) {
            existing?.runnable?.let(unschedule)
            entries[key] = Entry(lastSentAtMs = now)
            deliver(notification)
            return
        }

        existing.pending = notification
        if (existing.runnable == null) {
            val delayMs = (windowMs - (now - existing.lastSentAtMs)).coerceAtLeast(1L)
            val task = Runnable { flush(key) }
            existing.runnable = task
            schedule(task, delayMs)
        }
    }

    @Synchronized
    private fun flush(key: String) {
        val entry = entries[key] ?: return
        entry.runnable = null
        val latest = entry.pending ?: return
        entry.pending = null
        entry.lastSentAtMs = nowMs()
        deliver(latest)
    }

    /** Cancels any pending throttled update for [key] and forgets its timer state. */
    @Synchronized
    fun cancel(key: String) {
        val removed = entries.remove(key) ?: return
        removed.runnable?.let(unschedule)
        removed.runnable = null
        removed.pending = null
    }

    /** Cancels all pending throttled updates and clears all state. */
    @Synchronized
    fun clear() {
        for (entry in entries.values) {
            entry.runnable?.let(unschedule)
        }
        entries.clear()
    }

    @Synchronized
    internal fun pendingCountForTest(): Int = entries.values.count { it.runnable != null }

    private fun pruneExpired(now: Long) {
        val iter = entries.entries.iterator()
        while (iter.hasNext()) {
            val e = iter.next().value
            if (e.runnable == null && e.pending == null && now - e.lastSentAtMs >= windowMs) {
                iter.remove()
            }
        }
    }

    companion object {
        const val DEFAULT_WINDOW_MS = 1_000L
    }
}
