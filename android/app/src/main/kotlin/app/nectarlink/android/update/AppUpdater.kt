// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.core.content.edit
import androidx.core.net.toUri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.NectarlinkApplication
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates from GitHub releases: checks the latest release, and installs a
 * newer signed APK after checking it against the release's SHA-256 sums.
 * Android only installs it over this app when it's signed with the same
 * key, so only release builds update themselves (debug builds are signed
 * differently).
 */
class AppUpdater(context: Context) {
    private val context = context.applicationContext
    private val prefs = context.getSharedPreferences("updates", Context.MODE_PRIVATE)

    /** A newer release. */
    data class Update(val version: String, val notesUrl: String, internal val apkUrl: String, internal val apkName: String, internal val sumsUrl: String)

    sealed interface State {
        data object Idle : State
        data object Checking : State
        data class Available(val update: Update) : State
        data class Downloading(val update: Update) : State
        /** Waiting for Android (and the user) to install it. */
        data class Installing(val update: Update) : State
        data class Failed(val update: Update?, val reason: Reason) : State
    }

    enum class Reason { Offline, Damaged, NotAllowed, Install }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    /** Whether this build updates itself. */
    val enabled: Boolean get() = !BuildConfig.DEBUG

    /** Checks at most once a day (call when the app comes to the front). */
    suspend fun checkIfDue() {
        val last = prefs.getLong(LAST_CHECK, 0)
        if (System.currentTimeMillis() - last < CHECK_EVERY_MS) return
        check()
    }

    /** Checks now; returns whether an update is available. */
    suspend fun check(): Boolean {
        if (!enabled) return false
        _state.value = State.Checking
        val found = withContext(Dispatchers.IO) {
            runCatching { latest() }.onFailure { Log.i(TAG, "couldn't check for updates", it) }
        }
        prefs.edit { putLong(LAST_CHECK, System.currentTimeMillis()) }
        _state.value = found.fold(
            onSuccess = { update -> update?.let { State.Available(it) } ?: State.Idle },
            onFailure = { State.Failed(null, Reason.Offline) },
        )
        return _state.value is State.Available
    }

    private fun latest(): Update? {
        val release = JSONObject(String(get(LATEST, 1 shl 20)))
        if (release.optBoolean("draft") || release.optBoolean("prerelease")) return null
        val version = release.getString("tag_name").removePrefix("v")
        if (!isNewer(version, BuildConfig.VERSION_NAME)) return null
        val assets = release.getJSONArray("assets")
        var apk: JSONObject? = null
        var sums: JSONObject? = null
        for (i in 0 until assets.length()) {
            val asset = assets.getJSONObject(i)
            val name = asset.getString("name")
            if (name.endsWith("-android.apk")) apk = asset
            if (name == "SHA256SUMS.txt") sums = asset
        }
        if (apk == null || sums == null) return null
        return Update(
            version = version,
            notesUrl = release.getString("html_url"),
            apkUrl = apk.getString("browser_download_url"),
            apkName = apk.getString("name"),
            sumsUrl = sums.getString("browser_download_url"),
        )
    }

    /** Whether this app may install apps (the user allows it once, in Settings). */
    fun canInstall(): Boolean = context.packageManager.canRequestPackageInstalls()

    /** Android's screen where the user lets Nectarlink install updates. */
    fun allowInstallsIntent(): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, "package:${context.packageName}".toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Downloads, checks and hands the update to Android's installer. */
    suspend fun install(update: Update) {
        if (!canInstall()) {
            _state.value = State.Failed(update, Reason.NotAllowed)
            return
        }
        _state.value = State.Downloading(update)
        val apk = withContext(Dispatchers.IO) { runCatching { download(update) } }.getOrElse {
            Log.w(TAG, "update download failed", it)
            _state.value = State.Failed(update, if (it is DamagedException) Reason.Damaged else Reason.Offline)
            return
        }
        _state.value = State.Installing(update)
        withContext(Dispatchers.IO) { runCatching { commit(apk) } }.onFailure {
            Log.w(TAG, "couldn't start the install", it)
            _state.value = State.Failed(update, Reason.Install)
        }
    }

    private class DamagedException : Exception("checksum mismatch")

