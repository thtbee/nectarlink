// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.Manifest
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.KeyEvent
import android.view.WindowManager
import androidx.core.content.edit
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.calls.CallCompanion
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.TouchPhase
import io.github.muntashirakon.adb.AbsAdbConnectionManager
import io.github.muntashirakon.adb.AdbPairingRequiredException
import io.github.muntashirakon.adb.AdbStream
import io.github.muntashirakon.adb.android.AdbMdns
import kotlin.concurrent.thread
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.OutputStreamWriter
import java.io.Writer
import java.net.InetAddress
import java.security.SecureRandom

/**
 * The Elevated level (docs/PLAN.md §4.6) through the phone's own wireless
 * debugging, with nothing else to install: the user pairs NectarLink once
 * with a pairing code, and from then on NectarLink connects to the phone's
 * debugging service and runs [InputServer] with the shell's permissions,
 * for real touch and keys on the mirrored screen.
 *
 * Wireless debugging turns itself off on a reboot or a new Wi-Fi network;
 * after the first pairing NectarLink may turn it back on itself
 * (WRITE_SECURE_SETTINGS, which it grants itself through the shell).
 */
object Elevated {
    sealed interface State {
        /** Never set up. */
        data object NotSetUp : State
        /** Set up, but wireless debugging is off (or the phone isn't on Wi-Fi). */
        data object Waiting : State
        data object Starting : State
        data object Running : State
        /** Pairing or starting failed. */
        data class Failed(val reason: Reason) : State
    }

    enum class Reason { WrongCode, NoPairingDialog, NotPaired, RootDenied, Other }

    private const val TAG = "Elevated"
    private const val PREFS = "elevated"
    private const val PAIRED = "paired"
    /** Set up through root (the user chose Use root) rather than Wireless debugging. */
    private const val ROOT = "root"
    /** The shell user, which the helper runs as however it's started. */
    private const val SHELL_UID = "2000"
    /** The version [compileIfUpdated] last asked Android to compile. */
    private const val COMPILED_VERSION = "compiled_version"
    /** How long after the helper starts the compile waits, so Android has recorded which code starting up uses. */
    private const val COMPILE_AFTER_MS = 2 * 60_000L

    private val _state = MutableStateFlow<State>(State.NotSetUp)
    val state: StateFlow<State> = _state.asStateFlow()

    private lateinit var context: Context
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lock = Mutex()
    private var adb: Connection? = null
    private var shell: AdbStream? = null
    private var rootProcess: Process? = null
    @Volatile private var input: Writer? = null
    private var socket: java.net.Socket? = null
    /** Where the helper listens, and the token it wants, while it runs. */
    @Volatile private var helper: Pair<Int, String>? = null
    @Volatile private var screenOffRequested = false

    /** True when the running helper was started through root (`su`), not Wireless debugging. */
    @Volatile var rootActive: Boolean = false
        private set

    val running: Boolean get() = input != null

    private val rootAvailableCached: Boolean by lazy {
        val paths = arrayOf(
            "/system/bin/su",
            "/system/xbin/su",
            "/sbin/su",
            "/debug_ramdisk/su",
            "/data/adb/ksu/bin/su",
            "/data/adb/ap/bin/su",
        )
        paths.any { runCatching { java.io.File(it).exists() }.getOrDefault(false) }
    }

    /**
     * Whether a `su` binary (Magisk, KernelSU, APatch) is on the phone, so
     * Elevated may offer root as a way to start. Only looks for the file:
     * nothing asks for root until the user picks it.
     */
    fun isRootAvailable(): Boolean = rootAvailableCached

    /** Whether the user set Elevated up through root. */
    private fun usesRoot() = isRootAvailable() && prefs().getBoolean(ROOT, false)

    private fun prefs() = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /**
     * Sets Elevated up through root, as the user asked (instead of pairing
     * Wireless debugging). The root manager asks once; after that Elevated
     * starts by itself, also after a restart and away from Wi-Fi. The helper
     * still runs as the shell user, with the same permissions either way.
     */
    suspend fun setUpWithRoot(): Boolean = withContext(Dispatchers.IO) {
        if (!isRootAvailable()) return@withContext false
        val started = lock.withLock {
            if (running) return@withLock true
            _state.value = State.Starting
            tryStartWithRootLocked()
        }
        prefs().edit { putBoolean(ROOT, started) }
        if (started) {
            cancelRetry()
        } else {
            _state.value = State.Failed(Reason.RootDenied)
        }
        started
    }

