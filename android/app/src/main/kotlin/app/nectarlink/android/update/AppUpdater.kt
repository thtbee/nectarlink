// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.core.net.toUri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates from GitHub releases: checks the latest release, and installs a
 * newer signed APK after checking it against the release's SHA-256 sums.
 * Android only installs it over this app when it's signed with the same
 * key, so only release builds update themselves (debug builds are signed
 * differently).
 *
 * Downloads resume where they stopped: across a dropped or changed network
 * (it waits for one to come back), and across restarts of the app.
 */
@androidx.compose.runtime.Stable
class AppUpdater(context: Context) {
    private val context = context.applicationContext
    internal val prefs = context.getSharedPreferences("updates", Context.MODE_PRIVATE)

    /** Installs outlive the screen that started them. */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val busy = Mutex()

    /** A newer release. */
    @androidx.compose.runtime.Immutable
    data class Update(val version: String, val notesUrl: String, internal val apkUrl: String, internal val apkName: String, internal val sumsUrl: String)

    sealed interface State {
        data object Idle : State
        data object Checking : State

        /** A check found nothing newer. */
        data object UpToDate : State
        data class Available(val update: Update) : State

        /** Waiting for the user to let NectarLink install apps; continues when they have. */
        data class NeedsPermission(val update: Update) : State

        /** `total` is 0 until the size is known. */
        data class Downloading(val update: Update, val done: Long, val total: Long, val waitingForNetwork: Boolean) : State

        /** Waiting for Android (and the user) to install it. */
        data class Installing(val update: Update) : State
        data class Failed(val update: Update?, val reason: Reason) : State
    }

    enum class Reason { Offline, Damaged, Install }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    /** Whether this build updates itself. */
    val enabled: Boolean get() = !BuildConfig.DEBUG

    private val _autoInstall = MutableStateFlow(prefs.getBoolean(AUTO_INSTALL, true))

    /** Whether a newer release installs by itself, without asking (Settings). */
    val autoInstall: StateFlow<Boolean> = _autoInstall.asStateFlow()

    fun setAutoInstall(on: Boolean) {
        prefs.edit { putBoolean(AUTO_INSTALL, on) }
        _autoInstall.value = on
    }

    /**
     * For the background: checks when due and, with automatic updates on, installs a newer
     * release. Android 12 and later install it without asking once NectarLink has installed
     * itself before; otherwise a notification asks for one tap.
     */
    suspend fun updateIfDue() {
        if (!enabled || isBusy()) return
        checkIfDue()
        val available = (_state.value as? State.Available)?.update ?: return
        if (_autoInstall.value && canInstall()) install(available, automatic = true)
    }

    /** Checks at most once a day (call when the app comes to the front). */
    suspend fun checkIfDue() {
        val last = prefs.getLong(LAST_CHECK, 0)
        if (System.currentTimeMillis() - last < CHECK_EVERY_MS) return
        check()
    }

    /** Checks now; returns whether an update is available. Leaves an install in progress alone. */
    suspend fun check(): Boolean {
        if (!enabled || isBusy()) return _state.value is State.Available
        _state.value = State.Checking
        val found = withContext(Dispatchers.IO) {
            runCatching { latest() }.onFailure { Log.i(TAG, "couldn't check for updates", it) }
        }
        prefs.edit { putLong(LAST_CHECK, System.currentTimeMillis()) }
        _state.value = found.fold(
            onSuccess = { update -> update?.let { State.Available(it) } ?: State.UpToDate },
            onFailure = { State.Failed(null, Reason.Offline) },
        )
        return _state.value is State.Available
    }

    /** Checks now, from a button. */
    fun checkNow() {
        scope.launch { check() }
    }

    private fun isBusy(): Boolean = _state.value.let {
        it is State.Downloading || it is State.Installing || it is State.Checking
    }

