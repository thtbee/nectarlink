// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.os.ParcelFileDescriptor
import app.nectarlink.services.IDisplaySession
import app.nectarlink.services.client.NectarlinkServiceClient
import java.io.Closeable
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Video stream for full-screen mirroring or an app window (socket or AIDL session). */
interface ElevatedDisplayStream : Closeable {
    val inputStream: InputStream
    fun requestKeyframe()
    fun resize(width: Int, height: Int, dpi: Int)
}

internal class SocketDisplayStream(private val socket: Socket) : ElevatedDisplayStream {
    override val inputStream: InputStream = socket.getInputStream()

    override fun requestKeyframe() {
        runCatching {
            synchronized(socket) {
                socket.getOutputStream().apply {
                    write("K\n".toByteArray())
                    flush()
                }
            }
        }
    }

    override fun resize(width: Int, height: Int, dpi: Int) {
        runCatching {
            synchronized(socket) {
                socket.getOutputStream().apply {
                    write("R $width $height $dpi\n".toByteArray())
                    flush()
                }
            }
        }
    }

    override fun close() {
        runCatching { socket.close() }
    }
}

internal class BinderDisplayStream(
    private val session: IDisplaySession,
    pfd: ParcelFileDescriptor,
) : ElevatedDisplayStream {
    override val inputStream: InputStream = ParcelFileDescriptor.AutoCloseInputStream(pfd)

    override fun requestKeyframe() {
        runCatching { session.requestKeyframe() }
    }

    override fun resize(width: Int, height: Int, dpi: Int) {
        runCatching { session.resize(width, height, dpi) }
    }

    override fun close() {
        runCatching { inputStream.close() }
        runCatching { session.close() }
    }
}

/** Device playback audio stream (48 kHz stereo 16-bit PCM). */
interface ElevatedAudioStream : Closeable {
    val inputStream: InputStream
}

internal class SocketAudioStream(private val socket: Socket) : ElevatedAudioStream {
    override val inputStream: InputStream = socket.getInputStream()

    override fun close() {
        runCatching { socket.close() }
    }
}

internal class PfdAudioStream(pfd: ParcelFileDescriptor) : ElevatedAudioStream {
    override val inputStream: InputStream = ParcelFileDescriptor.AutoCloseInputStream(pfd)

    override fun close() {
        runCatching { inputStream.close() }
    }
}

/** Bidirectional PSTN call audio stream (16 kHz mono 16-bit PCM). */
interface ElevatedCallAudioStream : Closeable {
    val inputStream: InputStream
    val outputStream: OutputStream
}

internal class SocketCallAudioStream(private val socket: Socket) : ElevatedCallAudioStream {
    override val inputStream: InputStream = socket.getInputStream()
    override val outputStream: OutputStream = socket.getOutputStream()

    override fun close() {
        runCatching { socket.close() }
    }
}

internal class PfdCallAudioStream(private val pfd: ParcelFileDescriptor) : ElevatedCallAudioStream {
    override val inputStream: InputStream = FileInputStream(pfd.fileDescriptor)
    override val outputStream: OutputStream = FileOutputStream(pfd.fileDescriptor)

    override fun close() {
        runCatching { inputStream.close() }
        runCatching { outputStream.close() }
        runCatching { pfd.close() }
    }
}

/** Shell commands translated to typed IPC calls on [NectarlinkServiceClient]. */
internal sealed interface SystemServiceCommand {
    data object ExpandNotifications : SystemServiceCommand
    data class SetWifi(val enabled: Boolean) : SystemServiceCommand
    data class SetBluetooth(val enabled: Boolean) : SystemServiceCommand
    data class SetHotspot(val enabled: Boolean) : SystemServiceCommand
    data class SetBrightness(val value: Int) : SystemServiceCommand
    data class UninstallPackage(val pkg: String) : SystemServiceCommand
    data object ProvisionPermissions : SystemServiceCommand
    data object LaunchPcStartActivity : SystemServiceCommand
}

