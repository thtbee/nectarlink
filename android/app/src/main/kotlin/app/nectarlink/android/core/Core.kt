// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.net.wifi.WifiManager
import android.service.notification.NotificationListenerService
import android.util.Log
import android.net.Uri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.media.PcMedia
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.files.OutgoingFiles
import app.nectarlink.android.files.ReceivedFiles
import app.nectarlink.android.files.TransferNotifications
import app.nectarlink.android.R
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.core.Event
import app.nectarlink.core.EventListener
import app.nectarlink.core.Link
import app.nectarlink.core.NectarlinkException
import app.nectarlink.core.NectarlinkNode
import app.nectarlink.core.NodeOptions
import app.nectarlink.core.Notification
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import app.nectarlink.core.initLogging
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The app's single connection to `nectarlink-core`. Owns the node, folds
 * its events into [state], feeds it the phone's battery and network
 * changes, and runs commands for the UI.
 */
class Core(context: Context, private val scope: CoroutineScope) : EventListener {
    private val context = context.applicationContext
    private val ringer = Ringer(this.context)
    /** What plays on this phone, for PCs (needs notification access). */
    private val phoneMedia = PhoneMedia(this.context) { players ->
        notificationOps.trySend { it.mediaChanged(players) }
    }
    private val platform = PhonePlatform(this.context, ringer, phoneMedia)