    private fun latest(): Update? {
        val body = try {
            get(LATEST, 1 shl 20)
        } catch (e: HttpStatus) {
            // GitHub answers 404 until the first release is published: nothing newer.
            if (e.code == 404) return null
            throw e
        }
        val release = JSONObject(String(body))
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

    /** Android's screen where the user lets NectarLink install updates. */
    fun allowInstallsIntent(): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, "package:${context.packageName}".toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /**
     * The user tapped Update. Starts the install, or, when Android first needs
     * the user to allow installs, returns the screen for that; the update then
     * continues by itself once they come back ([onAppResumed]).
     */
    fun requestInstall(update: Update): Intent? {
        if (!canInstall()) {
            _state.value = State.NeedsPermission(update)
            return allowInstallsIntent()
        }
        scope.launch { install(update) }
        return null
    }

    /** The app is in front again: carries on an update the user was allowing. */
    fun onAppResumed() {
        val waiting = (_state.value as? State.NeedsPermission)?.update ?: return
        if (canInstall()) scope.launch { install(waiting) }
    }

    /** Downloads, checks and hands the update to Android's installer. */
    suspend fun install(update: Update, automatic: Boolean = false) {
        if (!busy.tryLock()) return
        try {
            prefs.edit { putBoolean(LAST_AUTOMATIC, automatic) }
            if (!canInstall()) {
                _state.value = State.NeedsPermission(update)
                return
            }
            _state.value = State.Downloading(update, 0, 0, waitingForNetwork = false)
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
        } finally {
            busy.unlock()
        }
    }

    private class DamagedException : Exception("checksum mismatch")

    /**
     * The verified APK. Kept under a name taken from its expected hash, so a
     * partial download only ever continues into the same file.
     */
    private suspend fun download(update: Update): File {
        val sums = retrying(update) { String(get(update.sumsUrl, 64 * 1024)) }
        val expected = listedHash(sums, update.apkName) ?: throw DamagedException()
        val dir = File(context.cacheDir, UPDATES_DIR).apply { mkdirs() }
        val name = "nectarlink-${expected.take(16)}.apk"
        val apk = File(dir, name)
        val part = File(dir, "$name.part")
        dir.listFiles()?.filter { it != apk && it != part }?.forEach { it.deleteRecursively() }
        if (apk.isFile && sha256(apk) == expected) return apk
        apk.delete()

        retrying(update) { fetchInto(update, part) }
        if (sha256(part) != expected) {
            part.delete()
            throw DamagedException()
        }
        if (!part.renameTo(apk)) throw IOException("can't keep the download")
        return apk
    }

    /**
     * Runs [block], and after a network error waits for a connection and
     * tries again. Gives up after [MAX_ATTEMPTS] tries in a row that got
     * nowhere; any progress resets the count.
     */
    private suspend fun <T> retrying(update: Update, block: suspend () -> T): T {
        var failures = 0
        var lastDone = downloaded()
        while (true) {
            try {
                return block()
            } catch (e: DamagedException) {
                throw e
            } catch (e: HttpStatus) {
                if (e.code in 400..499 && e.code != 408 && e.code != 429) throw e
                failures = nextFailure(failures, lastDone).also { lastDone = downloaded() }
                if (failures > MAX_ATTEMPTS) throw e
            } catch (e: IOException) {
                failures = nextFailure(failures, lastDone).also { lastDone = downloaded() }
                if (failures > MAX_ATTEMPTS) throw e
            }
            setWaiting(update, true)
            awaitNetwork()
            delay(BACKOFF_MS[(failures - 1).coerceIn(0, BACKOFF_MS.lastIndex)])
            setWaiting(update, false)
        }
    }

    private fun downloaded(): Long = (_state.value as? State.Downloading)?.done ?: 0

    private fun nextFailure(failures: Int, lastDone: Long): Int = if (downloaded() > lastDone) 1 else failures + 1

    private fun setWaiting(update: Update, waiting: Boolean) {
        _state.update { s ->
            if (s is State.Downloading && s.update == update) s.copy(waitingForNetwork = waiting) else s
        }
    }

    /** Waits (up to two minutes) for a network that reaches the internet. */
    private suspend fun awaitNetwork() {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return
        val deadline = SystemClock.elapsedRealtime() + NETWORK_WAIT_MS
        while (SystemClock.elapsedRealtime() < deadline) {
            val caps = manager.getNetworkCapabilities(manager.activeNetwork)
            if (caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) == true) return
            delay(1_000)
        }
    }

