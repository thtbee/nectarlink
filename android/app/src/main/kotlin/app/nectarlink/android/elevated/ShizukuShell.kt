// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.content.ComponentName
import android.content.Context
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Binder
import android.os.IBinder
import android.os.Parcel
import android.util.Log
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import rikka.shizuku.Shizuku
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread
import kotlin.system.exitProcess

/**
 * Starting Elevated through [Shizuku](https://shizuku.rikka.app/) (#110),
 * for phones where it already runs: Shizuku keeps a process with the
 * shell's permissions alive after it's started once, so Elevated needn't
 * hold Wireless debugging itself, and keeps working away from Wi-Fi.
 *
 * Shizuku starts [ShizukuShell] in a process of its own, as its own user,
 * and hands NectarLink a Binder to it. Through that it runs the same grants
 * and starts the same [InputServer] as the other ways do. Like root, it's
 * only used when that user is the shell (Shizuku started through Wireless
 * debugging or a PC), never when Shizuku runs as root.
 */
object ShizukuElevated {
    /** Shizuku's own app. */
    const val PACKAGE = "moe.shizuku.privileged.api"

    private const val TAG = "Elevated"
    /** Shizuku 13 passes the service a Context, which [ShizukuShell] needs to know who may call it. */
    private const val MIN_SERVER_VERSION = 13
    private const val PERMISSION_REQUEST = 0x5A71
    private const val BIND_MS = 15_000L
    private const val PERMISSION_MS = 120_000L

    /** Why starting through Shizuku didn't work. */
    enum class Problem { NotRunning, Denied, Root }

    /** Whether Shizuku's app is on the phone (so Elevated may offer it). */
    fun installed(context: Context): Boolean = runCatching {
        context.packageManager.getPackageInfo(PACKAGE, 0)
        true
    }.getOrDefault(false)

    /** Whether Shizuku's service is running and answering. */
    fun running(): Boolean = runCatching { Shizuku.pingBinder() }.getOrDefault(false)

    /** Whether Shizuku is running, new enough, and lets NectarLink use it. */
    fun allowed(): Boolean = running() && runCatching {
        !Shizuku.isPreV11() && Shizuku.getVersion() >= MIN_SERVER_VERSION &&
            Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
    }.getOrDefault(false)

    /** Calls [onRunning] whenever Shizuku's service (re)appears. */
    fun watch(onRunning: () -> Unit) {
        Shizuku.addBinderReceivedListenerSticky { onRunning() }
    }

    /**
     * Asks Shizuku to let NectarLink use it, if it hasn't yet; Shizuku shows
     * its own dialog. Null when Shizuku isn't usable at all.
     */
    suspend fun requestPermission(): Problem? {
        if (!running()) return Problem.NotRunning
        if (runCatching { Shizuku.isPreV11() || Shizuku.getVersion() < MIN_SERVER_VERSION }.getOrDefault(true)) {
            return Problem.Denied
        }
        if (allowed()) return null
        val answer = CompletableDeferred<Boolean>()
        val listener = Shizuku.OnRequestPermissionResultListener { code, result ->
            if (code == PERMISSION_REQUEST) answer.complete(result == PackageManager.PERMISSION_GRANTED)
        }
        Shizuku.addRequestPermissionResultListener(listener)
        return try {
            Shizuku.requestPermission(PERMISSION_REQUEST)
            if (withTimeoutOrNull(PERMISSION_MS) { answer.await() } == true) null else Problem.Denied
        } catch (e: RuntimeException) {
            Log.i(TAG, "Shizuku's permission request failed", e)
            Problem.Denied
        } finally {
            Shizuku.removeRequestPermissionResultListener(listener)
        }
    }

    private var args: Shizuku.UserServiceArgs? = null
    private var connection: ServiceConnection? = null
    @Volatile private var service: IBinder? = null

    /** The service's Binder, binding it first if needed. Null if Shizuku won't. */
    private suspend fun bind(context: Context): IBinder? {
        service?.takeIf { it.pingBinder() }?.let { return it }
        stop()
        val bound = CompletableDeferred<IBinder?>()
        val serviceArgs = Shizuku.UserServiceArgs(ComponentName(context.packageName, ShizukuShell::class.java.name))
            .processNameSuffix("elevated")
            .tag("elevated")
            .version(app.nectarlink.android.BuildConfig.VERSION_CODE)
            .debuggable(app.nectarlink.android.BuildConfig.DEBUG)
            // Ends with NectarLink, so the helper never outlives the app that controls it.
            .daemon(false)
        val conn = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
                bound.complete(service?.takeIf { it.pingBinder() })
            }

