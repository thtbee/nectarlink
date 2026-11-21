// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

/** When to ask, on Home, to come chat on Discord (kept in [Preferences]). */
internal object CommunityInvite {
    /** At most once in this long. */
    const val EVERY_MS = 30L * 24 * 60 * 60 * 1000
    /** And not after it was answered this many times. */
    const val TIMES = 3

    /** Whether an update should bring the card back now. */
    fun due(done: Boolean, armed: Boolean, count: Int, lastAt: Long, now: Long): Boolean =
        !done && !armed && count < TIMES && now - lastAt >= EVERY_MS
}