    /** What plays on the PCs, in Android's media controls. */
    val pcMedia = PcMedia(this.context) { pc, player, action, position ->
        command { it.mediaCommand(pc, player, action, position) }
    }
    /**
     * Notification changes, in order: a removal must never overtake the post
     * it removes. Runs once the node is up.
     */
    private val notificationOps = Channel<suspend (NectarlinkNode) -> Unit>(Channel.UNLIMITED)
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
                downloadsDir = context.cacheDir.resolve("received").absolutePath,
            )
            val started = try {
                NectarlinkNode.start(options, platform, KeystoreKeyProtector(), this@Core)
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
            started.updatePower(PowerLevel.BASIC, capabilities(NotificationListener.hasAccess(context)))
            scope.launch {
                for (op in notificationOps) {
                    runCatching { op(started) }.onFailure { Log.w(TAG, "notification update failed", it) }
                }
            }
        }
    }

    // ---- Notifications (from NotificationListener) ----

    /**
     * The notification listener connected (with what's showing now) or lost
     * access. What this phone offers PCs follows.
     */
    fun notificationAccessChanged(granted: Boolean, showing: List<Notification>) {
        _state.update { it.copy(notificationAccess = granted) }
        scope.launch(Dispatchers.Main) { if (granted) phoneMedia.start() else phoneMedia.stop() }
        notificationOps.trySend { node ->
            node.updatePower(PowerLevel.BASIC, capabilities(granted))
            node.notificationsReset(showing)
        }
    }

    fun notificationPosted(notification: Notification) {
        notificationOps.trySend { it.notificationPosted(notification) }
    }

    fun notificationRemoved(key: String) {
        notificationOps.trySend { it.notificationRemoved(key) }
    }

    /**
     * Re-checks notification access and background restrictions (the user
     * may have changed them in Settings) and asks Android to reconnect the listener if it's allowed
     * but not running.
     */
    fun refreshNotificationAccess() {
        val granted = NotificationListener.hasAccess(context)
        val unrestricted = BackgroundAccess.isUnrestricted(context)
        _state.update { it.copy(notificationAccess = granted, backgroundUnrestricted = unrestricted) }
        if (granted && NotificationListener.instance == null) {
            NotificationListenerService.requestRebind(NotificationListener.component(context))
        }
    }

    override fun onEvent(event: Event) {
        _state.update { it.reduce(event) }
        when (event) {
            is Event.Transfer -> onTransfer(event.transfer)
            is Event.MediaChanged -> pcMedia.update(event.id, _state.value.nameOf(event.id).orEmpty(), event.players)
            // A PC that's gone isn't playing for this phone anymore; it
            // sends what plays when it's back.
            is Event.LinkChanged -> if (event.link is Link.Offline) pcMedia.update(event.id, "", emptyList())
            is Event.DeviceRemoved -> pcMedia.update(event.id, "", emptyList())
            else -> {}
        }
    }

    // ---- Files ----

    private fun onTransfer(transfer: Transfer) {
        val pc = _state.value.nameOf(transfer.deviceId).orEmpty()
        TransferNotifications.update(context, transfer, pc)
        val done = transfer.status as? TransferStatus.Done ?: return
        if (transfer.direction != TransferDirection.INCOMING) return
        // Out of the app's cache, into Downloads, then tell the user.
        scope.launch(Dispatchers.IO) {
            val published = done.saved.mapNotNull { ReceivedFiles.publish(context, java.io.File(it)) }
            TransferNotifications.received(context, transfer, published, pc)
        }
    }

    /**
     * Sends picked or shared files to a PC; problems arrive as messages.
     * The files are opened right away: Android's permission to read a
     * shared item ends with the activity that received it.
     */
    fun sendFiles(pcId: String, uris: List<Uri>) {
        val files = OutgoingFiles.open(context, uris)
        if (files.isEmpty()) {
            _messages.tryEmit(context.getString(R.string.transfer_nothing))
            return
        }
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val node = node ?: return@launch
            try {
                node.sendFiles(pcId, files)
            } catch (e: NectarlinkException) {
                val name = _state.value.nameOf(pcId).orEmpty()
                _messages.tryEmit(
                    if (e is NectarlinkException.Denied) context.getString(R.string.transfer_off_for, name) else describe(e),
                )
            }
        }
    }

    fun cancelTransfer(id: String) {
        node?.cancelTransfer(id)
    }

    // ---- Clipboard ----

    /**
     * Sends text to every connected PC; returns what to tell the user.
     * Waits for the core when the app was just launched to send.
     */
    suspend fun sendClipboard(text: String): String =
        sendToPcs(image = false) { node, pc -> node.sendClipboard(pc, text) }

    /** Sends a copied image to the connected PCs; returns what to tell the user. */
    suspend fun sendClipboardImage(uri: Uri): String {
        startJob?.join()
        if (_state.value.devices.none { it.online }) return context.getString(R.string.clip_no_pc)
        val image = when (val read = withContext(Dispatchers.IO) { PhoneClipboard.readImage(context, uri) }) {
            is PhoneClipboard.ImageResult.Ready -> read.image
            PhoneClipboard.ImageResult.TooLarge -> return context.getString(R.string.clip_image_too_large)
            PhoneClipboard.ImageResult.Unreadable -> return context.getString(R.string.clip_image_unreadable)
        }
        return sendToPcs(image = true) { node, pc -> node.sendClipboardImage(pc, image.mime, image.bytes) }
    }

    private suspend fun sendToPcs(image: Boolean, send: suspend (NectarlinkNode, String) -> Unit): String {
        startJob?.join()
        val node = node ?: return context.getString(R.string.clip_no_pc)
        val pcs = _state.value.devices.filter { it.online }
        if (pcs.isEmpty()) return context.getString(R.string.clip_no_pc)
        val sentTo = mutableListOf<String>()
        var failure: String? = null
        for (pc in pcs) {
            try {
                send(node, pc.id)
                sentTo += pc.name
            } catch (e: NectarlinkException) {
                failure = when (e) {
                    is NectarlinkException.Denied -> context.getString(R.string.clip_off_for, pc.name)
                    is NectarlinkException.TooLarge -> context.getString(
                        if (image) R.string.clip_image_too_large else R.string.clip_too_large,
                    )
                    is NectarlinkException.Unsupported ->
                        if (image) context.getString(R.string.clip_image_unsupported, pc.name) else describe(e)
                    else -> describe(e)
                }
            }
        }
        return if (sentTo.isNotEmpty()) context.getString(R.string.clip_sent_to, sentTo.joinToString()) else failure.orEmpty()
    }

    /** What this phone offers PCs (docs/protocol/capabilities.md). */
    private fun capabilities(notificationAccess: Boolean): List<String> =
        CLIPBOARD_CAPABILITIES + if (notificationAccess) NOTIFICATION_CAPABILITIES + MEDIA_CAPABILITIES else emptyList()

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

    /** Reconnects to PCs that aren't connected and syncs connected ones. */
    fun refresh() {
        refreshNotificationAccess()
        command { it.refresh() }
    }

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
        /** Offered while notification access is granted (docs/protocol/capabilities.md). */
        val NOTIFICATION_CAPABILITIES = listOf("notify.mirror", "notify.reply")
        /** Sharing this phone's players, which also needs notification access. */
        val MEDIA_CAPABILITIES = listOf("media.control")
        /** Accepting the PC's clipboard, and sending this one when asked. */
        val CLIPBOARD_CAPABILITIES = listOf("clip.write", "clip.share")
    }
}