            override fun onServiceDisconnected(name: ComponentName?) {
                service = null
                bound.complete(null)
            }
        }
        withContext(Dispatchers.Main) {
            runCatching { Shizuku.bindUserService(serviceArgs, conn) }
                .onFailure { Log.i(TAG, "Shizuku didn't bind the helper's service", it) }
        }
        args = serviceArgs
        connection = conn
        return withTimeoutOrNull(BIND_MS) { bound.await() }.also { service = it }
    }

    /**
     * Runs the grants and starts [InputServer] with [token] through Shizuku.
     * Returns the port it listens on, or why it couldn't.
     */
    suspend fun startHelper(context: Context, grants: List<String>, token: String): Result<Int> {
        if (!running()) return Result.failure(ShizukuProblem(Problem.NotRunning))
        if (!allowed()) return Result.failure(ShizukuProblem(Problem.Denied))
        val binder = bind(context) ?: return Result.failure(ShizukuProblem(Problem.Denied))
        return withContext(Dispatchers.IO) {
            runCatching {
                val uid = transact(binder, ShizukuShell.UID) { reply -> reply.readInt() }
                if (uid != ShizukuShell.SHELL_UID) {
                    Log.i(TAG, "Shizuku runs as $uid, not the shell; not using it")
                    throw ShizukuProblem(Problem.Root)
                }
                grants.forEach { grant ->
                    transact(binder, ShizukuShell.RUN, { it.writeString(grant) }) { reply -> reply.readInt() }
                }
                val (port, said) = transact(binder, ShizukuShell.START, { it.writeString(token) }) { reply ->
                    reply.readInt() to reply.readString().orEmpty()
                }
                check(port > 0) { "the helper said ${said.ifEmpty { "nothing" }}" }
                port
            }
        }
    }

    /** Stops the helper and lets Shizuku end the service's process. */
    fun stop() {
        val serviceArgs = args ?: return
        val conn = connection
        args = null
        connection = null
        service = null
        runCatching { Shizuku.unbindUserService(serviceArgs, conn, true) }
            .onFailure { Log.i(TAG, "Shizuku didn't unbind the helper's service", it) }
    }

    private fun <T> transact(binder: IBinder, code: Int, write: (Parcel) -> Unit = {}, read: (Parcel) -> T): T {
        val data = Parcel.obtain()
        val reply = Parcel.obtain()
        try {
            data.writeInterfaceToken(ShizukuShell.DESCRIPTOR)
            write(data)
            check(binder.transact(code, data, reply, 0)) { "the helper's service refused call $code" }
            reply.readException()
            return read(reply)
        } finally {
            data.recycle()
            reply.recycle()
        }
    }

    class ShizukuProblem(val problem: Problem) : Exception("Shizuku: $problem")
}

/**
 * Runs in the process Shizuku starts for NectarLink (as Shizuku's user,
 * normally the shell), not in the app. Answers only NectarLink itself and
 * Shizuku, and only to: say which user it runs as, run a grant from
 * [ShizukuShellRules.isGrant], and start [InputServer] from NectarLink's own
 * APK with a token. Nothing it's given ends up in a shell command line.
 */
class ShizukuShell(context: Context) : Binder() {
    private val appUid = context.applicationInfo.uid
    private val apk = context.applicationInfo.sourceDir
    @Volatile private var helper: Process? = null

