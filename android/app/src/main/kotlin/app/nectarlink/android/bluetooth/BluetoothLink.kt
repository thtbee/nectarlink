// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.bluetooth

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.content.ContextCompat
import app.nectarlink.core.bluetoothServiceUuid
import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.Socket
import java.util.UUID

/**
 * Carries the session over Bluetooth when a PC reaches this phone no other
 * way (#313): listens for RFCOMM links from PCs this phone is bonded with and
 * pipes each one into the core's loopback port, which does the rest (the
 * same encrypted session, only light messages over it).
 *
 * Secure RFCOMM only: the PC has to be paired in Android's Bluetooth
 * settings, and NectarLink's own pairing still decides who's let in.
 */
class BluetoothLink(context: Context) {
    private val context = context.applicationContext
    private val lock = Any()
    private var port = 0
    private var server: BluetoothServerSocket? = null
    private var acceptThread: Thread? = null
    private val links = mutableSetOf<Closeable>()
    private var receiver: BroadcastReceiver? = null

    /** Starts listening, piping links into `127.0.0.1:[corePort]`. */
    fun start(corePort: Int) {
        synchronized(lock) {
            if (port == corePort && receiver != null) return
            stopLocked()
            port = corePort
            val stateReceiver = object : BroadcastReceiver() {
                override fun onReceive(c: Context, intent: Intent) {
                    when (intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR)) {
                        BluetoothAdapter.STATE_ON -> synchronized(lock) { if (port != 0) listenLocked() }
                        BluetoothAdapter.STATE_TURNING_OFF, BluetoothAdapter.STATE_OFF ->
                            synchronized(lock) { closeServerLocked() }
                    }
                }
            }
            ContextCompat.registerReceiver(
                context,
                stateReceiver,
                IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED),
                ContextCompat.RECEIVER_NOT_EXPORTED,
            )
            receiver = stateReceiver
            listenLocked()
        }
    }

    fun stop() {
        synchronized(lock) { stopLocked() }
    }

    private fun stopLocked() {
        receiver?.let { runCatching { context.unregisterReceiver(it) } }
        receiver = null
        port = 0
        closeServerLocked()
        links.toList().forEach { runCatching { it.close() } }
        links.clear()
    }

    private fun closeServerLocked() {
        runCatching { server?.close() }
        server = null
        acceptThread = null
    }

    @SuppressLint("MissingPermission")
    private fun listenLocked() {
        if (server != null || !hasPermission(context)) return
        val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter ?: return
        if (!adapter.isEnabled) return
        val listening = try {
            adapter.listenUsingRfcommWithServiceRecord(SERVICE_NAME, serviceUuid)
        } catch (e: IOException) {
            Log.w(TAG, "can't listen for Bluetooth links", e)
            return
        } catch (e: SecurityException) {
            Log.w(TAG, "no Bluetooth permission", e)
            return
        }
        server = listening
        val corePort = port
        acceptThread = Thread({ acceptLoop(listening, corePort) }, "bt-link-accept").apply {
            isDaemon = true
            start()
        }
        Log.i(TAG, "listening for Bluetooth links")
    }

    private fun acceptLoop(listening: BluetoothServerSocket, corePort: Int) {
        while (true) {
            val socket = try {
                listening.accept()
            } catch (_: IOException) {
                return
            }
            Thread({ pipe(socket, corePort) }, "bt-link").apply {
                isDaemon = true
                start()
            }
        }
    }

    /** Copies bytes both ways between [bt] and the core until either side ends. */
    private fun pipe(bt: BluetoothSocket, corePort: Int) {
        val core = try {
            Socket(InetAddress.getLoopbackAddress(), corePort).apply { tcpNoDelay = true }
        } catch (e: IOException) {
            Log.w(TAG, "the core isn't listening for Bluetooth links", e)
            runCatching { bt.close() }
            return
        }
        synchronized(lock) {
            if (port != corePort) {
                runCatching { bt.close() }
                runCatching { core.close() }
                return
            }
            links += bt
            links += core
        }
        val closeBoth = {
            runCatching { bt.close() }
            runCatching { core.close() }
            synchronized(lock) {
                links -= bt
                links -= core
            }
        }
        val up = Thread({ copy(bt.inputStream, core.getOutputStream()); closeBoth() }, "bt-link-up")
        up.isDaemon = true
        up.start()
        copy(core.getInputStream(), bt.outputStream)
        closeBoth()
    }

    private fun copy(from: InputStream, to: OutputStream) {
        val buf = ByteArray(BUFFER_BYTES)
        try {
            while (true) {
                val n = from.read(buf)
                if (n < 0) return
                to.write(buf, 0, n)
                to.flush()
            }
        } catch (_: IOException) {
        }
    }

    companion object {
        private const val TAG = "BluetoothLink"
        private const val SERVICE_NAME = "NectarLink"
        private const val BUFFER_BYTES = 16 * 1024

        private val serviceUuid: UUID by lazy { UUID.fromString(bluetoothServiceUuid()) }

        /** The permission a Bluetooth link needs, if this Android asks for one at run time. */
        val permission: String? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) Manifest.permission.BLUETOOTH_CONNECT else null

        fun hasPermission(context: Context): Boolean = permission?.let {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        } ?: true
    }
}
