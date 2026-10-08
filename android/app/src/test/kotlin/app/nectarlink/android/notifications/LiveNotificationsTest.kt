// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import app.nectarlink.core.LivePoint
import app.nectarlink.core.LiveSegment
import app.nectarlink.core.Notification
import app.nectarlink.core.NotificationLive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveNotificationsTest {

    private fun liveNote(
        key: String = "0|com.ubercab|1",
        progress: UInt = 30u,
        max: UInt = 100u,
        indeterminate: Boolean = false,
        chip: String? = "5 min",
    ): Notification = Notification(
        key = key,
        app = "com.ubercab",
        appName = "Uber",
        title = "Your driver is on the way",
        text = "Arriving in 5 min",
        sub = null,
        `when` = 1_700_000_000_000L,
        actions = emptyList(),
        silent = true,
        icon = null,
        image = null,
        live = NotificationLive(
            progress = if (indeterminate) null else progress,
            max = if (indeterminate) null else max,
            indeterminate = indeterminate,
            chip = chip,
            segments = emptyList(),
            points = emptyList(),
            chronometer = false,
            countdown = false,
        ),
    )

    @Test
    fun `rate limiter delivers first live post immediately and coalesces burst within 1s`() {
        var now = 1_000L
        val scheduled = mutableListOf<Pair<Runnable, Long>>()
        val delivered = mutableListOf<Notification>()

        val limiter = LiveRateLimiter(
            windowMs = 1_000L,
            nowMs = { now },
            schedule = { r, delayMs -> scheduled += r to delayMs },
            unschedule = { r -> scheduled.removeAll { it.first === r } },
            deliver = { delivered += it },
        )

        // First post goes out immediately with zero scheduled timers.
        limiter.onPosted(liveNote(progress = 10u))
        assertEquals(1, delivered.size)
        assertEquals(10u, delivered.last().live?.progress)
        assertEquals(0, limiter.pendingCountForTest())
        assertTrue(scheduled.isEmpty())

        // Rapid updates at +100ms, +250ms, +600ms coalesce into one scheduled runnable.
        now = 1_100L
        limiter.onPosted(liveNote(progress = 20u))
        now = 1_250L
        limiter.onPosted(liveNote(progress = 35u))
        now = 1_600L
        limiter.onPosted(liveNote(progress = 50u))

        assertEquals(1, delivered.size)
        assertEquals(1, limiter.pendingCountForTest())
        assertEquals(1, scheduled.size)
        assertEquals(900L, scheduled.single().second)

        // When the timer fires at +1000ms, only the latest state (50%) is sent.
        now = 2_000L
        val task = scheduled.removeAt(0).first
        task.run()

        assertEquals(2, delivered.size)
        assertEquals(50u, delivered.last().live?.progress)
        assertEquals(0, limiter.pendingCountForTest())
    }

    @Test
    fun `rate limiter delivers 100 percent completion immediately and cancels pending timer`() {
        var now = 5_000L
        val scheduled = mutableListOf<Runnable>()
        val delivered = mutableListOf<Notification>()

        val limiter = LiveRateLimiter(
            windowMs = 1_000L,
            nowMs = { now },
            schedule = { r, _ -> scheduled += r },
            unschedule = { r -> scheduled.remove(r) },
            deliver = { delivered += it },
        )

        limiter.onPosted(liveNote(progress = 40u))
        now = 5_200L
        limiter.onPosted(liveNote(progress = 80u))
        assertEquals(1, scheduled.size)

        // Reaching 100% within the window flushes immediately and cancels the timer.
        now = 5_350L
        limiter.onPosted(liveNote(progress = 100u))
        assertEquals(2, delivered.size)
        assertEquals(100u, delivered.last().live?.progress)
        assertTrue(scheduled.isEmpty())
        assertEquals(0, limiter.pendingCountForTest())
    }

    @Test
    fun `rate limiter cancels pending timer on removal or clear leaving zero timers`() {
        var now = 10_000L
        val scheduled = mutableListOf<Runnable>()
        val delivered = mutableListOf<Notification>()

        val limiter = LiveRateLimiter(
            windowMs = 1_000L,
            nowMs = { now },
            schedule = { r, _ -> scheduled += r },
            unschedule = { r -> scheduled.remove(r) },
            deliver = { delivered += it },
        )

        limiter.onPosted(liveNote(key = "k1", progress = 10u))
        limiter.onPosted(liveNote(key = "k2", progress = 10u))
        now = 10_100L
        limiter.onPosted(liveNote(key = "k1", progress = 20u))
        limiter.onPosted(liveNote(key = "k2", progress = 30u))
        assertEquals(2, scheduled.size)

        limiter.cancel("k1")
        assertEquals(1, scheduled.size)
        assertEquals(1, limiter.pendingCountForTest())

        limiter.clear()
        assertTrue(scheduled.isEmpty())
        assertEquals(0, limiter.pendingCountForTest())
    }

    @Test
    fun `buildLive extracts progress, segments, points, chip and chronometer`() {
        // Non-live notification returns null.
        assertNull(
            NotificationReader.buildLive(
                progress = 0,
                progressMax = 0,
                indeterminate = false,
                chip = null,
                segments = emptyList(),
                points = emptyList(),
                chronometer = false,
                countdown = false,
                promoted = false,
            ),
        )

        // Segmented ProgressStyle computes max from segments if progressMax is 0, clamps progress & chip.
        val segmented = NotificationReader.buildLive(
            progress = 75,
            progressMax = 0,
            indeterminate = false,
            chip = "  7 mins away from stop  ",
            segments = listOf(
                LiveSegment(length = 30u, color = 0xFF4CAF50u),
                LiveSegment(length = 70u, color = null),
            ),
            points = listOf(
                LivePoint(position = 30u, color = null),
                LivePoint(position = 250u, color = null), // out of range, filtered
            ),
            chronometer = true,
            countdown = true,
            promoted = true,
        )
        assertNotNull(segmented)
        assertEquals(75u, segmented!!.progress)
        assertEquals(100u, segmented.max)
        assertFalse(segmented.indeterminate)
        assertEquals(16, segmented.chip?.length)
        assertEquals(2, segmented.segments.size)
        assertEquals(1, segmented.points.size)
        assertTrue(segmented.chronometer)
        assertTrue(segmented.countdown)
    }

    @Test
    fun `task notifications format duration and completion strings`() {
        assertEquals("Took 38s", TaskNotifications.formatTook(38_100L))
        assertEquals("Took 4m 12s", TaskNotifications.formatTook(252_000L))
        assertEquals("Took 1h 04m", TaskNotifications.formatTook(3_840_000L))

        assertEquals("cargo test finished", TaskNotifications.completionTitle("cargo test", 0))
        assertEquals("cargo build failed (exit 101)", TaskNotifications.completionTitle("cargo build", 101))
        assertEquals("Took 4m 12s · My-PC", TaskNotifications.completionBody("My-PC", 252_000L))
        assertEquals("cargo…", TaskNotifications.shortChipText("cargo build --release"))
    }
}
