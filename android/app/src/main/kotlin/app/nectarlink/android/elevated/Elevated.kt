// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import android.util.Log
import android.view.KeyEvent
import android.view.WindowManager
import androidx.core.content.edit
import app.nectarlink.android.calls.CallCompanion
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.TouchPhase
import io.github.muntashirakon.adb.AbsAdbConnectionManager
import io.github.muntashirakon.adb.AdbPairingRequiredException
import io.github.muntashirakon.adb.AdbStream
import io.github.muntashirakon.adb.android.AdbMdns
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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
 * debugging, with nothing else to install: the user pairs Nectarlink once
 * with a pairing code, and from then on Nectarlink connects to the phone's
 * debugging service and runs [InputServer] with the shell's permissions,
 * for real touch and keys on the mirrored screen.
 *
 * Wireless debugging turns itself off on a reboot or a new Wi-Fi network;
 * after the first pairing Nectarlink may turn it back on itself
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

    enum class Reason { WrongCode, NoPairingDialog, NotPaired, Other }

    private const val TAG = "Elevated"
    private const val PREFS = "elevated"
    private const val PAIRED = "paired"

    private val _state = MutableStateFlow<State>(State.NotSetUp)
    val state: StateFlow<State> = _state.asStateFlow()

    private lateinit var context: Context
    private val lock = Mutex()
    private var adb: Connection? = null
    private var shell: AdbStream? = null
    @Volatile private var input: Writer? = null
    private var socket: java.net.Socket? = null
    /** Where the helper listens, and the token it wants, while it runs. */
    @Volatile private var helper: Pair<Int, String>? = null

    val running: Boolean get() = input != null

    fun init(context: Context) {
        this.context = context.applicationContext
        if (paired()) _state.value = State.Waiting
    }

    private fun paired() = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(PAIRED, false)

    /** The phone's own adbd, as Nectarlink. */
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
            val port = discover(AdbMdns.SERVICE_TYPE_TLS_PAIRING)
            if (port == null) {
                _state.value = State.Failed(Reason.NoPairingDialog)
                return@withContext false
            }
            val paired = runCatching { connection().pair("127.0.0.1", port, code.trim()) }.getOrElse {
                Log.w(TAG, "pairing failed", it)
                false
            }
            if (!paired) {
                _state.value = State.Failed(Reason.WrongCode)
                return@withContext false
            }
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(PAIRED, true) }
        }
        start()
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
        if (!paired()) return@withContext false
        lock.withLock {
            if (running) return@withContext true
            _state.value = State.Starting
            turnOnWirelessDebugging()
            val adb = connection()
            val connected = try {
                adb.isConnected || adb.connectTls(context, CONNECT_MS)
            } catch (e: AdbPairingRequiredException) {
                // adbd forgot us (the user revoked the authorizations).
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(PAIRED, false) }
                _state.value = State.Failed(Reason.NotPaired)
                return@withContext false
            } catch (e: Exception) {
                Log.i(TAG, "wireless debugging isn't reachable", e)
                false
            }
            if (!connected) {
                _state.value = State.Waiting
                return@withContext false
            }
            runCatching { launch(adb) }.onFailure {
                Log.w(TAG, "the input helper didn't start", it)
                stopLocked()
                _state.value = State.Failed(Reason.Other)
            }.isSuccess
        }
    }

    /** Wireless debugging back on, when Nectarlink may (granted at the first start). */
    private fun turnOnWirelessDebugging() {
        if (context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) != PackageManager.PERMISSION_GRANTED) return
        runCatching {
            if (Settings.Global.getInt(context.contentResolver, ADB_WIFI, 0) != 1) {
                Settings.Global.putInt(context.contentResolver, ADB_WIFI, 1)
                Thread.sleep(1500) // adbd announces itself.
            }
        }.onFailure { Log.i(TAG, "can't turn on wireless debugging", it) }
    }

    private fun launch(adb: Connection) {
        // Turning wireless debugging back on later is Nectarlink's to do.
        if (context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) != PackageManager.PERMISSION_GRANTED) {
            run(adb, "pm grant ${context.packageName} ${Manifest.permission.WRITE_SECURE_SETTINGS}")
        }
        // Lets Telecom bind the calling companion: mute, speaker, hold and
        // the keypad from a PC.
        if (!CallCompanion.allowed(context) && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            run(adb, "appops set ${context.packageName} ${CallCompanion.OP_NAME} allow")
        }
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
        helper = port to token
        val local = java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply { tcpNoDelay = true }
        socket = local
        val writer = OutputStreamWriter(local.outputStream, Charsets.UTF_8)
        writer.write(token + "\n")
        writer.flush()
        input = writer
        _state.value = State.Running
        // Watches the helper: when its shell ends, Elevated stops.
        Thread {
            runCatching { output.lineSequence().forEach { line -> Log.d(TAG, "helper: $line") } }
            input = null
            runCatching { socket?.close() }
            if (_state.value == State.Running) _state.value = State.Waiting
            onChange()
        }.start()
        onChange()
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

    /** Stops the helper and disconnects. */
    suspend fun stop() = withContext(Dispatchers.IO) { lock.withLock { stopLocked() } }

    private fun stopLocked() {
        input = null
        helper = null
        runCatching { socket?.close() }
        runCatching { shell?.close() }
        runCatching { adb?.disconnect() }
        socket = null
        shell = null
        if (paired()) _state.value = State.Waiting
        onChange()
    }

    /** Forgets the pairing (the user turned Elevated off). */
    suspend fun forget() {
        stop()
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(PAIRED, false) }
        runCatching { adb?.close() }
        adb = null
        AdbKey.delete(context.noBackupFilesDir.resolve("adb"))
        _state.value = State.NotSetUp
        onChange()
    }

    /** Told when Elevated starts or stops (what the phone offers changes). */
    @Volatile var onChange: () -> Unit = {}

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
            // One line per piece: no line breaks inside.
            is MirrorInputEvent.Text -> return event.text.split('\n').withIndex().all { (i, piece) ->
                (i == 0 || send(writer, "${to}K ${KeyEvent.KEYCODE_ENTER}")) && (piece.isEmpty() || send(writer, "${to}T $piece"))
            }
        }
        return send(writer, line)
    }

    /**
     * Opens an app window: the helper runs [pkg] on a display of its own of
     * [width] × [height] pixels at [dpi], and streams it on the returned
     * connection (packets as [AppDisplay] writes them). Null when Elevated
     * isn't running. Write "K\n" for a keyframe; close it to close the window.
     */
    fun openApp(pkg: String, width: Int, height: Int, dpi: Int, bitrate: Int, fps: Int): java.net.Socket? {
        val (port, token) = helper ?: return null
        return runCatching {
            java.net.Socket(java.net.InetAddress.getLoopbackAddress(), port).apply {
                tcpNoDelay = true
                val out = OutputStreamWriter(getOutputStream(), Charsets.UTF_8)
                out.write("$token\nV $width $height $dpi $bitrate $fps $pkg\n")
                out.flush()
            }
        }.onFailure { Log.w(TAG, "can't open an app window", it) }.getOrNull()
    }

    private fun send(writer: Writer, line: String): Boolean = try {
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
    private const val CONNECT_MS = 10_000L
}
