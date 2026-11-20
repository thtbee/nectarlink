// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.Context
import android.os.Build
import androidx.annotation.RequiresApi
import app.nectarlink.android.BuildConfig
import app.nectarlink.core.countHealth
import app.nectarlink.core.recordCrash
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/**
 * The phone's crash records, kept with the core's in `diag/crashes`
 * (`nectarlink_core::diag::crash`): where a crash happened in NectarLink's
 * code and what kind it was, never the values it was working on.
 *
 * - [install] keeps a record of a Java or Kotlin crash as it happens.
 * - [noteExits] reads, once per start, how Android ended the app's earlier
 *   runs: a native crash or "not responding" becomes a record, and being
 *   ended in the background to save memory or battery is counted, since
 *   that's what phone makers' battery savers do.
 */
object Crashes {
    private const val KEEP = 5
    private const val MAX_TEXT = 200
    private const val PREFS = "diag"
    private const val LAST_EXIT_SEEN = "last_exit_seen"
    private const val BACKGROUND_KILLS = "background_kills"

    /** Where the diagnostics live: never backed up, never shared. */
    fun dir(context: Context): File = context.noBackupFilesDir.resolve("diag")

    /** Keeps a record of any uncaught exception, then lets it crash the app as before. */
    fun install(context: Context) {
        val crashes = dir(context).resolve("crashes")
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            runCatching { save(crashes, exceptionRecord(error, thread.name, BuildConfig.VERSION_NAME, now())) }
            previous?.uncaughtException(thread, error)
        }
    }

    /** A crash record, as the core writes it. */
    internal data class Record(
        val kind: String,
        val location: String,
        val message: String,
        val thread: String?,
        val version: String,
        val at: Long,
    ) {
        val sig: String get() = signature(kind, location, message)
    }

    /**
     * The record of [error]: where (the top three frames in NectarLink's own
     * code, without line numbers, so a moved line is still the same bug)
     * and what (the exception's class; its message is left out, since it
     * may hold anything).
     */
    internal fun exceptionRecord(error: Throwable, thread: String?, version: String, at: Long): Record {
        val root = generateSequence(error) { it.cause }.last()
        val frames = (error.stackTrace.asSequence() + root.stackTrace.asSequence())
            .filter { it.className.startsWith("app.nectarlink.") }
            .map { "${it.className}.${it.methodName}" }
            .distinct()
            .take(3)
            .toList()
        val location = frames.ifEmpty {
            error.stackTrace.take(1).map { "${it.className}.${it.methodName}" }
        }.joinToString(" < ").ifEmpty { "<unknown>" }
        val message = if (root === error) error.javaClass.name else "${error.javaClass.name} from ${root.javaClass.name}"
        return Record("exception", cut(location), cut(message), thread?.let(::cut), version, at)
    }

    /** 12 hex digits of SHA-256 over [parts], each ended by a zero byte: the core's `signature`. */
    internal fun signature(vararg parts: String): String {
        val digest = MessageDigest.getInstance("SHA-256")
        for (part in parts) {
            digest.update(part.toByteArray(Charsets.UTF_8))
            digest.update(0)
        }
        return digest.digest().take(6).joinToString("") { "%02x".format(it) }
    }

    internal fun save(crashes: File, record: Record) {
        crashes.mkdirs()
        val json = JSONObject()
            .put("sig", record.sig)
            .put("kind", record.kind)
            .put("location", record.location)
            .put("message", record.message)
            .put("version", record.version)
            .put("at", record.at)
        record.thread?.let { json.put("thread", it) }
        crashes.resolve("%011d-%s.json".format(record.at, record.sig)).writeText(json.toString())
        crashes.listFiles { f -> f.name.endsWith(".json") }
            ?.sortedByDescending { it.name }
            ?.drop(KEEP)
            ?.forEach { it.delete() }
    }

    /**
     * Reads how Android ended this app's runs since last time (Android 11+;
     * older phones don't say). Call once per start, after the core's
     * logging is set up, off the main thread.
     */
    fun noteExits(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
        runCatching { noteExitsNow(context) }
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun noteExitsNow(context: Context) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val seen = prefs.getLong(LAST_EXIT_SEEN, 0)
        val activities = context.getSystemService(ActivityManager::class.java) ?: return
        val exits = activities.getHistoricalProcessExitReasons(context.packageName, 0, 16)
            .filter { it.timestamp > seen && it.processName == context.packageName }
        if (exits.isEmpty()) return
        var killed = prefs.getInt(BACKGROUND_KILLS, 0)
        for (exit in exits) {
            when (val outcome = outcome(exit.reason, exit.importance, exit.status)) {
                is Outcome.Crash -> recordCrash(outcome.kind, outcome.location, outcome.message, null, (exit.timestamp / 1000).toULong())
                Outcome.Counted -> countHealth("crash", "ok")
                Outcome.BackgroundKill -> {
                    killed++
                    countHealth("background_kill", "ok")
                }
                Outcome.Nothing -> {}
            }
        }
        prefs.edit()
            .putLong(LAST_EXIT_SEEN, exits.maxOf { it.timestamp })
            .putInt(BACKGROUND_KILLS, killed)
            .apply()
    }

    /** How many times Android has ended the app in the background to save memory or battery. */
    fun backgroundKills(context: Context): Int =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(BACKGROUND_KILLS, 0)

    internal sealed interface Outcome {
        data class Crash(val kind: String, val location: String, val message: String) : Outcome
        /** A crash already recorded as it happened: only counted. */
        data object Counted : Outcome
        data object BackgroundKill : Outcome
        data object Nothing : Outcome
    }

    /** What one of Android's exit records means for us. */
    internal fun outcome(reason: Int, importance: Int, status: Int): Outcome = when (reason) {
        // Already recorded as it happened, by [install].
        ApplicationExitInfo.REASON_CRASH -> Outcome.Counted
        ApplicationExitInfo.REASON_CRASH_NATIVE -> Outcome.Crash("native", "<native>", "signal $status")
        ApplicationExitInfo.REASON_ANR -> Outcome.Crash("anr", "<main>", "not responding")
        ApplicationExitInfo.REASON_LOW_MEMORY,
        ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE,
        REASON_FREEZER,
        ApplicationExitInfo.REASON_OTHER,
        -> if (importance > ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND_SERVICE) {
            Outcome.BackgroundKill
        } else {
            Outcome.Nothing
        }
        else -> Outcome.Nothing
    }

    /** `ApplicationExitInfo.REASON_FREEZER` (Android 14), which older SDK stubs lack. */
    private const val REASON_FREEZER = 14

    private fun cut(text: String) = if (text.length <= MAX_TEXT) text else text.take(MAX_TEXT) + "…"

    private fun now() = System.currentTimeMillis() / 1000
}
