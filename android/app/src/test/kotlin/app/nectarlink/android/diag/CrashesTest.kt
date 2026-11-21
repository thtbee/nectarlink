// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

import android.app.ActivityManager.RunningAppProcessInfo
import android.app.ApplicationExitInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class CrashesTest {
    @Test
    fun signaturesMatchTheCores() {
        // nectarlink_core::diag::crash::signature(&["exception", "app.nectarlink.android.Foo.bar",
        // "java.lang.IllegalStateException"]), so a phone's records group with the core's.
        assertEquals(
            "b3af6c6e31db",
            Crashes.signature("exception", "app.nectarlink.android.Foo.bar", "java.lang.IllegalStateException"),
        )
    }

    @Test
    fun anExceptionKeepsWhereAndWhatButNotItsMessage() {
        val error = IllegalStateException("can't send to Mum: 0712345678")
        error.stackTrace = arrayOf(
            StackTraceElement("java.util.ArrayList", "get", "ArrayList.java", 10),
            StackTraceElement("app.nectarlink.android.sms.Sender", "send", "Sender.kt", 42),
            StackTraceElement("app.nectarlink.android.sms.Queue", "drain", "Queue.kt", 7),
        )
        val record = Crashes.exceptionRecord(error, "main", "0.1.6", 100)
        assertEquals("exception", record.kind)
        assertEquals("app.nectarlink.android.sms.Sender.send < app.nectarlink.android.sms.Queue.drain", record.location)
        assertEquals("java.lang.IllegalStateException", record.message)
        assertFalse(record.toString().contains("0712"))
    }

    @Test
    fun theSameBugKeepsItsSignatureWhenLinesMove() {
        fun crashAt(line: Int) = IllegalStateException().apply {
            stackTrace = arrayOf(StackTraceElement("app.nectarlink.android.X", "y", "X.kt", line))
        }
        assertEquals(
            Crashes.exceptionRecord(crashAt(10), null, "0.1.6", 1).sig,
            Crashes.exceptionRecord(crashAt(99), null, "0.1.7", 2).sig,
        )
    }

    @Test
    fun aCauseIsNamedToo() {
        val error = RuntimeException(IllegalArgumentException("x"))
        error.stackTrace = arrayOf(StackTraceElement("app.nectarlink.android.X", "y", "X.kt", 1))
        assertEquals(
            "java.lang.RuntimeException from java.lang.IllegalArgumentException",
            Crashes.exceptionRecord(error, null, "0.1.6", 1).message,
        )
    }

    @Test
    fun androidsExitReasonsMapToCrashesAndBackgroundKills() {
        val background = RunningAppProcessInfo.IMPORTANCE_CACHED
        val onScreen = RunningAppProcessInfo.IMPORTANCE_FOREGROUND
        assertEquals(Crashes.Outcome.Counted, Crashes.outcome(ApplicationExitInfo.REASON_CRASH, onScreen, 0))
        assertEquals(
            Crashes.Outcome.Crash("native", "<native>", "signal 11"),
            Crashes.outcome(ApplicationExitInfo.REASON_CRASH_NATIVE, onScreen, 11),
        )
        assertEquals(
            Crashes.Outcome.Crash("anr", "<main>", "not responding"),
            Crashes.outcome(ApplicationExitInfo.REASON_ANR, onScreen, 0),
        )
        assertEquals(Crashes.Outcome.BackgroundKill, Crashes.outcome(ApplicationExitInfo.REASON_LOW_MEMORY, background, 0))
        assertEquals(Crashes.Outcome.BackgroundKill, Crashes.outcome(ApplicationExitInfo.REASON_OTHER, background, 0))
        // Ended while on screen or as a foreground service: not a battery saver.
        assertEquals(Crashes.Outcome.Nothing, Crashes.outcome(ApplicationExitInfo.REASON_LOW_MEMORY, onScreen, 0))
        assertEquals(Crashes.Outcome.Nothing, Crashes.outcome(ApplicationExitInfo.REASON_USER_REQUESTED, background, 0))
    }
}