    override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
        val caller = getCallingUid()
        if (caller != appUid && caller != android.os.Process.myUid()) return false
        if (code == DESTROY) {
            stopHelper()
            exitProcess(0)
        }
        if (code !in UID..START) return super.onTransact(code, data, reply, flags)
        data.enforceInterface(DESCRIPTOR)
        val out = reply ?: return false
        when (code) {
            UID -> {
                out.writeNoException()
                out.writeInt(android.os.Process.myUid())
            }
            RUN -> {
                val grant = data.readString().orEmpty()
                out.writeNoException()
                out.writeInt(if (ShizukuShellRules.isGrant(grant)) run(ShizukuShellRules.words(grant)) else -1)
            }
            START -> {
                val token = data.readString().orEmpty()
                val (port, said) = if (ShizukuShellRules.isToken(token)) startHelper(token) else -1 to "bad token"
                out.writeNoException()
                out.writeInt(port)
                out.writeString(said)
            }
        }
        return true
    }

    /** Runs one grant to its end (they print next to nothing); its exit code, or -1. */
    private fun run(words: List<String>): Int = runCatching {
        val process = ProcessBuilder(words).redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.to(java.io.File("/dev/null"))).start()
        if (process.waitFor(RUN_S, TimeUnit.SECONDS)) process.exitValue() else -1.also { process.destroyForcibly() }
    }.getOrDefault(-1)

    /** Starts [InputServer]; its port, or -1 and what it said. */
    private fun startHelper(token: String): Pair<Int, String> {
        stopHelper()
        val process = runCatching {
            ProcessBuilder(APP_PROCESS, "/system/bin", InputServer::class.java.name, token)
                .redirectErrorStream(true)
                .apply { environment()["CLASSPATH"] = apk }
                .start()
        }.getOrElse { return -1 to "can't start app_process: $it" }
        val output = process.inputStream.bufferedReader()
        val reader = Executors.newSingleThreadExecutor()
        val first = runCatching { reader.submit<String> { output.readLine().orEmpty() }.get(READY_S, TimeUnit.SECONDS) }
            .getOrDefault("")
        reader.shutdownNow()
        val port = ShizukuShellRules.readyPort(first)
        if (port == null) {
            process.destroyForcibly()
            return -1 to first
        }
        helper = process
        thread(name = "elevated-helper-log") {
            runCatching { output.lineSequence().forEach { Log.d("Elevated", "shizuku-helper: $it") } }
        }
        return port to ""
    }

    private fun stopHelper() {
        helper?.let { proc ->
            runCatching {
                if (!proc.waitFor(250, TimeUnit.MILLISECONDS)) {
                    proc.destroyForcibly()
                }
            }
        }
        helper = null
    }

    companion object {
        const val DESCRIPTOR = "app.nectarlink.android.elevated.ShizukuShell"
        const val UID = FIRST_CALL_TRANSACTION
        const val RUN = FIRST_CALL_TRANSACTION + 1
        const val START = FIRST_CALL_TRANSACTION + 2
        /** What Shizuku sends when it ends the service (`USER_SERVICE_TRANSACTION_destroy`). */
        const val DESTROY = 16777115
        const val SHELL_UID = 2000
        private const val APP_PROCESS = "/system/bin/app_process"
        private const val RUN_S = 30L
        private const val READY_S = 15L
    }
}

/** What [ShizukuShell] accepts, as plain functions so they're unit-tested. */
object ShizukuShellRules {
    private val TOKEN = Regex("[0-9a-f]{48}")
    private val PACKAGE = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")
    private val OP = Regex("[A-Z][A-Z0-9_]*")
    private val PERMISSION = Regex("android\\.permission\\.[A-Z][A-Z0-9_]*")
    private val COMPONENT = Regex("${PACKAGE.pattern}/${PACKAGE.pattern}")

    /** A token [Elevated] makes: 24 random bytes in hex. */
    fun isToken(token: String) = TOKEN.matches(token)

    /**
     * One of the grants [Elevated] runs before the helper starts, for
     * NectarLink's own package and nothing else: `pm grant`, `appops set …
     * allow` and `cmd notification allow_listener|allow_dnd`.
     */
    fun isGrant(command: String): Boolean {
        val w = words(command)
        val own = app.nectarlink.android.BuildConfig.APPLICATION_ID
        return when {
            w.size == 4 && w[0] == "pm" && w[1] == "grant" -> w[2] == own && PERMISSION.matches(w[3])
            w.size == 5 && w[0] == "appops" && w[1] == "set" && w[4] == "allow" -> w[2] == own && OP.matches(w[3])
            w.size == 4 && w[0] == "cmd" && w[1] == "notification" && w[2] == "allow_dnd" -> w[3] == own
            w.size == 4 && w[0] == "cmd" && w[1] == "notification" && w[2] == "allow_listener" ->
                COMPONENT.matches(w[3]) && w[3].substringBefore('/') == own
            else -> false
        }
    }

    fun words(command: String): List<String> = command.trim().split(Regex("\\s+")).filter(String::isNotEmpty)

    /** The port in the helper's first line (`ready <port>`), if that's what it said. */
    fun readyPort(line: String): Int? =
        line.takeIf { it.startsWith("ready ") }?.removePrefix("ready ")?.trim()?.toIntOrNull()?.takeIf { it in 1..65535 }
}