    private fun download(update: Update): File {
        val sums = String(get(update.sumsUrl, 64 * 1024))
        val expected = listedHash(sums, update.apkName) ?: throw DamagedException()
        val dir = File(context.cacheDir, "updates").apply { deleteRecursively(); mkdirs() }
        val file = File(dir, update.apkName)
        val digest = MessageDigest.getInstance("SHA-256")
        open(update.apkUrl).inputStream.use { input ->
            file.outputStream().use { output ->
                val buffer = ByteArray(256 * 1024)
                var total = 0L
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    total += read
                    if (total > MAX_APK) throw DamagedException()
                    digest.update(buffer, 0, read)
                    output.write(buffer, 0, read)
                }
            }
        }
        val actual = digest.digest().joinToString("") { "%02x".format(it) }
        if (actual != expected) {
            file.delete()
            throw DamagedException()
        }
        return file
    }

    private fun commit(apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            apk.inputStream().use { input ->
                session.openWrite("base.apk", 0, apk.length()).use { output ->
                    input.copyTo(output)
                    session.fsync(output)
                }
            }
            val done = PendingIntent.getBroadcast(
                context,
                id,
                Intent(context, InstallResult::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
            )
            session.commit(done.intentSender)
        }
    }

    /** Android's answer: ask the user to confirm, or it's done (or failed). */
    class InstallResult : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
                PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                    val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                    } else {
                        @Suppress("DEPRECATION")
                        intent.getParcelableExtra(Intent.EXTRA_INTENT)
                    }
                    confirm?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)?.let(context::startActivity)
                }
                PackageInstaller.STATUS_SUCCESS -> Unit // The new version starts fresh.
                else -> {
                    Log.w(TAG, "install failed: ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}")
                    (context.applicationContext as NectarlinkApplication).updater.installFailed()
                }
            }
        }
    }

    /** Android didn't install it. */
    internal fun installFailed() {
        _state.update { s -> State.Failed((s as? State.Installing)?.update, Reason.Install) }
    }

    companion object {
        private const val TAG = "AppUpdater"
        private const val LATEST = "https://api.github.com/repos/thtbee/nectarlink/releases/latest"
        private const val LAST_CHECK = "lastCheck"
        private const val CHECK_EVERY_MS = 24L * 3600 * 1000
        private const val MAX_APK = 300L * 1024 * 1024

        private fun open(url: String): HttpURLConnection =
            (URL(url).openConnection() as HttpURLConnection).apply {
                connectTimeout = 10_000
                readTimeout = 60_000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "Nectarlink/${BuildConfig.VERSION_NAME} (Android)")
                setRequestProperty("Accept", "application/vnd.github+json, application/octet-stream")
                if (responseCode != 200) throw java.io.IOException("HTTP $responseCode")
            }

        private fun get(url: String, limit: Int): ByteArray =
            open(url).inputStream.use { input ->
                val bytes = input.readBytes()
                if (bytes.size > limit) throw java.io.IOException("too large")
                bytes
            }

        /** The SHA-256 a sums file lists for `name` (`<hex>  <name>` lines). */
        fun listedHash(sums: String, name: String): String? = sums.lineSequence().firstNotNullOfOrNull { line ->
            val parts = line.trim().split(Regex("\\s+"), limit = 2)
            val hash = parts.getOrNull(0)?.lowercase()
            val file = parts.getOrNull(1)?.trimStart('*')
            hash?.takeIf { file == name && it.length == 64 }
        }

        /** `1.2.3` style versions; a release is newer than its pre-releases. */
        fun isNewer(candidate: String, current: String): Boolean {
            fun parse(v: String): Pair<List<Int>, String?>? {
                val numbers = v.substringBefore('-').split('.').map { it.toIntOrNull() ?: return null }
                if (numbers.size !in 2..3) return null
                return (numbers + List(3 - numbers.size) { 0 }) to v.substringAfter('-', "").ifEmpty { null }
            }
            val (a, aPre) = parse(candidate) ?: return false
            val (b, bPre) = parse(current) ?: return false
            for (i in 0..2) if (a[i] != b[i]) return a[i] > b[i]
            return when {
                aPre == null && bPre != null -> true
                aPre != null && bPre != null -> aPre > bPre
                else -> false
            }
        }
    }
}