    /** Downloads into `part`, continuing from what it already holds. */
    private fun fetchInto(update: Update, part: File) {
        val have = part.length()
        val connection = connect(update.apkUrl, from = have)
        try {
            val append = when (val code = connection.responseCode) {
                HttpURLConnection.HTTP_PARTIAL -> true
                HttpURLConnection.HTTP_OK -> false
                // Asked for bytes past the end: it's all here already.
                416 -> return
                else -> throw HttpStatus(code)
            }
            val total = if (append) {
                rangeTotal(connection.getHeaderField("Content-Range")) ?: (have + connection.contentLengthLong)
            } else {
                connection.contentLengthLong
            }
            if (total > MAX_APK) throw DamagedException()
            var done = if (append) have else 0L
            var lastReport = 0L
            report(update, done, total)
            FileOutputStream(part, append).use { output ->
                connection.inputStream.use { input ->
                    val buffer = ByteArray(256 * 1024)
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        done += read
                        if (done > MAX_APK) throw DamagedException()
                        output.write(buffer, 0, read)
                        val now = SystemClock.elapsedRealtime()
                        if (now - lastReport > PROGRESS_EVERY_MS) {
                            lastReport = now
                            report(update, done, total)
                        }
                    }
                }
            }
            report(update, done, total)
            if (total > 0 && done < total) throw IOException("download ended early")
        } finally {
            connection.disconnect()
        }
    }

    private fun report(update: Update, done: Long, total: Long) {
        _state.value = State.Downloading(update, done, maxOf(total, 0), waitingForNetwork = false)
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
            val updater = (context.applicationContext as NectarlinkApplication).updater
            when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
                PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                    val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                    } else {
                        @Suppress("DEPRECATION")
                        intent.getParcelableExtra(Intent.EXTRA_INTENT)
                    }?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) ?: return
                    // Started by the user: Android's prompt opens now. In the background it
                    // can't, so a notification waits for the tap.
                    if (updater.prefs.getBoolean(LAST_AUTOMATIC, false)) {
                        updater.notifyReady(confirm)
                    } else {
                        runCatching { context.startActivity(confirm) }.onFailure { updater.notifyReady(confirm) }
                    }
                }
                // This process is replaced; the new version says it's updated (onUpdated).
                PackageInstaller.STATUS_SUCCESS -> Unit
                // The user said no to Android's prompt: the update is still there to install.
                PackageInstaller.STATUS_FAILURE_ABORTED -> updater.installCancelled()
                else -> {
                    Log.w(TAG, "install failed: ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}")
                    updater.installFailed()
                }
            }
        }
    }

    private fun channel(manager: NotificationManager) {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_updates), NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    private fun mayNotify(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    /** "NectarLink 0.2.0 is ready", opening Android's install prompt when tapped. */
    internal fun notifyReady(confirm: Intent) {
        if (!mayNotify()) return
        val manager = context.getSystemService(NotificationManager::class.java)
        channel(manager)
        val version = (_state.value as? State.Installing)?.update?.version.orEmpty()
        val tap = PendingIntent.getActivity(context, READY_ID, confirm, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.update_ready_title, version))
            .setContentText(context.getString(R.string.update_ready_text))
            .setContentIntent(tap)
            .setAutoCancel(true)
            .build()
        manager.notify(READY_ID, notification)
    }

    /**
     * The new version's first run after an update (StartupReceiver): clears
     * the download and says so. Android doesn't let an app open its own window
     * from the background, so the notification opens it.
     */
    fun onUpdated() {
        runCatching { File(context.cacheDir, UPDATES_DIR).deleteRecursively() }
        if (!mayNotify()) return
        val manager = context.getSystemService(NotificationManager::class.java)
        channel(manager)
        manager.cancel(READY_ID)
        val launch = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return
        val tap = PendingIntent.getActivity(
            context,
            UPDATED_ID,
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.update_done_title, BuildConfig.VERSION_NAME))
            .setContentText(context.getString(R.string.update_done_text))
            .setContentIntent(tap)
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .build()
        manager.notify(UPDATED_ID, notification)
    }

    /** Android didn't install it. */
    internal fun installFailed() {
        _state.update { s -> State.Failed((s as? State.Installing)?.update, Reason.Install) }
    }

    internal fun installCancelled() {
        _state.update { s -> (s as? State.Installing)?.update?.let { State.Available(it) } ?: s }
    }

    companion object {
        private const val TAG = "AppUpdater"
        private const val LATEST = "https://api.github.com/repos/alejandrokundrah/nectarlink/releases/latest"
        private const val LAST_CHECK = "lastCheck"
        private const val AUTO_INSTALL = "autoInstall"
        private const val LAST_AUTOMATIC = "lastAutomatic"
        private const val UPDATES_DIR = "updates"
        private const val CHANNEL = "updates"
        private const val READY_ID = 0x0D47
        private const val UPDATED_ID = 0x0D48
        private const val CHECK_EVERY_MS = 24L * 3600 * 1000
        private const val MAX_APK = 300L * 1024 * 1024
        private const val MAX_ATTEMPTS = 6
        private const val NETWORK_WAIT_MS = 2L * 60 * 1000
        private const val PROGRESS_EVERY_MS = 150L
        private val BACKOFF_MS = longArrayOf(1_000, 2_000, 4_000, 8_000, 15_000, 30_000)

        /** A response other than the one expected. */
        private class HttpStatus(val code: Int) : IOException("HTTP $code")

        /** A connection for `url`, asking for the bytes from `from` on when that's past 0. */
        private fun connect(url: String, from: Long = 0): HttpURLConnection {
            if (!url.startsWith("https://")) throw IOException("only https")
            return (URL(url).openConnection() as HttpURLConnection).apply {
                connectTimeout = 15_000
                // A stalled connection fails soon, so the download resumes on a fresh one.
                readTimeout = 20_000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "Nectarlink/${BuildConfig.VERSION_NAME} (Android)")
                setRequestProperty("Accept", "application/vnd.github+json, application/octet-stream")
                if (from > 0) setRequestProperty("Range", "bytes=$from-")
                // Redirects stay on https (HttpURLConnection never follows one to another
                // protocol); checked again in case that ever changes.
                responseCode
                if (this.url.protocol != "https") throw IOException("only https")
            }
        }

        /** The body, up to `limit` bytes; never buffers more than that. */
        private fun get(url: String, limit: Int): ByteArray {
            val connection = connect(url)
            try {
                if (connection.responseCode != HttpURLConnection.HTTP_OK) throw HttpStatus(connection.responseCode)
                return connection.inputStream.use { input ->
                    val out = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(16 * 1024)
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        if (out.size() + read > limit) throw IOException("too large")
                        out.write(buffer, 0, read)
                    }
                    out.toByteArray()
                }
            } finally {
                connection.disconnect()
            }
        }

        private fun sha256(file: File): String {
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                val buffer = ByteArray(256 * 1024)
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    digest.update(buffer, 0, read)
                }
            }
            return digest.digest().joinToString("") { "%02x".format(it) }
        }

        /** The full size from a `Content-Range: bytes 100-199/200` header. */
        fun rangeTotal(header: String?): Long? =
            header?.substringAfterLast('/', "")?.trim()?.toLongOrNull()?.takeIf { it > 0 }

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