    fun init(context: Context) {
        this.context = context.applicationContext
        watchWirelessDebugging()
        scope.launch(Dispatchers.IO) {
            if (paired() || usesRoot()) {
                _state.value = State.Waiting
                start()
            }
        }
    }

    // ---- Staying in sync ----
    //
    // Wireless debugging turns itself off when Wi-Fi drops or changes, and
    // comes back (NectarLink turns it on, or the user does) a moment after
    // the new network is up. A single attempt on the network change is
    // usually too early, so Elevated retries with a backoff while the phone
    // is on Wi-Fi, and starts right away when wireless debugging turns on.

    private var retryJob: Job? = null
    @Volatile private var retryAttempt = 0

    /**
     * Stopped on purpose (the connection service was turned off): retries and
     * network changes leave it stopped until something calls [start] again.
     */
    @Volatile private var held = false

    /** The network changed: try now, and keep trying from a short delay. */
    fun onNetworkChanged() {
        if (held) return
        retryAttempt = 0
        scope.launch { start() }
    }

    private fun watchWirelessDebugging() {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                val on = runCatching { Settings.Global.getInt(context.contentResolver, ADB_WIFI, 0) == 1 }.getOrDefault(false)
                if (on && !running) onNetworkChanged()
            }
        }
        runCatching {
            context.contentResolver.registerContentObserver(Settings.Global.getUriFor(ADB_WIFI), false, observer)
        }.onFailure { Log.i(TAG, "can't watch wireless debugging", it) }
    }

    /** Tries again later, while it can still work: set up, not running, on Wi-Fi (or through root). */
    private fun scheduleRetry() {
        if (held || !(paired() || usesRoot()) || running || BankingPause.isPaused(context)) return
        if (!usesRoot() && !onWifi()) return
        val wait = RETRY_MS[retryAttempt.coerceAtMost(RETRY_MS.lastIndex)]
        retryAttempt++
        retryJob?.cancel()
        retryJob = scope.launch {
            delay(wait)
            // So the attempt can schedule the next one without cancelling itself.
            retryJob = null
            if (!held) start()
        }
    }

    private fun cancelRetry() {
        retryJob?.cancel()
        retryJob = null
        retryAttempt = 0
    }

    private fun onWifi(): Boolean = runCatching {
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        connectivity.getNetworkCapabilities(connectivity.activeNetwork)
            ?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
    }.getOrDefault(false)

    /** Drops the connection to adbd; the next start makes a fresh one. */
    private fun discardConnection() {
        runCatching { adb?.close() }
        adb = null
    }

    private fun paired() = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(PAIRED, false)

    /** The phone's own adbd, as NectarLink. */
    private class Connection(context: Context) : AbsAdbConnectionManager() {
        private val key = AdbKey.load(context.noBackupFilesDir.resolve("adb"))

        init {
            setApi(Build.VERSION.SDK_INT)
            setHostAddress("127.0.0.1")
        }

        override fun getPrivateKey() = key.privateKey
        override fun getCertificate() = key.certificate
        override fun getDeviceName() = "Nectarlink"
    }

    // ---- Pairing ----

    /**
     * Pairs with the code wireless debugging shows ("Pair device with
     * pairing code"; the dialog must still be open), then starts.
     */
    suspend fun pair(code: String): Boolean = withContext(Dispatchers.IO) {
        lock.withLock {
            _state.value = State.Starting
            val paired = if (LoopbackAdb.preferred()) {
                pairOverLoopback(code.trim())
            } else {
                val port = discover(AdbMdns.SERVICE_TYPE_TLS_PAIRING)
                if (port == null) {
                    null
                } else {
                    runCatching { connection().pair("127.0.0.1", port, code.trim()) }.getOrElse {
                        Log.w(TAG, "pairing failed", it)
                        false
                    }
                }
            }
            if (paired == null) {
                _state.value = State.Failed(Reason.NoPairingDialog)
                return@withContext false
            }
            if (!paired) {
                _state.value = State.Failed(Reason.WrongCode)
                return@withContext false
            }
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(PAIRED, true) }
        }
        start()
    }

    /**
     * Connects to wireless debugging found on 127.0.0.1 (Android 17 and later,
     * see [LoopbackAdb]), waiting a moment for it to start listening.
     */
    private suspend fun connectOverLoopback(adb: Connection): Boolean {
        val deadline = System.currentTimeMillis() + CONNECT_MS
        while (true) {
            val port = LoopbackAdb.connectPort(LoopbackAdb.openPorts())
            if (port != null) return adb.connect(LoopbackAdb.HOST, port)
            if (System.currentTimeMillis() >= deadline) return false
            delay(LOOPBACK_RETRY_MS)
        }
    }

    /**
     * Pairs through the pairing port found on 127.0.0.1: any listening port
     * that isn't adbd's connection port. `null` when there is none (the
     * pairing dialog isn't open).
     */
    private suspend fun pairOverLoopback(code: String): Boolean? {
        val deadline = System.currentTimeMillis() + DISCOVERY_MS
        while (true) {
            val open = LoopbackAdb.openPorts()
            val connectPort = LoopbackAdb.connectPort(open)
            val candidates = open.filter { it != connectPort }
            for (port in candidates) {
                if (pairOnPort(port, code)) return true
            }
            if (candidates.isNotEmpty()) return false
            if (System.currentTimeMillis() >= deadline) return null
            delay(LOOPBACK_RETRY_MS)
        }
    }

    /** One pairing attempt, given up after [PAIR_MS] (a port that isn't adb's may never answer). */
    private suspend fun pairOnPort(port: Int, code: String): Boolean {
        val result = CompletableDeferred<Boolean>()
        thread(isDaemon = true, name = "adb-pair") {
            result.complete(
                runCatching { connection().pair(LoopbackAdb.HOST, port, code) }.getOrElse {
                    Log.i(TAG, "no pairing on port $port", it)
                    false
                },
            )
        }
        return withTimeoutOrNull(PAIR_MS) { result.await() } ?: false
    }

    /** The port a wireless debugging service is announced on, if it shows up soon. */
    private suspend fun discover(service: String): Int? {
        val found = CompletableDeferred<Int>()
        val mdns = AdbMdns(context, service) { _: InetAddress?, port: Int -> if (port > 0) found.complete(port) }
        mdns.start()
        return try {
            withTimeoutOrNull(DISCOVERY_MS) { found.await() }
        } finally {
            mdns.stop()
        }
    }

    private fun connection(): Connection = adb ?: Connection(context).also { adb = it }

    // ---- Starting ----

    /** Connects and starts the input helper (when set up; again after it stopped). */
    suspend fun start(): Boolean = withContext(Dispatchers.IO) {
        held = false
        // Paused for a banking app: debugging stays off until it's over
        // (BankingPause starts Elevated again then).
        if (BankingPause.isPaused(context)) {
            if (!running) _state.value = State.Waiting
            return@withContext false
        }
        val started = lock.withLock {
            if (running) return@withLock true
            _state.value = State.Starting
            if (usesRoot() && tryStartWithRootLocked()) {
                return@withLock true
            }
            if (!paired()) {
                // No wireless debugging set up: a PC this phone is plugged into can start it.
                if (startOverUsbLocked()) return@withLock true
                _state.value = State.NotSetUp
                return@withLock false
            }
            // A PC that already has this phone in its own adb (USB, or the
            // developer's wireless `adb connect`) starts the helper through that
            // connection first, so NectarLink doesn't hold a second debugging
            // session of its own next to it.
            if (startOverUsbLocked()) return@withLock true
            turnOnWirelessDebugging()
            // A connection from before is stale once the helper stopped
            // (wireless debugging went off with the old network): start fresh.
            discardConnection()
            val adb = connection()
            val connected = try {
                if (LoopbackAdb.preferred()) connectOverLoopback(adb) else adb.connectTls(context, CONNECT_MS)
            } catch (e: AdbPairingRequiredException) {
                // adbd forgot us (the user revoked the authorizations).
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(PAIRED, false) }
                _state.value = State.Failed(Reason.NotPaired)
                return@withLock false
            } catch (e: Exception) {
                Log.i(TAG, "wireless debugging isn't reachable", e)
                false
            }
            if (!connected) {
                discardConnection()
                if (startOverUsbLocked()) return@withLock true
                _state.value = State.Waiting
                return@withLock false
            }
            runCatching { launch(adb) }.onFailure {
                Log.w(TAG, "the input helper didn't start", it)
                stopLocked()
                discardConnection()
                _state.value = State.Failed(Reason.Other)
            }.isSuccess
        }
        when {
            started -> cancelRetry()
            _state.value.let { it == State.Waiting || it == State.Failed(Reason.Other) } -> scheduleRetry()
        }
        started
    }

    /**
     * Starts [InputServer] through `su` (Magisk, KernelSU, APatch), without
     * Wireless debugging. It runs as the shell user, as through Wireless
     * debugging: root only starts it. A root manager that can't switch to
     * that user fails here rather than run the helper as root.
     */
    private fun tryStartWithRootLocked(): Boolean {
        if (!isRootAvailable()) return false
        val uid = asShell("id -u")?.trim()
        if (uid != SHELL_UID) {
            Log.i(TAG, "root can't start the helper as the shell user (got ${uid ?: "nothing"})")
            return false
        }
        grants().forEach { asShell(it) }
        return runCatching {
            val token = ByteArray(24).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
            val apk = context.applicationInfo.sourceDir
            val cmd = "CLASSPATH=$apk app_process /system/bin ${InputServer::class.java.name} $token"
            val proc = ProcessBuilder("su", SHELL_UID, "-c", cmd).start()
            rootProcess = proc
            val output = proc.inputStream.bufferedReader()
            val first = output.readLine().orEmpty()
            val port = first.removePrefix("ready ").toIntOrNull()
            check(first.startsWith("ready ") && port != null) {
                "root helper said ${first.ifEmpty { "nothing" }}"
            }
            rootActive = true
            connectSocketLocked(port, token)
            Thread {
                runCatching { output.lineSequence().forEach { line -> Log.d(TAG, "root-helper: $line") } }
                input = null
                rootActive = false
                runCatching { socket?.close() }
                if (_state.value == State.Running) _state.value = State.Waiting
                onChange()
                scheduleRetry()
            }.start()
            true
        }.onFailure {
            Log.i(TAG, "the helper didn't start through root", it)
            runCatching { rootProcess?.destroyForcibly() }
            rootProcess = null
            rootActive = false
        }.getOrDefault(false)
    }

    /** Runs [command] as the shell user through `su`; its output, or null if it didn't run. */
    private fun asShell(command: String): String? = runCatching {
        val process = ProcessBuilder("su", SHELL_UID, "-c", command).redirectErrorStream(true).start()
        if (!process.waitFor(SU_WAIT_S, java.util.concurrent.TimeUnit.SECONDS)) {
            process.destroyForcibly()
            return null
        }
        process.inputStream.bufferedReader().readText().takeIf { process.exitValue() == 0 }
    }.getOrNull()

    /** Wireless debugging back on, when NectarLink may (granted at the first start). */
    private fun turnOnWirelessDebugging() {
        if (context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) != PackageManager.PERMISSION_GRANTED) return
        runCatching {
            if (Settings.Global.getInt(context.contentResolver, ADB_WIFI, 0) != 1) {
                Settings.Global.putInt(context.contentResolver, ADB_WIFI, 1)
                Thread.sleep(1500) // adbd announces itself.
            }
        }.onFailure { Log.i(TAG, "can't turn on wireless debugging", it) }
    }

    /** What the shell grants NectarLink once, before the helper starts (however it starts). */
    private fun grants(): List<String> = buildList {
        val pkg = context.packageName
        // Turning wireless debugging back on later is NectarLink's to do.
        if (context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) != PackageManager.PERMISSION_GRANTED) {
            add("pm grant $pkg ${Manifest.permission.WRITE_SECURE_SETTINGS}")
        }
        // Lets Telecom bind the calling companion: mute, speaker, hold and
        // the keypad from a PC.
        if (!CallCompanion.allowed(context) && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            add("appops set $pkg ${CallCompanion.OP_NAME} allow")
        }
        // Lets the notification listener read one-time codes in notifications
        // Android 15+ marks sensitive, and refreshes the listener registration
        // so NotificationManagerService updates its trusted-listener UID set.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            if (!NotificationListener.canReadSensitive(context)) {
                add("appops set $pkg ${NotificationListener.OP_SENSITIVE_NAME} allow")
            }
            if (NotificationListener.hasAccess(context)) {
                add("cmd notification allow_listener $pkg/${NotificationListener::class.java.name}")
            }
        }
        // Lets PCs toggle Do Not Disturb / silent mode and screen brightness without extra prompts.
        if (context.getSystemService(android.app.NotificationManager::class.java)?.isNotificationPolicyAccessGranted == false) {
            add("cmd notification allow_dnd $pkg")
        }
        if (!Settings.System.canWrite(context)) {
            add("appops set $pkg WRITE_SETTINGS allow")
        }
        // Pause for banking apps sees which app is in front; only for those using it.
        if (BankingPause.apps(context).isNotEmpty() && !BankingPause.hasUsageAccess(context)) {
            add("appops set $pkg GET_USAGE_STATS allow")
        }
    }

    private fun launch(adb: Connection) {
        grants().forEach { run(adb, it) }
        val token = ByteArray(24).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
        val apk = context.applicationInfo.sourceDir
        val command = "CLASSPATH=$apk app_process /system/bin ${InputServer::class.java.name} $token"
        val stream = adb.openStream("shell:$command")
        shell = stream
        // The helper says "ready <port>" once it listens on loopback.
        val output = stream.openInputStream().bufferedReader()
        val first = output.readLine().orEmpty()
        val port = first.removePrefix("ready ").toIntOrNull()
        check(first.startsWith("ready ") && port != null) { "the helper said ${first.ifEmpty { "nothing" }}" }
        connectSocketLocked(port, token)
        // Watches the helper: when its shell ends, Elevated stops.
        Thread {
            runCatching { output.lineSequence().forEach { line -> Log.d(TAG, "helper: $line") } }
            input = null
            runCatching { socket?.close() }
            if (_state.value == State.Running) _state.value = if (paired()) State.Waiting else State.NotSetUp
            onChange()
            scheduleRetry()
        }.start()
    }

    private fun connectSocketLocked(port: Int, token: String) {
        helper = port to token
        val local = java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply { tcpNoDelay = true }
        socket = local
        val writer = OutputStreamWriter(local.outputStream, Charsets.UTF_8)
        writer.write("$token\nH\n")
        writer.flush()
        input = writer
        _state.value = State.Running
        if (screenOffRequested) send(writer, "P 0")
        compileIfUpdated()
        Thread {
            runCatching {
                local.inputStream.bufferedReader().lineSequence().forEach { line ->
                    if (line.startsWith("CLIP\t")) {
                        val parts = line.split('\t')
                        val b64Text = parts.getOrNull(1).orEmpty()
                        val b64Html = parts.getOrNull(2).orEmpty()
                        val text = runCatching {
                            String(android.util.Base64.decode(b64Text, android.util.Base64.DEFAULT), Charsets.UTF_8)
                        }.getOrDefault("")
                        val html = if (b64Html.isNotEmpty()) {
                            runCatching {
                                String(android.util.Base64.decode(b64Html, android.util.Base64.DEFAULT), Charsets.UTF_8)
                            }.getOrNull()?.takeIf(String::isNotBlank)
                        } else {
                            null
                        }
                        if (text.isNotBlank()) {
                            onClipboardChanged(text, html)
                        }
                    }
                }
            }
            if (socket === local) {
                input = null
                runCatching { local.close() }
                if (_state.value == State.Running) _state.value = if (paired()) State.Waiting else State.NotSetUp
                onChange()
                scheduleRetry()
            }
        }.start()
        onChange()
    }

    private var compileJob: Job? = null

    /**
     * A sideloaded app starts slowly after each install or update, about twice
     * as slowly, until Android compiles it during idle maintenance, often days
     * later. With the helper running, ask for that compile now, once per version.
     */
    private fun compileIfUpdated() {
        val version = app.nectarlink.android.BuildConfig.VERSION_CODE
        if (app.nectarlink.android.BuildConfig.DEBUG || compileJob?.isActive == true) return
        if (prefs().getInt(COMPILED_VERSION, -1) == version) return
        compileJob = scope.launch {
            delay(COMPILE_AFTER_MS)
            if (runCommand(COMPILE_APP_COMMAND)) prefs().edit { putInt(COMPILED_VERSION, version) }
        }
    }

    /** Attaches to an [InputServer] started directly from `adb shell`. */
    suspend fun attachFromShell(appContext: Context, port: Int, token: String): Boolean = withContext(Dispatchers.IO) {
        if (!::context.isInitialized) init(appContext)
        lock.withLock {
            runCatching { socket?.close() }
            runCatching { connectSocketLocked(port, token) }.onFailure {
                Log.w(TAG, "couldn't attach to shell helper", it)
            }.isSuccess
        }
    }

    /** Receives the loopback port and token from an [InputServer] started via `adb shell`. */
    class AttachReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val port = intent.getIntExtra("port", 0)
            val token = intent.getStringExtra("token").orEmpty()
            if (port <= 0 || token.isEmpty()) return
            val pending = goAsync()
            val app = context.applicationContext
            scope.launch {
                try {
                    attachFromShell(app, port, token)
                } finally {
                    pending.finish()
                }
            }
        }
    }

    /** Runs a shell command to its end; returns what it printed. */
    private fun run(adb: Connection, command: String): String {
        val out = java.io.ByteArrayOutputStream()
        adb.openStream("shell:$command").use { stream ->
            val input = stream.openInputStream()
            val buffer = ByteArray(4096)
            // The library reports the end of a command's output as "Stream closed".
            try {
                while (true) {
                    val n = input.read(buffer)
                    if (n < 0) break
                    out.write(buffer, 0, n)
                }
            } catch (_: java.io.IOException) {
            }
        }
        return out.toString(Charsets.UTF_8.name())
    }

    /**
     * Starts through a PC this phone is plugged into with USB debugging allowed
     * (`docs/protocol/elevated-usb.md`), when Elevated isn't running. Called when
     * a PC connects, so plugging in is enough.
     */
    suspend fun startOverUsb(): Boolean = withContext(Dispatchers.IO) {
        if (held || running || BankingPause.isPaused(context)) return@withContext running
        lock.withLock { running || startOverUsbLocked() }
    }

    /**
     * Asks the PC this phone is plugged into to run the grants and start the
     * helper over USB, then connects to it. The helper runs until the cable
     * comes out (its socket then closes, see [connectSocketLocked]).
     */
    private suspend fun startOverUsbLocked(): Boolean {
        val app = context.applicationContext as? app.nectarlink.android.NectarlinkApplication ?: return false
        val token = ByteArray(24).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
        val port = app.core.elevatedUsbStart(context.applicationInfo.sourceDir, token, grants()) ?: return false
        return runCatching {
            connectSocketLocked(port, token)
            Log.i(TAG, "started over USB")
            cancelRetry()
            true
        }.getOrElse {
            Log.w(TAG, "the helper started over USB didn't answer", it)
            false
        }
    }

    /** Stops the helper and disconnects, and keeps it stopped until [start]. */
    suspend fun stop() = withContext(Dispatchers.IO) {
        held = true
        cancelRetry()
        lock.withLock { stopLocked() }
    }

    private fun stopLocked() {
        screenOffRequested = false
        rootActive = false
        input = null
        helper = null
        runCatching { socket?.close() }
        runCatching { shell?.close() }
        runCatching { rootProcess?.destroyForcibly() }
        runCatching { adb?.disconnect() }
        socket = null
        shell = null
        rootProcess = null
        _state.value = if (paired() || usesRoot()) State.Waiting else State.NotSetUp
        onChange()
    }

    /** Forgets the pairing (the user turned Elevated off). */
    suspend fun forget() {
        cancelRetry()
        screenOffRequested = false
        stop()
        prefs().edit {
            putBoolean(PAIRED, false)
            putBoolean(ROOT, false)
        }
        runCatching { adb?.close() }
        adb = null
        AdbKey.delete(context.noBackupFilesDir.resolve("adb"))
        _state.value = State.NotSetUp
        onChange()
    }

    /** Told when Elevated starts or stops (what the phone offers changes). */
    @Volatile var onChange: () -> Unit = {}

    /** Told when the shell helper observes a primary clipboard change in the background: `(text, html)`. */
    @Volatile var onClipboardChanged: (String, String?) -> Unit = { _, _ -> }

    // ---- Input ----

    /** The PC's input as real events; false if Elevated isn't running. */
    fun handle(event: MirrorInputEvent): Boolean {
        val (w, h) = screen()
        return handleOn(null, w, h, event)
    }

    /**
     * The PC's input on display [display] (an app window's, [w] × [h]
     * pixels), or the screen when null.
     */
    fun handleOn(display: Int?, w: Float, h: Float, event: MirrorInputEvent): Boolean {
        val writer = input ?: return false
        val to = display?.let { "@$it " }.orEmpty()
        val line = to + when (event) {
            is MirrorInputEvent.Touch -> {
                val action = when (event.action) {
                    TouchPhase.DOWN -> "D"
                    TouchPhase.MOVE -> "M"
                    TouchPhase.UP -> "U"
                }
                "$action ${event.x * w} ${event.y * h}"
            }
            is MirrorInputEvent.Scroll -> "S ${event.x * w} ${event.y * h} ${event.dx} ${event.dy}"
            is MirrorInputEvent.Key -> keys[event.key]?.let { "K $it" }
                ?: if (event.key == "notifications" && display == null) "C cmd statusbar expand-notifications" else return true
            // A key typed on the PC types; anything longer (a paste, a word from
            // an input method, an emoji) goes on the clipboard, quietly, and is
            // pasted at once: typing it out took a moment per letter, and a line
            // break typed as Enter sends a chat message early.
            is MirrorInputEvent.Text -> return when {
                event.text.isEmpty() -> true
                event.text == "\n" || event.text == "\r" || event.text == "\r\n" ->
                    send(writer, "${to}K ${KeyEvent.KEYCODE_ENTER}")
                event.text.length == 1 && event.text[0].code in 32..126 -> send(writer, "${to}T ${event.text}")
                // The clipboard is the phone's own, whichever display the text is for.
                else -> setClipboard(event.text, null) && send(writer, "${to}K ${KeyEvent.KEYCODE_PASTE}")
            }
            is MirrorInputEvent.Gamepad ->
                "G ${event.buttons} ${event.lx} ${event.ly} ${event.rx} ${event.ry} ${event.lt} ${event.rt}"
        }
        return send(writer, line)
    }

    /** Sends a gamepad state directly through the Elevated helper. */
    fun sendGamepad(buttons: Int, lx: Int, ly: Int, rx: Int, ry: Int, lt: Int, rt: Int): Boolean {
        val writer = input ?: return false
        return send(writer, "G $buttons $lx $ly $rx $ry $lt $rt")
    }

    /**
     * Opens a 48 kHz 16-bit stereo PCM audio stream from the Elevated helper
     * (`InputServer` command `"A"`). Returns null if Elevated isn't running.
     */
    fun openAudioStream(): java.net.Socket? {
        val (port, token) = helper ?: return null
        return runCatching {
            java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply {
                tcpNoDelay = true
                val out = OutputStreamWriter(getOutputStream(), Charsets.UTF_8)
                out.write("$token\nA\n")
                out.flush()
            }
        }.onFailure { Log.w(TAG, "can't open Elevated audio stream", it) }.getOrNull()
    }

    /** Lets NectarLink see which app is in front (for Pause for banking apps); false if Elevated isn't running. */
    fun grantUsageAccess(): Boolean = runCommand("appops set ${context.packageName} GET_USAGE_STATS allow")

    /** Runs an allowed shell command through the Elevated helper. */
    fun runCommand(command: String): Boolean {
        val writer = input ?: return false
        return send(writer, "C $command")
    }

    /**
     * Puts a PC's text on the clipboard through the helper, so the phone
     * doesn't pop up "copied" for a copy made on the PC. False when the
     * helper isn't running (the app writes it then).
     */
    fun setClipboard(text: String, html: String?): Boolean {
        val writer = input ?: return false
        val b64 = { value: String -> android.util.Base64.encodeToString(value.toByteArray(Charsets.UTF_8), android.util.Base64.NO_WRAP) }
        return send(writer, "B ${b64(text)} ${html?.let(b64).orEmpty()}")
    }

    /**
     * Wakes the screen when it's off, as the power key would (the lock screen
     * shows if the phone is locked). Mirroring otherwise waits for someone to
     * turn the screen on by hand (#82).
     */
    fun wakeScreen(): Boolean {
        val writer = input ?: return false
        val power = context.getSystemService(android.os.PowerManager::class.java)
        if (power?.isInteractive != false) return true
        return send(writer, "K ${KeyEvent.KEYCODE_WAKEUP}")
    }

    /** Turns the physical screen panel off or back on while mirroring. */
    fun setScreenOff(off: Boolean): Boolean {
        screenOffRequested = off
        val writer = input ?: return false
        return send(writer, "P ${if (off) 0 else 1}")
    }

    /**
     * Opens an app window: the helper runs [pkg] on a display of its own of
     * [width] × [height] pixels at [dpi], and streams it on the returned
     * connection (packets as [AppDisplay] writes them). Null when Elevated
     * isn't running. Write "K\n" for a keyframe; close it to close the window.
     */
    fun openApp(
        pkg: String,
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): java.net.Socket? {
        if (!PACKAGE.matches(pkg)) return null
        val (port, token) = helper ?: return null
        return runCatching {
            java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply {
                tcpNoDelay = true
                val out = OutputStreamWriter(getOutputStream(), Charsets.UTF_8)
                out.write("$token\nV $width $height $dpi $bitrate $fps $pkg $codec\n")
                out.flush()
            }
        }.onFailure { Log.w(TAG, "can't open an app window", it) }.getOrNull()
    }

    /**
     * Opens a full-screen mirror stream (`pkg == "-"`) via the Elevated shell
     * helper without requiring a MediaProjection consent prompt on the phone.
     */
    fun openScreen(
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): java.net.Socket? {
        val (port, token) = helper ?: return null
        return runCatching {
            java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply {
                tcpNoDelay = true
                val out = OutputStreamWriter(getOutputStream(), Charsets.UTF_8)
                out.write("$token\nV $width $height $dpi $bitrate $fps - $codec\n")
                out.flush()
            }
        }.onFailure { Log.w(TAG, "can't open Elevated screen mirror", it) }.getOrNull()
    }

    /**
     * Writes [line] to the helper. Android doesn't allow socket writes on
     * the main thread (they throw, which read as the helper having gone);
     * a line sent from there goes out on [offMain], in order.
     */
    private fun send(writer: Writer, line: String): Boolean {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            offMain.execute { sendNow(writer, line) }
            return true
        }
        return sendNow(writer, line)
    }

    private val offMain = java.util.concurrent.Executors.newSingleThreadExecutor { r ->
        Thread(r, "elevated-send").apply { isDaemon = true }
    }

    private fun sendNow(writer: Writer, line: String): Boolean = try {
        require('\r' !in line && '\n' !in line)
        synchronized(writer) {
            writer.write(line + "\n")
            writer.flush()
        }
        true
    } catch (e: Exception) {
        Log.i(TAG, "the input helper went away", e)
        input = null
        false
    }

    private val PACKAGE = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

    private val keys = mapOf(
        "back" to KeyEvent.KEYCODE_BACK,
        "home" to KeyEvent.KEYCODE_HOME,
        "recents" to KeyEvent.KEYCODE_APP_SWITCH,
        "enter" to KeyEvent.KEYCODE_ENTER,
        "backspace" to KeyEvent.KEYCODE_DEL,
        "delete" to KeyEvent.KEYCODE_FORWARD_DEL,
        "left" to KeyEvent.KEYCODE_DPAD_LEFT,
        "right" to KeyEvent.KEYCODE_DPAD_RIGHT,
        "up" to KeyEvent.KEYCODE_DPAD_UP,
        "down" to KeyEvent.KEYCODE_DPAD_DOWN,
        "tab" to KeyEvent.KEYCODE_TAB,
    )

    /** The screen's size in pixels, as it's turned now. */
    private fun screen(): Pair<Float, Float> {
        val windows = context.getSystemService(WindowManager::class.java)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windows.maximumWindowMetrics.bounds.let { it.width().toFloat() to it.height().toFloat() }
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windows.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels.toFloat() to metrics.heightPixels.toFloat()
        }
    }

    private const val ADB_WIFI = "adb_wifi_enabled"
    private const val DISCOVERY_MS = 10_000L
    private const val LOOPBACK_RETRY_MS = 700L
    private const val PAIR_MS = 4_000L
    private const val CONNECT_MS = 10_000L
    /** How long a `su` command may take; the first one waits for the root manager's prompt. */
    private const val SU_WAIT_S = 30L
    /** Waits between attempts to start again; the last one repeats. */
    private val RETRY_MS = longArrayOf(2_000L, 5_000L, 10_000L, 20_000L, 40_000L, 60_000L)
}
