// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import app.nectarlink.android.BuildConfig
import app.nectarlink.core.Event
import app.nectarlink.core.EventListener
import app.nectarlink.core.NectarlinkException
import app.nectarlink.core.NectarlinkNode
import app.nectarlink.core.NodeOptions
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
import app.nectarlink.core.initLogging
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * The app's single connection to `nectarlink-core`. Owns the node, folds
 * its events into [state], feeds it the phone's battery and network
 * changes, and runs commands for the UI.
 */
class Core(context: Context, private val scope: CoroutineScope) : EventListener {
    private val context = context.applicationContext
    private val ringer = Ringer(this.context)
    private val _state = MutableStateFlow(CoreState())
    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    private var node: NectarlinkNode? = null
    private var startJob: Job? = null
    private var networkChange: Job? = null

    private val battery = BatteryMonitor(this.context) { battery ->
        node?.let { scope.launch { it.updateBattery(battery) } }
    }
    private val network = NetworkMonitor(this.context) {
        // Network callbacks come in bursts; tell the core once they settle.
        networkChange?.cancel()
        networkChange = scope.launch {
            delay(1_000)
            node?.networkChanged()
        }
    }

    // Android drops multicast (local discovery) unless a lock is held. The
    // connection service holds one while paired; this one covers pairing.
    private val pairingMulticast: WifiManager.MulticastLock? =
        this.context.getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-pairing")
            ?.apply { setReferenceCounted(false) }

    val state: StateFlow<CoreState> = _state.asStateFlow()

    /** Short messages for the user (a command failed). */
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** Starts the node (once). Safe to call repeatedly. */
    fun start() {
        if (startJob != null) return
        startJob = scope.launch(Dispatchers.IO) {
            initLogging(if (BuildConfig.DEBUG) "debug,iroh=info,swarm_discovery=error" else "info,iroh=warn,swarm_discovery=error")
            val options = NodeOptions(
                dataDir = context.noBackupFilesDir.resolve("core").absolutePath,
                device = deviceInfo(context),
                appVersion = BuildConfig.VERSION_NAME,
                power = PowerLevel.BASIC,
                awayMode = false,
            )
            val started = try {
                NectarlinkNode.start(options, ringer, KeystoreKeyProtector(), this@Core)
            } catch (e: NectarlinkException) {
                Log.e(TAG, "core failed to start", e)
                _state.update { it.copy(status = CoreStatus.Failed(e.message ?: e.javaClass.simpleName)) }
                return@launch
            }
            node = started
            val devices = runCatching { started.pairedDevices() }.getOrDefault(emptyList())
            _state.update { it.withDevices(devices).copy(status = CoreStatus.Ready(started.deviceId())) }
            scope.launch(Dispatchers.Main) {
                battery.start()
                network.start()
            }
        }
    }

    override fun onEvent(event: Event) {
        _state.update { it.reduce(event) }
    }

    // ---- Pairing ----

    /**
     * Pairs with a link that arrived before the core finished starting (the
     * app was launched by opening the link).
     */
    fun joinWhenReady(link: String) {
        scope.launch {
            startJob?.join()
            joinWithLink(link)
        }
    }

    /** Pairs with the PC whose QR code was scanned. */
    fun joinWithLink(link: String) = command {
        _state.update { it.copy(pairing = PairingState.Joining) }
        try {
            it.pairingJoin(link)
        } catch (e: NectarlinkException) {
            val failure = when (e) {
                is NectarlinkException.InvalidPairingLink -> PairingFailure.Other("not a Nectarlink code")
                is NectarlinkException.Denied -> PairingFailure.Rejected
                else -> PairingFailure.Unreachable
            }
            _state.update { s -> s.copy(pairing = PairingState.Failed(failure)) }
        }
    }

    /** Pairs with a nearby PC that shows its pairing screen. */
    fun pairNearby(peer: String) = command {
        _state.update { it.copy(pairing = PairingState.Connecting(peer)) }
        try {
            it.pairingStartNearby(peer)
        } catch (e: NectarlinkException) {
            _state.update { s ->
                if (s.pairing is PairingState.Connecting) s.copy(pairing = PairingState.Failed(PairingFailure.Unreachable)) else s
            }
        }
    }

    fun confirmCode(matches: Boolean) {
        val comparing = _state.value.pairing as? PairingState.Comparing ?: return
        runCatching { node?.pairingConfirm(matches) }
        _state.update {
            it.copy(
                pairing = if (matches) PairingState.Confirmed(comparing.peer, comparing.code)
                else PairingState.Failed(PairingFailure.Declined),
            )
        }
    }

    /** Keeps local discovery working while the pairing screen is open. */
    fun setPairingScreenOpen(open: Boolean) {
        pairingMulticast?.runCatching { if (open) acquire() else release() }
    }

    fun resetPairing() {
        node?.pairingCancel()
        _state.update { it.copy(pairing = PairingState.Idle) }
    }

    // ---- Devices ----

    fun ring(id: String, on: Boolean) = command { it.ring(id, on) }

    fun unpair(id: String) = command { it.unpair(id) }

    /** Stops this phone ringing. */
    fun stopRinging() {
        ringer.stopRinging()
        _state.update { it.copy(ringingFrom = null) }
    }

    /** Runs a command on the node, turning errors into a message. */
    private fun command(block: suspend (NectarlinkNode) -> Unit) {
        val node = node ?: return
        scope.launch {
            try {
                block(node)
            } catch (e: NectarlinkException) {
                _messages.tryEmit(describe(e))
            }
        }
    }

    private fun describe(e: NectarlinkException): String = when (e) {
        is NectarlinkException.Offline -> "The PC isn't connected right now."
        is NectarlinkException.NotPaired -> "That PC isn't paired anymore."
        is NectarlinkException.Timeout -> "The PC didn't answer in time."
        is NectarlinkException.Unsupported -> "The PC's app doesn't support that yet."
        else -> "Something went wrong."
    }

    private companion object {
        const val TAG = "Nectarlink"
    }
}
