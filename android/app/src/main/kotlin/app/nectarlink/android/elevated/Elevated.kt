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
import app.nectarlink.services.ServiceStatus
import app.nectarlink.services.client.NectarlinkServiceClient
import app.nectarlink.services.client.NectarlinkServiceContract
import io.github.muntashirakon.adb.AbsAdbConnectionManager
import io.github.muntashirakon.adb.AdbPairingRequiredException
import io.github.muntashirakon.adb.AdbStream
import io.github.muntashirakon.adb.android.AdbMdns
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

    enum class Reason { WrongCode, NoPairingDialog, NotPaired, RootDenied, ShizukuNotRunning, ShizukuDenied, ShizukuRoot, Other }

    private const val TAG = "Elevated"

    /** How long the helper may take to set up a call's sound. */
    private const val CALL_AUDIO_ANSWER_MS = 3_000
    /** How long the helper has to prove it's the one we started ([HelperProof]). */
    private const val PROOF_MS = 3_000
    private const val PREFS = "elevated"
    private const val PAIRED = "paired"
    /** Set up through root (the user chose Use root) rather than Wireless debugging. */
    private const val ROOT = "root"
    /** Set up through Shizuku (the user chose Use Shizuku), see [ShizukuElevated]. */
    private const val SHIZUKU = "shizuku"
    /** The shell user, which the helper runs as however it's started. */
    private const val SHELL_UID = "2000"
    /**
     * The shell user's groups as adbd gives them: input, log, adb, storage,
     * Bluetooth, inet (sockets), network stats, readproc, uhid (gamepads)
     * and readtracefs.
     */
    private val SHELL_GROUPS = listOf(1004, 1007, 1011, 1015, 1028, 1078, 1079, 3001, 3002, 3003, 3006, 3009, 3011, 3012)
    /** Android lets only this group open sockets, the helper's included. */
    private const val INET_GROUP = "3003"
    /** The version [compileIfUpdated] last asked Android to compile. */
    private const val COMPILED_VERSION = "compiled_version"
    /** Whether NectarLink turns Wireless debugging back on by itself (on unless turned off). */
    private const val TURN_ON_WIRELESS = "turn_on_wireless_debugging"
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

    /**
     * Whether the helper watches the clipboard: only while some device takes
     * this phone's copies by itself (#655). It's on until the app says.
     */
    @Volatile private var clipboardWatchWanted = true
    @Volatile private var serviceClient: NectarlinkServiceClient? = null
    @Volatile private var serviceStatus: ServiceStatus? = null

    /** True when the running helper was started through root (`su`), not Wireless debugging. */
    @Volatile var rootActive: Boolean = false
        private set

    /** True when the running helper was started through Shizuku. */
    @Volatile var shizukuActive: Boolean = false
        private set

    /** True when the custom ROM's privileged Nectarlink Services package is available on this device. */
    fun hasSystemService(context: Context): Boolean =
        NectarlinkServiceClient.isAvailable(context, requireSystemApp = true)

    fun hasSystemService(): Boolean =
        ::context.isInitialized && hasSystemService(context)

    /** True when connected to the custom ROM's Nectarlink Services binder after a verified handshake. */
    val romServiceActive: Boolean
        get() = serviceClient?.isConnected == true

    /** Capabilities reported by the connected custom ROM service, or 0L if unavailable/rejected. */
    val romServiceCapabilities: Long
        get() = if (romServiceActive) serviceStatus?.capabilities ?: 0L else 0L

    /** True when the shell-UID helper ([InputServer]) is actively running. */
    val helperRunning: Boolean
        get() = input != null

    /** True when the connected system service provides all core Elevated capabilities for this API level. */
    fun romServiceHasAllCapabilities(): Boolean =
        romServiceActive &&
            NectarlinkServiceContract.hasAllRequiredCapabilities(romServiceCapabilities, Build.VERSION.SDK_INT)

    val running: Boolean get() = input != null || romServiceActive

    /** Whether [flag] (one of [NectarlinkServiceContract].CAP_*) is active right now. */
    fun hasCapability(flag: Long): Boolean {
        if (input != null) {
            return flag != NectarlinkServiceContract.CAP_CALL_AUDIO || callAudioSupported
        }
        val client = serviceClient
        if (client != null && client.isConnected) {
            return serviceStatus?.hasCapability(flag) == true
        }
        return false
    }

    private fun activeService(requiredCapability: Long? = null): NectarlinkServiceClient? {
        val client = serviceClient?.takeIf { it.isConnected } ?: return null
        if (requiredCapability != null && serviceStatus?.hasCapability(requiredCapability) != true) return null
        return client
    }

    private val suPaths = arrayOf(
        "/system/bin/su",
        "/system/xbin/su",
        "/sbin/su",
        "/debug_ramdisk/su",
        "/data/adb/ksu/bin/su",
        "/data/adb/ap/bin/su",
    )

    /**
     * Whether the phone is rooted (Magisk, KernelSU, APatch), so Elevated may
     * offer root as a way to start. Only looks: nothing asks for root until the
     * user picks it. KernelSU and APatch show `su` only to apps they already
     * allow, so their manager apps count too (#291). Not cached: allowing
     * NectarLink there and coming back shows it at once.
     */
    fun isRootAvailable(): Boolean =
        suPaths.any { runCatching { java.io.File(it).exists() }.getOrDefault(false) } ||
            (::context.isInitialized && RootManagers.anyInstalled(context.packageManager))

    /** Whether the user set Elevated up through root. */
    private fun usesRoot() = isRootAvailable() && prefs().getBoolean(ROOT, false)

    /** Whether the user set Elevated up through Shizuku. */
    fun usesShizuku(): Boolean = prefs().getBoolean(SHIZUKU, false)

    /** Whether the user set up normal Elevated (Wireless debugging, root, or Shizuku). */
    fun isSetUp() = paired() || usesRoot() || usesShizuku()

    private fun hasAnyBackendSetUp() = hasSystemService() || isSetUp()

    private fun prefs() = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** The user picked Wireless debugging: root and Shizuku are no longer tried first. */
    fun useWirelessDebugging() {
        prefs().edit {
            putBoolean(ROOT, false)
            putBoolean(SHIZUKU, false)
        }
    }

    /**
     * Sets Elevated up through root, as the user asked (instead of pairing
     * Wireless debugging). The root manager asks once; after that Elevated
     * starts by itself, also after a restart and away from Wi-Fi. The helper
     * still runs as the shell user, with the same permissions either way.
     */
    suspend fun setUpWithRoot(): Boolean = withContext(Dispatchers.IO) {
        if (!isRootAvailable()) return@withContext false
        val started = lock.withLock {
            if (input != null) return@withLock true
            _state.value = State.Starting
            tryStartWithRootLocked()
        }
        prefs().edit {
            putBoolean(ROOT, started)
            if (started) putBoolean(SHIZUKU, false)
        }
        if (started) {
            cancelRetry()
        } else {
            _state.value = State.Failed(Reason.RootDenied)
        }
        started
    }

    /**
     * Sets Elevated up through Shizuku, as the user asked: Shizuku asks once
     * whether NectarLink may use it, and from then on Elevated starts through
     * it whenever Shizuku runs. The helper runs as the shell user, as through
     * Wireless debugging; a Shizuku running as root isn't used.
     */
    suspend fun setUpWithShizuku(): Boolean = withContext(Dispatchers.IO) {
        ShizukuElevated.requestPermission()?.let { problem ->
            _state.value = State.Failed(reasonFor(problem))
            return@withContext false
        }
        prefs().edit {
            putBoolean(SHIZUKU, true)
            putBoolean(ROOT, false)
        }
        held = false
        val started = lock.withLock {
            if (input != null && !shizukuActive) stopLocked()
            if (input != null) return@withLock true
            _state.value = State.Starting
            tryStartWithShizukuLocked()
        }
        if (started) cancelRetry()
        started
    }

    fun init(context: Context) {
        this.context = context.applicationContext
        if (hasSystemService(this.context)) {
            initSystemService()
        }
        watchWirelessDebugging()
        // Shizuku's service comes after the app on a boot, and again whenever
        // it's restarted: start then, if Elevated goes through it.
        runCatching {
            ShizukuElevated.watch {
                if (usesShizuku() && !held && !running && !BankingPause.isPaused(context)) scope.launch { start() }
            }
        }.onFailure { Log.i(TAG, "can't watch for Shizuku", it) }
        scope.launch(Dispatchers.IO) {
            if (hasAnyBackendSetUp()) {
                _state.value = State.Waiting
                start()
            }
        }
    }

    private fun initSystemService() {
        val client = NectarlinkServiceClient(context, requireSystemApp = true)
        serviceClient = client
        scope.launch {
            client.clipboardEvents.collect { event ->
                if (event.isSensitive) {
                    onSensitiveCopied(event.text)
                } else {
                    onClipboardChanged(event.text, event.html)
                }
            }
        }
        scope.launch {
            client.inputBlockedEvents.collect {
                onInputBlocked()
            }
        }
        scope.launch {
            client.state.collect { conn ->
                when (conn) {
                    is NectarlinkServiceClient.ConnectionState.Connected -> {
                        serviceStatus = conn.status
                        callAudioSupported = conn.status.callAudioSupported || input != null
                        _state.value = if (input != null || romServiceHasAllCapabilities()) {
                            State.Running
                        } else if (isSetUp()) {
                            State.Waiting
                        } else {
                            State.NotSetUp
                        }
                        cancelRetry()
                        onChange()
                        scope.launch(Dispatchers.IO) {
                            client.provisionAppPermissions(context.packageName)
                            if (screenOffRequested &&
                                conn.status.hasCapability(NectarlinkServiceContract.CAP_DISPLAY_POWER)
                            ) {
                                client.setScreenOff(true)
                            }
                            onChange()
                        }
                    }
                    is NectarlinkServiceClient.ConnectionState.Rejected -> {
                        serviceStatus = null
                        callAudioSupported = false
                        if (input == null) {
                            _state.value = if (isSetUp()) State.Waiting else State.NotSetUp
                        }
                        onChange()
                    }
                    NectarlinkServiceClient.ConnectionState.Disconnected,
                    NectarlinkServiceClient.ConnectionState.Unavailable -> {
                        val wasRunning = _state.value == State.Running
                        serviceStatus = null
                        callAudioSupported = false
                        if (input == null && (wasRunning || !isSetUp())) {
                            _state.value = if (isSetUp()) State.Waiting else State.NotSetUp
                        }
                        onChange()
                        if (!held && hasSystemService() && !running) scheduleRetry()
                    }
                    NectarlinkServiceClient.ConnectionState.Connecting -> Unit
                }
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

    /** Tries again later, while it can still work: set up, not running, on Wi-Fi (or through system service, root or Shizuku). */
    private fun scheduleRetry() {
        if (held || !hasAnyBackendSetUp() || running || (!hasSystemService() && BankingPause.isPaused(context))) return
        val withoutWifi = hasSystemService() || usesRoot() || (usesShizuku() && ShizukuElevated.allowed())
        if (!withoutWifi && !onWifi()) return
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

    /** Whether wireless debugging is on; on when the setting can't be read. */
    private fun wirelessDebuggingOn(): Boolean =
        runCatching { Settings.Global.getInt(context.contentResolver, ADB_WIFI, 1) == 1 }.getOrDefault(true)

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
     * see [LoopbackAdb], and earlier when mDNS finds nothing), waiting a moment
     * for it to start listening.
     */
    private suspend fun connectOverLoopback(adb: Connection): Boolean {
        // With wireless debugging off nothing listens: no need to look through
        // every port for 10 seconds on each retry.
        if (!wirelessDebuggingOn()) return false
        val deadline = System.currentTimeMillis() + CONNECT_MS
        while (true) {
            val port = LoopbackAdb.connectPort(LoopbackAdb.openPorts())
            if (port != null) return adb.connect(LoopbackAdb.HOST, port)
            if (System.currentTimeMillis() >= deadline) return false
            delay(LOOPBACK_RETRY_MS)
        }
    }

    /**
     * Connects to wireless debugging found with mDNS, or else on 127.0.0.1:
     * some phones and networks never answer the lookup (#147), and adbd
     * listens on this phone either way. A pairing adbd forgot is still
     * reported, so the user is asked to pair again.
     */
    private suspend fun connectFoundOrLoopback(adb: Connection): Boolean =
        LoopbackAdb.foundOrLoopback(
            found = { adb.connectTls(context, CONNECT_MS) },
            loopback = { connectOverLoopback(adb) },
            onMissed = { Log.i(TAG, "wireless debugging wasn't found on the network; trying this phone directly", it) },
        )

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
            val helperPort = helper?.first
            val candidates = open.filter { it != connectPort && it != helperPort }
            if (candidates.isNotEmpty()) {
                return LoopbackAdb.anySucceeds(candidates, PAIR_MS) { port -> pairOnPort(port, code) }
            }
            if (System.currentTimeMillis() >= deadline) return null
            delay(LOOPBACK_RETRY_MS)
        }
    }

    /**
     * One pairing attempt on a connection of its own: the library holds a lock
     * while pairing, with no timeout, so a port that never answers would block
     * every attempt sharing that connection.
     */
    private fun pairOnPort(port: Int, code: String): Boolean =
        runCatching { Connection(context).pair(LoopbackAdb.HOST, port, code) }.getOrElse {
            Log.i(TAG, "no pairing on port $port", it)
            false
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
        if (hasSystemService()) {
            val client = serviceClient
            if (client != null) {
                val connected = if (client.isConnected) {
                    true
                } else {
                    lock.withLock {
                        if (client.isConnected) return@withLock true
                        if (client.state.value is NectarlinkServiceClient.ConnectionState.Rejected) {
                            client.disconnect()
                        }
                        _state.value = State.Starting
                        client.connect()
                    }
                }
                if (connected) {
                    cancelRetry()
                    if (romServiceHasAllCapabilities() || !isSetUp()) {
                        return@withContext true
                    }
                } else if (!isSetUp() && !BankingPause.isPaused(context)) {
                    // Service refused or is off and user hasn't configured normal Elevated yet;
                    // still allow USB auto-start if plugged in.
                    val usbStarted = lock.withLock { running || startOverUsbLocked() }
                    if (usbStarted) {
                        cancelRetry()
                        return@withContext true
                    }
                    if (client.state.value !is NectarlinkServiceClient.ConnectionState.Rejected) {
                        _state.value = State.NotSetUp
                        scheduleRetry()
                    } else {
                        _state.value = State.NotSetUp
                    }
                    return@withContext false
                }
            }
        }
        // Paused for a banking app: debugging stays off until it's over
        // (BankingPause starts Elevated again then).
        if (BankingPause.isPaused(context)) {
            if (!running) _state.value = State.Waiting
            return@withContext false
        }
        val started = lock.withLock {
            if (input != null) return@withLock true
            _state.value = State.Starting
            if (usesRoot() && tryStartWithRootLocked()) {
                return@withLock true
            }
            if (usesShizuku() && tryStartWithShizukuLocked()) {
                return@withLock true
            }
            if (!paired()) {
                // No wireless debugging set up: a PC this phone is plugged into can start it.
                if (startOverUsbLocked()) return@withLock true
                // Through Shizuku, the state says why it didn't start, or that it waits for Shizuku.
                if (!usesShizuku()) _state.value = State.NotSetUp
                return@withLock false
            }
            turnOnWirelessDebugging()
            // A connection from before is stale once the helper stopped
            // (wireless debugging went off with the old network): start fresh.
            discardConnection()
            val adb = connection()
            val connected = try {
                if (LoopbackAdb.preferred()) connectOverLoopback(adb) else connectFoundOrLoopback(adb)
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
            val proc = ProcessBuilder(suAsShell() + listOf("-c", cmd)).start()
            rootProcess = proc
            // The helper reports what fails (a screen it can't capture) on
            // stderr. Read it, so it reaches the log and a full pipe never
            // stalls the helper.
            Thread {
                runCatching {
                    proc.errorStream.bufferedReader().lineSequence().forEach { line -> Log.i(TAG, "root-helper: $line") }
                }
            }.apply { isDaemon = true }.start()
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
                if (rootProcess === proc) {
                    input = null
                    rootActive = false
                    runCatching { socket?.close() }
                    if (_state.value == State.Running) _state.value = State.Waiting
                    onChange()
                    scheduleRetry()
                }
            }.start()
            true
        }.onFailure {
            Log.i(TAG, "the helper didn't start through root", it)
            runCatching { rootProcess?.destroyForcibly() }
            rootProcess = null
            rootActive = false
        }.getOrDefault(false)
    }

    /**
     * Starts [InputServer] through Shizuku ([ShizukuElevated]). Leaves the
     * state saying why when it can't: Shizuku not running (it waits for it),
     * not allowed, or running as root.
     */
    private suspend fun tryStartWithShizukuLocked(): Boolean {
        val token = ByteArray(24).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
        val port = ShizukuElevated.startHelper(context, grants(), token).getOrElse { e ->
            val problem = (e as? ShizukuElevated.ShizukuProblem)?.problem
            if (problem == null) Log.i(TAG, "the helper didn't start through Shizuku", e)
            _state.value = when (problem) {
                ShizukuElevated.Problem.NotRunning -> State.Waiting
                null -> State.Failed(Reason.Other)
                else -> State.Failed(reasonFor(problem))
            }
            return false
        }
        return runCatching {
            shizukuActive = true
            connectSocketLocked(port, token)
            Log.i(TAG, "started through Shizuku")
            true
        }.getOrElse {
            Log.w(TAG, "the helper started through Shizuku didn't answer", it)
            shizukuActive = false
            ShizukuElevated.stop()
            _state.value = State.Failed(Reason.Other)
            false
        }
    }

    private fun reasonFor(problem: ShizukuElevated.Problem) = when (problem) {
        ShizukuElevated.Problem.NotRunning -> Reason.ShizukuNotRunning
        ShizukuElevated.Problem.Denied -> Reason.ShizukuDenied
        ShizukuElevated.Problem.Root -> Reason.ShizukuRoot
    }

    /** Runs [command] as the shell user through `su`; its output, or null if it didn't run. */
    private fun asShell(command: String): String? = runSu(suAsShell(), command)

    /** The `su` that [suAsShell] settled on, once root answered. */
    @Volatile
    private var shellSu: List<String>? = null

    /**
     * `su` to the shell user with the shell user's groups, which Magisk 26+
     * and KernelSU take as -g and -G. A plain `su 2000` leaves the user in no
     * group at all, so the helper can't open its socket (no inet) or make a
     * gamepad (no uhid). Root managers without -G get the plain one.
     */
    private fun suAsShell(): List<String> = shellSu ?: run {
        val plain = listOf("su", SHELL_UID)
        val withGroups = listOf("su", "-g", SHELL_UID) + SHELL_GROUPS.flatMap { listOf("-G", it.toString()) } + SHELL_UID
        val groups = runSu(withGroups, "id -G")?.trim()?.split(Regex("\\s+"))
        when {
            groups != null && INET_GROUP in groups -> withGroups.also { shellSu = it }
            groups != null -> plain.also {
                shellSu = it
                Log.i(TAG, "root gives the shell user no inet group: the helper may not open its socket")
            }
            // -G not understood, or root not allowed (yet): ask again next time.
            else -> plain
        }
    }

    /** Runs [command] through `su` with [su] as its arguments; its output, or null if it didn't run. */
    private fun runSu(su: List<String>, command: String): String? = runCatching {
        val process = ProcessBuilder(su + listOf("-c", command)).redirectErrorStream(true).start()
        if (!process.waitFor(SU_WAIT_S, java.util.concurrent.TimeUnit.SECONDS)) {
            process.destroyForcibly()
            return null
        }
        process.inputStream.bufferedReader().readText().takeIf { process.exitValue() == 0 }
    }.getOrNull()

    /**
     * Whether Elevated turns Wireless debugging back on by itself when it finds it off
     * (on any Wi-Fi). On unless the user turned it off; then they turn it on themselves
     * in Developer options when they want Elevated.
     */
    fun turnsOnWirelessDebugging(): Boolean =
        !::context.isInitialized || prefs().getBoolean(TURN_ON_WIRELESS, true)

    fun setTurnsOnWirelessDebugging(on: Boolean) {
        prefs().edit { putBoolean(TURN_ON_WIRELESS, on) }
    }

    /** Wireless debugging back on, when NectarLink may (granted at the first start, and not turned off). */
    private fun turnOnWirelessDebugging() {
        if (!turnsOnWirelessDebugging()) return
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
            if (shell === stream) {
                input = null
                runCatching { socket?.close() }
                if (_state.value == State.Running) _state.value = if (isSetUp()) State.Waiting else State.NotSetUp
                onChange()
                scheduleRetry()
            }
        }.start()
    }

    private fun connectSocketLocked(port: Int, token: String) {
        val local = HelperProof.open(port, token, "H", PROOF_MS)
        helper = port to token
        socket = local
        val writer = OutputStreamWriter(local.outputStream, Charsets.UTF_8)
        input = writer
        _state.value = State.Running
        if (screenOffRequested) send(writer, "P 0")
        send(writer, "W ${if (clipboardWatchWanted) 1 else 0}")
        compileIfUpdated()
        checkCallAudio()
        Thread {
            runCatching {
                local.inputStream.bufferedReader().lineSequence().forEach { line ->
                    if (line.startsWith("FI ") || line.startsWith("FP ")) {
                        pointerReplies.offer(line)
                        return@forEach
                    }
                    if (line == "BLOCKED") onInputBlocked()
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
                            if (parts.getOrNull(3) == "1") onSensitiveCopied(text) else onClipboardChanged(text, html)
                        }
                    }
                }
            }
            if (socket === local) {
                input = null
                shizukuActive = false
                runCatching { local.close() }
                if (_state.value == State.Running) _state.value = if (isSetUp()) State.Waiting else State.NotSetUp
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
        if (romServiceHasAllCapabilities() || held || input != null || BankingPause.isPaused(context)) {
            return@withContext running
        }
        lock.withLock { input != null || startOverUsbLocked() }
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

    /** Stops the helper while paused for a banking app; [onNetworkChanged] starts it again when resumed. */
    fun pauseForBanking() {
        if (romServiceActive && input == null) return
        cancelRetry()
        scope.launch(Dispatchers.IO) {
            lock.withLock { stopLocked() }
        }
    }

    private fun stopLocked() {
        val wasScreenOff = screenOffRequested
        screenOffRequested = false
        serviceClient?.let { client ->
            if (wasScreenOff && client.isConnected) {
                runCatching { client.setScreenOff(false) }
            }
            client.disconnect()
            serviceStatus = null
        }
        val writer = input
        input = null
        helper = null
        callAudioSupported = false
        if (wasScreenOff && writer != null) {
            runCatching {
                synchronized(writer) {
                    writer.write("P 1\n")
                    writer.flush()
                }
            }
        }
        runCatching { socket?.close() }
        socket = null
        if (wasScreenOff) {
            // Let InputServer restore panel power and brightness before its process is torn down.
            runCatching { Thread.sleep(150) }
        }
        rootActive = false
        if (shizukuActive) ShizukuElevated.stop()
        shizukuActive = false
        runCatching { shell?.close() }
        runCatching {
            rootProcess?.let { proc ->
                if (!proc.waitFor(200, java.util.concurrent.TimeUnit.MILLISECONDS)) {
                    proc.destroyForcibly()
                }
            }
        }
        runCatching { adb?.disconnect() }
        shell = null
        rootProcess = null
        _state.value = if (isSetUp()) State.Waiting else State.NotSetUp
        onChange()
    }

    /** Forgets the pairing (the user turned Elevated off). */
    suspend fun forget() {
        cancelRetry()
        stop()
        prefs().edit {
            putBoolean(PAIRED, false)
            putBoolean(ROOT, false)
            putBoolean(SHIZUKU, false)
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
    @Volatile var onSensitiveCopied: (String) -> Unit = {}

    /** The phone refuses the helper's taps and keys (Xiaomi, until USB debugging (Security settings) is on). */
    @Volatile var onInputBlocked: () -> Unit = {}

    // ---- Input ----

    /** The PC's input as real events; false if Elevated isn't running. */
    fun handle(event: MirrorInputEvent): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_INPUT_INJECTION)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) {
                val (w, h) = screen()
                return handleOnWithService(client, null, w, h, event)
            }
            offMain.execute {
                val (w, h) = screen()
                handleOnWithService(client, null, w, h, event)
            }
            return true
        }
        if (input == null) return false
        if (Thread.currentThread() === offMainThread) {
            val (w, h) = screen()
            return handleOn(null, w, h, event)
        }
        offMain.execute {
            val (w, h) = screen()
            handleOn(null, w, h, event)
        }
        return true
    }

    /**
     * The PC's input on display [display] (an app window's, [w] × [h]
     * pixels), or the screen when null.
     */
    fun handleOn(display: Int?, w: Float, h: Float, event: MirrorInputEvent): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_INPUT_INJECTION)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) {
                return handleOnWithService(client, display, w, h, event)
            }
            offMain.execute { handleOnWithService(client, display, w, h, event) }
            return true
        }
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
            is MirrorInputEvent.Key -> keys[event.key]?.let { "K $it" } ?: when {
                // No key code selects all: Ctrl+A, which text fields take as a shortcut.
                event.key == "select_all" -> "Y ${KeyEvent.KEYCODE_A} ${KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON}"
                event.key == "notifications" && display == null -> "C cmd statusbar expand-notifications"
                else -> return true
            }
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

    private fun handleOnWithService(
        client: NectarlinkServiceClient,
        display: Int?,
        w: Float,
        h: Float,
        event: MirrorInputEvent,
    ): Boolean {
        val targetDisplay = display ?: -1
        return when (event) {
            is MirrorInputEvent.Touch -> {
                val action = when (event.action) {
                    TouchPhase.DOWN -> NectarlinkServiceContract.TOUCH_DOWN
                    TouchPhase.MOVE -> NectarlinkServiceContract.TOUCH_MOVE
                    TouchPhase.UP -> NectarlinkServiceContract.TOUCH_UP
                }
                client.injectTouch(targetDisplay, action, event.x * w, event.y * h)
            }
            is MirrorInputEvent.Scroll ->
                client.injectScroll(targetDisplay, event.x * w, event.y * h, event.dx, event.dy)
            is MirrorInputEvent.Key -> keys[event.key]?.let {
                client.injectKey(targetDisplay, it, 0)
            } ?: when {
                event.key == "select_all" ->
                    client.injectKey(targetDisplay, KeyEvent.KEYCODE_A, KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON)
                event.key == "notifications" && display == null ->
                    client.expandNotifications()
                else -> true
            }
            is MirrorInputEvent.Text -> when {
                event.text.isEmpty() -> true
                event.text == "\n" || event.text == "\r" || event.text == "\r\n" ->
                    client.injectKey(targetDisplay, KeyEvent.KEYCODE_ENTER, 0)
                event.text.length == 1 && event.text[0].code in 32..126 ->
                    client.injectText(targetDisplay, event.text)
                else ->
                    client.setClipboard(event.text, null, false) &&
                        client.injectKey(targetDisplay, KeyEvent.KEYCODE_PASTE, 0)
            }
            is MirrorInputEvent.Gamepad ->
                client.injectGamepad(
                    targetDisplay,
                    event.buttons.toInt(),
                    event.lx.toInt(),
                    event.ly.toInt(),
                    event.rx.toInt(),
                    event.ry.toInt(),
                    event.lt.toInt(),
                    event.rt.toInt(),
                )
        }
    }

    /** Moves the music volume to this index through the helper, false if it isn't running. */
    fun setMusicVolume(index: Int): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_SYSTEM_TOGGLES)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) return client.setMusicVolume(index)
            offMain.execute { client.setMusicVolume(index) }
            return true
        }
        val writer = input ?: return false
        return send(writer, "V $index")
    }

    // ---- The PC's mouse on the phone (#107) ----

    /** What the helper answered to a pointer question (`FI`, `FP` lines). */
    private val pointerReplies = java.util.concurrent.LinkedBlockingQueue<String>()

    /** The screen and pointer scale for the PC's mouse here. */
    data class PointerInfo(val width: Int, val height: Int, val scale: Float)

    /**
     * Starts the PC's mouse here: the pointer shows at the [edge] ("left" or
     * "right") it came in from, [y] (0 to 1) down. Null without Elevated or
     * when it couldn't start. Takes about a second; not on the main thread.
     */
    fun pointerStart(edge: String, y: Float): PointerInfo? {
        val client = activeService(NectarlinkServiceContract.CAP_POINTER_FLOW)
        if (client != null) {
            val info = client.pointerStart(edge, y) ?: return null
            return PointerInfo(info.width, info.height, info.scale).takeIf { it.width > 0 && it.height > 0 && it.scale > 0f }
        }
        val writer = input ?: return null
        pointerReplies.clear()
        if (!sendNow(writer, "FS $edge $y")) return null
        val reply = pointerReplies.poll(POINTER_START_MS, java.util.concurrent.TimeUnit.MILLISECONDS) ?: return null
        val (w, h, scale) = reply.removePrefix("FI ").split(' ').takeIf { it.size == 3 } ?: return null
        val info = PointerInfo(w.toIntOrNull() ?: 0, h.toIntOrNull() ?: 0, scale.toFloatOrNull() ?: 0f)
        return info.takeIf { it.width > 0 && it.height > 0 && it.scale > 0f }
    }

    /** Moves the PC's mouse here by counts, with [buttons] held (bits: left, right, middle). */
    fun pointerMove(dx: Int, dy: Int, buttons: Int, wheel: Int): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_POINTER_FLOW)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) return client.pointerMove(dx, dy, buttons, wheel)
            offMain.execute { client.pointerMove(dx, dy, buttons, wheel) }
            return true
        }
        val writer = input ?: return false
        return send(writer, "FM $dx $dy $buttons $wheel")
    }

    /** Where the pointer is, in pixels; null when unknown. Not on the main thread. */
    fun pointerWhere(): Pair<Float, Float>? {
        val client = activeService(NectarlinkServiceContract.CAP_POINTER_FLOW)
        if (client != null) {
            return client.pointerWhere()
        }
        val writer = input ?: return null
        pointerReplies.clear()
        if (!sendNow(writer, "FW")) return null
        val reply = pointerReplies.poll(POINTER_WHERE_MS, java.util.concurrent.TimeUnit.MILLISECONDS) ?: return null
        val (x, y) = reply.removePrefix("FP ").split(' ').takeIf { it.size == 2 } ?: return null
        val at = (x.toFloatOrNull() ?: -1f) to (y.toFloatOrNull() ?: -1f)
        return at.takeIf { it.first >= 0f && it.second >= 0f }
    }

    /** The PC's mouse went back to the PC. */
    fun pointerStop() {
        val client = activeService(NectarlinkServiceContract.CAP_POINTER_FLOW)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) {
                client.pointerStop()
            } else {
                offMain.execute { client.pointerStop() }
            }
            return
        }
        val writer = input ?: return
        send(writer, "FX")
    }

    /** Sends a gamepad state directly through the Elevated helper. */
    fun sendGamepad(buttons: Int, lx: Int, ly: Int, rx: Int, ry: Int, lt: Int, rt: Int): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_INPUT_INJECTION)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) {
                return client.injectGamepad(-1, buttons, lx, ly, rx, ry, lt, rt)
            }
            offMain.execute { client.injectGamepad(-1, buttons, lx, ly, rx, ry, lt, rt) }
            return true
        }
        val writer = input ?: return false
        return send(writer, "G $buttons $lx $ly $rx $ry $lt $rt")
    }

    /**
     * Opens a 48 kHz 16-bit stereo PCM audio stream from the Elevated helper
     * (`InputServer` command `"A"`) or the system service. Returns null if Elevated isn't running.
     */
    fun openAudioStream(): ElevatedAudioStream? {
        val client = activeService(NectarlinkServiceContract.CAP_DEVICE_AUDIO)
        if (client != null) {
            return client.openDeviceAudio()?.let(::PfdAudioStream)
        }
        val (port, token) = helper ?: return null
        return runCatching {
            // The helper proves it's the one this app started before the token goes out.
            SocketAudioStream(HelperProof.open(port, token, "A", PROOF_MS))
        }.onFailure { Log.w(TAG, "can't open Elevated audio stream", it) }.getOrNull()
    }

    /**
     * Opens the call in progress's sound from the Elevated helper ([CallAudio]) or the system service:
     * the other party arrives on the stream, what's written goes into the
     * call. Null, with the helper's reason logged, if it can't.
     */
    fun openCallAudio(): ElevatedCallAudioStream? {
        val client = activeService(NectarlinkServiceContract.CAP_CALL_AUDIO)
        if (client != null) {
            val pfd = client.openCallAudio() ?: return null
            val stream = PfdCallAudioStream(pfd)
            val answer = SystemServiceBridge.readCallAudioAnswer(stream.inputStream, CALL_AUDIO_ANSWER_MS.toLong()) {
                runCatching { stream.close() }
            }
            if (answer != "OK") {
                Log.w(TAG, "no call sound from system service: ${answer ?: "timeout"}")
                runCatching { stream.close() }
                return null
            }
            return stream
        }
        return askCallAudio("CALL")?.let(::SocketCallAudioStream)
    }

    /**
     * Whether this phone lets the helper reach a call's sound (Android 13+ and
     * the audio HAL's telephony Rx/Tx devices): checked each time the helper
     * starts, and `call.audio` is offered only when true.
     */
    @Volatile var callAudioSupported: Boolean = false
        private set

    private fun checkCallAudio() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        Thread {
            val supported = askCallAudio("CALLCHECK")?.also { runCatching { it.close() } } != null
            if (supported != callAudioSupported && helper != null) {
                callAudioSupported = supported
                Log.i(TAG, "call sound on a PC: ${if (supported) "supported" else "not supported"}")
                onChange()
            }
        }.start()
    }

    /** Sends a call-sound command to the helper: the socket after its `OK`, or null. */
    private fun askCallAudio(command: String): java.net.Socket? {
        val (port, token) = helper ?: return null
        val socket = runCatching {
            HelperProof.open(port, token, command, PROOF_MS).apply { soTimeout = CALL_AUDIO_ANSWER_MS }
        }.onFailure { Log.w(TAG, "can't reach the Elevated helper for call sound", it) }.getOrNull() ?: return null
        // The answer line, read byte by byte so no sound after it is swallowed.
        val answer = runCatching {
            val line = StringBuilder()
            while (true) {
                val b = socket.getInputStream().read()
                if (b < 0 || b == '\n'.code || line.length > 256) break
                line.append(b.toChar())
            }
            line.toString()
        }.getOrDefault("")
        if (answer != "OK") {
            Log.w(TAG, "no call sound: $answer")
            runCatching { socket.close() }
            return null
        }
        socket.soTimeout = 0
        return socket
    }

    /** Lets NectarLink see which app is in front (for Pause for banking apps); false if Elevated isn't running. */
    fun grantUsageAccess(): Boolean {
        val client = activeService()
        if (client != null) {
            return client.provisionAppPermissions(context.packageName)
        }
        return runCommand("appops set ${context.packageName} GET_USAGE_STATS allow")
    }

    /**
     * Marks the received texts of a conversation read in the phone's messages,
     * through the helper (the shell may change them once allowed; see
     * [InputServer]). False when the helper isn't running.
     */
    fun markSmsRead(thread: Long): Boolean {
        if (thread <= 0) return false
        val writer = input ?: return false
        return send(writer, "R $thread")
    }

    /** Runs an allowed shell command through the Elevated helper or system service. */
    fun runCommand(command: String): Boolean {
        val requiredCap = SystemServiceBridge.requiredCapability(command, context.packageName)
        val client = activeService(requiredCap)
        if (client != null && SystemServiceBridge.executeCommand(client, command, context.packageName)) {
            return true
        }
        val writer = input ?: return false
        return send(writer, "C $command")
    }

    /**
     * Puts a PC's text on the clipboard through the helper, so the phone
     * doesn't pop up "copied" for a copy made on the PC. False when the
     * helper isn't running (the app writes it then).
     */
    /** Has the helper empty the clipboard if `text` is still on it; false if it isn't running. */
    fun clearClipboard(text: String): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_CLIPBOARD_SYNC)
        if (client != null) {
            return client.clearClipboard(text)
        }
        val writer = input ?: return false
        val b64 = android.util.Base64.encodeToString(text.toByteArray(Charsets.UTF_8), android.util.Base64.NO_WRAP)
        return send(writer, "X $b64")
    }

    fun setClipboard(text: String, html: String?, sensitive: Boolean = false): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_CLIPBOARD_SYNC)
        if (client != null) {
            return client.setClipboard(text, html, sensitive)
        }
        val writer = input ?: return false
        val b64 = { value: String -> android.util.Base64.encodeToString(value.toByteArray(Charsets.UTF_8), android.util.Base64.NO_WRAP) }
        return send(writer, "B ${b64(text)} ${html?.let(b64).orEmpty()} ${if (sensitive) "1" else "0"}")
    }

    /**
     * Wakes the screen when it's off, as the power key would (the lock screen
     * shows if the phone is locked). Mirroring otherwise waits for someone to
     * turn the screen on by hand (#82).
     */
    fun wakeScreen(): Boolean {
        val client = activeService(NectarlinkServiceContract.CAP_DISPLAY_POWER)
        if (client != null) {
            val power = context.getSystemService(android.os.PowerManager::class.java)
            if (power?.isInteractive != false) return true
            return client.wakeScreen()
        }
        val writer = input ?: return false
        val power = context.getSystemService(android.os.PowerManager::class.java)
        if (power?.isInteractive != false) return true
        return send(writer, "K ${KeyEvent.KEYCODE_WAKEUP}")
    }

    /**
     * Has the helper watch the clipboard, or stop: it only needs to while some
     * device takes this phone's copies by itself. On some phones (a Xiaomi
     * with HyperOS) its watching left paste empty in every app (#655).
     */
    fun setClipboardWatch(on: Boolean) {
        if (clipboardWatchWanted == on) return
        clipboardWatchWanted = on
        val writer = input ?: return
        send(writer, "W ${if (on) 1 else 0}")
    }

    /** Turns the physical screen panel off or back on while mirroring. */
    fun setScreenOff(off: Boolean): Boolean {
        screenOffRequested = off
        val client = activeService(NectarlinkServiceContract.CAP_DISPLAY_POWER)
        if (client != null) {
            if (Thread.currentThread() === offMainThread) return client.setScreenOff(off)
            offMain.execute { client.setScreenOff(off) }
            return true
        }
        val writer = input ?: return false.also { Log.i(TAG, "screen ${if (off) "off" else "on"} asked, Elevated isn't running") }
        return send(writer, "P ${if (off) 0 else 1}")
    }

    /**
     * Opens an app window: the helper runs [pkg] on a display of its own of
     * [width] × [height] pixels at [dpi], and streams it on the returned
     * stream (packets as [AppDisplay] writes them). Null when Elevated
     * isn't running.
     */
    fun openApp(
        pkg: String,
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): ElevatedDisplayStream? {
        if (!PACKAGE.matches(pkg)) return null
        val client = activeService(NectarlinkServiceContract.CAP_APP_VIRTUAL_DISPLAY)
        if (client != null) {
            val session = client.openAppDisplay(pkg, width, height, dpi, bitrate, fps, codec) ?: return null
            val pfd = runCatching { session.videoReadFd }.getOrNull()
            if (pfd == null) {
                runCatching { session.close() }
                return null
            }
            return BinderDisplayStream(session, pfd)
        }
        val (port, token) = helper ?: return null
        return runCatching {
            SocketDisplayStream(HelperProof.open(port, token, "V $width $height $dpi $bitrate $fps $pkg $codec", PROOF_MS))
        }.onFailure { Log.w(TAG, "can't open an app window", it) }.getOrNull()
    }

    /**
     * Opens a full-screen mirror stream (`pkg == "-"`) via the Elevated shell
     * helper or system service without requiring a MediaProjection consent prompt on the phone.
     */
    fun openScreen(
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): ElevatedDisplayStream? {
        val client = activeService(NectarlinkServiceContract.CAP_SCREEN_MIRROR)
        if (client != null) {
            val session = client.openScreenMirror(width, height, dpi, bitrate, fps, codec) ?: return null
            val pfd = runCatching { session.videoReadFd }.getOrNull()
            if (pfd == null) {
                runCatching { session.close() }
                return null
            }
            return BinderDisplayStream(session, pfd)
        }
        val (port, token) = helper ?: return null
        return runCatching {
            SocketDisplayStream(HelperProof.open(port, token, "V $width $height $dpi $bitrate $fps - $codec", PROOF_MS))
        }.onFailure { Log.w(TAG, "can't open Elevated screen mirror", it) }.getOrNull()
    }

    /**
     * Writes [line] to the helper. Dispatched onto [offMain] in order unless
     * already running there, so neither Android's main thread nor the core's
     * Tokio worker threads block on loopback socket writes.
     */
    private fun send(writer: Writer, line: String): Boolean {
        if (Thread.currentThread() !== offMainThread) {
            offMain.execute { sendNow(writer, line) }
            return true
        }
        return sendNow(writer, line)
    }

    @Volatile private var offMainThread: Thread? = null

    private val offMain = java.util.concurrent.Executors.newSingleThreadExecutor { r ->
        Thread(r, "elevated-send").apply {
            isDaemon = true
            offMainThread = this
        }
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
        if (input === writer) input = null
        false
    }

    private val PACKAGE = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

    private const val POINTER_START_MS = 5_000L
    private const val POINTER_WHERE_MS = 2_000L

    private val keys = mapOf(
        "back" to KeyEvent.KEYCODE_BACK,
        "home" to KeyEvent.KEYCODE_HOME,
        "lock" to KeyEvent.KEYCODE_SLEEP,
        "recents" to KeyEvent.KEYCODE_APP_SWITCH,
        "enter" to KeyEvent.KEYCODE_ENTER,
        "backspace" to KeyEvent.KEYCODE_DEL,
        "delete" to KeyEvent.KEYCODE_FORWARD_DEL,
        "left" to KeyEvent.KEYCODE_DPAD_LEFT,
        "right" to KeyEvent.KEYCODE_DPAD_RIGHT,
        "up" to KeyEvent.KEYCODE_DPAD_UP,
        "down" to KeyEvent.KEYCODE_DPAD_DOWN,
        "tab" to KeyEvent.KEYCODE_TAB,
        // Text fields copy and cut on these, as on a keyboard's Copy and Cut keys.
        "copy" to KeyEvent.KEYCODE_COPY,
        "cut" to KeyEvent.KEYCODE_CUT,
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
    private const val PAIR_MS = 6_000L
    private const val CONNECT_MS = 10_000L
    /** How long a `su` command may take; the first one waits for the root manager's prompt. */
    private const val SU_WAIT_S = 30L
    /**
     * Waits between attempts to start again; the last one repeats. Up to 10
     * minutes: a network change or wireless debugging coming on tries at once
     * anyway, and each try looks through thousands of ports.
     */
    private val RETRY_MS = longArrayOf(2_000L, 5_000L, 10_000L, 20_000L, 40_000L, 60_000L, 120_000L, 300_000L, 600_000L)
}