internal object SystemServiceBridge {
    private const val BRIGHTNESS_PREFIX = "settings put system screen_brightness "
    private const val UNINSTALL_PREFIX = "pm uninstall "
    private const val OWN_PACKAGE = "app.nectarlink"
    private val PACKAGE = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")

    private val handshakePool = Executors.newCachedThreadPool { r ->
        Thread(r, "nectarlink-service-handshake").apply { isDaemon = true }
    }

    fun parseCommand(cmd: String, ownPackage: String): SystemServiceCommand? = when {
        cmd == "cmd statusbar expand-notifications" -> SystemServiceCommand.ExpandNotifications
        cmd == "svc wifi enable" -> SystemServiceCommand.SetWifi(true)
        cmd == "svc wifi disable" -> SystemServiceCommand.SetWifi(false)
        cmd == "svc bluetooth enable" -> SystemServiceCommand.SetBluetooth(true)
        cmd == "svc bluetooth disable" -> SystemServiceCommand.SetBluetooth(false)
        cmd == "hotspot on" || cmd == "cmd wifi start-softap" -> SystemServiceCommand.SetHotspot(true)
        cmd == "hotspot off" || cmd == "cmd wifi stop-softap" -> SystemServiceCommand.SetHotspot(false)
        cmd == "appops set $ownPackage GET_USAGE_STATS allow" -> SystemServiceCommand.ProvisionPermissions
        cmd == "am start -n $ownPackage/app.nectarlink.android.service.PcStartActivity" ->
            SystemServiceCommand.LaunchPcStartActivity
        cmd.startsWith(BRIGHTNESS_PREFIX) -> {
            val v = cmd.removePrefix(BRIGHTNESS_PREFIX).toIntOrNull()
            if (v != null && v in 0..255) SystemServiceCommand.SetBrightness(v) else null
        }
        cmd.startsWith(UNINSTALL_PREFIX) -> {
            val pkg = cmd.removePrefix(UNINSTALL_PREFIX)
            if (PACKAGE.matches(pkg) && pkg != ownPackage && pkg != OWN_PACKAGE && !pkg.startsWith("$OWN_PACKAGE.")) {
                SystemServiceCommand.UninstallPackage(pkg)
            } else {
                null
            }
        }
        else -> null
    }

    fun executeCommand(client: NectarlinkServiceClient, cmd: String, ownPackage: String): Boolean {
        return when (val parsed = parseCommand(cmd, ownPackage)) {
            SystemServiceCommand.ExpandNotifications -> client.expandNotifications()
            is SystemServiceCommand.SetWifi -> client.setWifiEnabled(parsed.enabled)
            is SystemServiceCommand.SetBluetooth -> client.setBluetoothEnabled(parsed.enabled)
            is SystemServiceCommand.SetHotspot -> client.setHotspotEnabled(parsed.enabled)
            is SystemServiceCommand.SetBrightness -> client.setScreenBrightness(parsed.value)
            is SystemServiceCommand.UninstallPackage -> client.uninstallPackage(parsed.pkg)
            SystemServiceCommand.ProvisionPermissions -> client.provisionAppPermissions(ownPackage)
            SystemServiceCommand.LaunchPcStartActivity ->
                client.launchActivityFromBackground(ownPackage, "app.nectarlink.android.service.PcStartActivity")
            null -> false
        }
    }

    /** Reads the first line (`OK` or `ERR ...`) from a call-audio socket pair with a timeout. */
    fun readCallAudioAnswer(input: InputStream, timeoutMs: Long, onTimeout: () -> Unit): String? {
        val future = handshakePool.submit<String?> {
            val line = StringBuilder()
            while (true) {
                val b = input.read()
                if (b < 0 || b == '\n'.code || line.length > 256) break
                line.append(b.toChar())
            }
            line.toString()
        }
        return try {
            future.get(timeoutMs, TimeUnit.MILLISECONDS)
        } catch (_: Exception) {
            onTimeout()
            future.cancel(true)
            null
        }
    }
}
