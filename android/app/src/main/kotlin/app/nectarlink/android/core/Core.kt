// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.service.notification.NotificationListenerService
import android.util.Log
import android.net.Uri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.links.LinkNotifications
import app.nectarlink.android.media.PcMedia
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.files.OutgoingFiles
import app.nectarlink.android.files.ReceivedFiles
import app.nectarlink.android.files.TransferNotifications
import app.nectarlink.android.R
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.calls.CallCompanion
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mirror.AppWindows
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.mirror.MirrorRequests
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.recorder.DeliveryState
import app.nectarlink.android.recorder.RecordingsStore
import app.nectarlink.android.recorder.SavedRecording
import app.nectarlink.android.service.ConnectionService
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.core.Event
import app.nectarlink.core.EventListener
import app.nectarlink.core.FileToSend
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.Link
import app.nectarlink.core.NectarlinkException
import app.nectarlink.core.NectarlinkNode
import app.nectarlink.core.NodeOptions
import app.nectarlink.core.Notification
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
import app.nectarlink.core.RecordingMarker
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
    private val platform = PhonePlatform(
        this.context,
        ringer,
        phoneMedia,
        onLink = { pc, url -> LinkNotifications.show(this.context, _state.value.nameOf(pc).orEmpty(), url) },
        calls = { calls },
        contacts = { contacts },
        sms = { sms },
        toggles = { toggles },
        onMirror = { pc, request ->
            MirrorRequests.show(this.context, request, _state.value.nameOf(pc).orEmpty())
        },
        appWindows = AppWindows(this.context, scope, open = { pc -> mirrorOpen(pc) }, nameOf = { pc -> _state.value.nameOf(pc).orEmpty() }),
    )
    private val toggles = PhoneToggles(
        this.context,
        onChanged = { state ->
            notificationOps.trySend { node ->
                runCatching { node.togglesChanged(state) }.onFailure { Log.i(TAG, "toggles weren't reported", it) }
            }
        },
        onCapabilitiesChanged = { refreshNotificationAccess() },
    )
    private val sms = PhoneSms(this.context) {
        notificationOps.trySend { it.smsChanged(null) }
    }
    private val calls = PhoneCalls(
        this.context,
        onChange = { call ->
            notificationOps.trySend { node ->
                runCatching { node.callChanged(call) }.onFailure { Log.i(TAG, "a call wasn't reported", it) }
            }
        },
        onLogChanged = {
            notificationOps.trySend { it.callLogChanged() }
        },
    )
    private val contacts = PhoneContacts(this.context) {
        notificationOps.trySend { it.contactsChanged() }
    }
    private val photos = RecentPhotos(
        this.context,
        onPhoto = { photo ->
            notificationOps.trySend { node ->
                runCatching { node.photoTaken(photo) }.onFailure { Log.i(TAG, "a new photo wasn't announced", it) }
            }
        },
        onChanged = {
            notificationOps.trySend { it.photosChanged() }
        },
    )

    /** What plays on the PCs, in Android's media controls. */
    val pcMedia = PcMedia(this.context) { pc, player, action, position ->
        command { it.mediaCommand(pc, player, action, position) }
    }
    /**
     * Notification changes, in order: a removal must never overtake the post
     * it removes. Runs once the node is up.
     */
    private val notificationOps = Channel<suspend (NectarlinkNode) -> Unit>(Channel.UNLIMITED)

    /**
     * Remote input for PCs, sent one after another: a button's release must
     * never overtake its press (or the PC's button would stay down), nor a
     * laser update its "off".
     */
    private val remoteOps = Channel<suspend (NectarlinkNode) -> Unit>(Channel.UNLIMITED)
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
            // A new network turns wireless debugging off; Elevated turns it back on.
            if (!Elevated.running) Elevated.start()
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
            if (devices.isNotEmpty()) ConnectionService.start(context)
            scope.launch(Dispatchers.Main) {
                battery.start()
                network.start()
            }
            _state.update {
                it.copy(
                    photoAccess = RecentPhotos.hasAccess(context),
                    photoPartialAccess = RecentPhotos.hasPartialAccess(context),
                    callAccess = PhoneCalls.hasAll(context),
                    contactsAccess = PhoneContacts.canRead(context),
                    smsAccess = PhoneSms.hasAll(context),
                    dndAccess = PhoneToggles.hasDndAccess(context),
                    writeSettingsAccess = PhoneToggles.hasWriteSettings(context),
                )
            }
            started.updatePower(powerLevel(), capabilities(NotificationListener.hasAccess(context)))
            // Elevated, when set up and wireless debugging is on.
            scope.launch { Elevated.start() }
            scope.launch(Dispatchers.Main) {
                toggles.start()
                if (_state.value.photoAccess) photos.start()
                if (PhoneCalls.canFollow(context) || PhoneCalls.canReadLog(context)) calls.start()
                if (PhoneContacts.canRead(context)) contacts.start()
                if (PhoneSms.canRead(context)) sms.start()
            }
            scope.launch {
                for (op in notificationOps) {
                    runCatching { op(started) }.onFailure { Log.w(TAG, "notification update failed", it) }
                }
            }
            scope.launch {
                for (op in remoteOps) {
                    runCatching { op(started) }.onFailure { Log.w(TAG, "remote input failed", it) }
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
        if (granted && _state.value.devices.isNotEmpty()) ConnectionService.start(context)
        scope.launch(Dispatchers.Main) { if (granted) phoneMedia.start() else phoneMedia.stop() }
        notificationOps.trySend { node ->
            node.updatePower(powerLevel(), capabilities(granted))
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
        val photoAccess = RecentPhotos.hasAccess(context)
        val photoPartialAccess = RecentPhotos.hasPartialAccess(context)
        val callAccess = PhoneCalls.hasAll(context)
        val contactsAccess = PhoneContacts.canRead(context)
        val smsAccess = PhoneSms.hasAll(context)
        val inputAccess = InputService.running
        val elevated = Elevated.running
        val dndAccess = PhoneToggles.hasDndAccess(context)
        val writeSettingsAccess = PhoneToggles.hasWriteSettings(context)
        val localNetwork = LocalNetwork.granted(context)
        if (localNetwork && !_state.value.localNetwork) {
            // Just allowed: reach the PCs now rather than at the next retry.
            scope.launch { node?.networkChanged() }
        }
        val prevPhotos = _state.value.photoAccess
        _state.update {
            it.copy(
                notificationAccess = granted,
                backgroundUnrestricted = unrestricted,
                photoAccess = photoAccess,
                photoPartialAccess = photoPartialAccess,
                callAccess = callAccess,
                contactsAccess = contactsAccess,
                smsAccess = smsAccess,
                inputAccess = inputAccess,
                elevated = elevated,
                dndAccess = dndAccess,
                writeSettingsAccess = writeSettingsAccess,
                localNetwork = localNetwork,
            )
        }
        if (granted && NotificationListener.instance == null) {
            NotificationListenerService.requestRebind(NotificationListener.component(context))
        }
        if (node != null) {
            scope.launch(Dispatchers.Main) {
                toggles.start()
                if (photoAccess) photos.start() else photos.stop()
                if (PhoneCalls.canFollow(context) || PhoneCalls.canReadLog(context)) calls.start() else calls.stop()
                if (contactsAccess) contacts.start() else contacts.stop()
                if (PhoneSms.canRead(context)) sms.start() else sms.stop()
            }
            notificationOps.trySend {
                it.updatePower(powerLevel(), capabilities(_state.value.notificationAccess))
                if (photoAccess && (!prevPhotos || photoPartialAccess)) it.photosChanged()
            }
        }
    }

    /** Voice recordings stored on this phone and their delivery status to PCs. */
    val recordings = RecordingsStore(this.context)

    override fun onEvent(event: Event) {
        _state.update { it.reduce(event) }
        when (event) {
            is Event.Transfer -> onTransfer(event.transfer)
            is Event.MediaChanged -> pcMedia.update(event.id, _state.value.nameOf(event.id).orEmpty(), event.players)
            // A PC that's gone isn't playing for this phone anymore; it
            // sends what plays when it's back.
            is Event.LinkChanged -> {
                if (event.link is Link.Offline) {
                    pcMedia.update(event.id, "", emptyList())
                } else if (event.link is Link.Online) {
                    retryWaitingRecordings(event.id)
                }
            }
            is Event.DeviceAdded -> ConnectionService.start(context)
            is Event.DeviceRemoved -> {
                pcMedia.update(event.id, "", emptyList())
                if (_state.value.devices.isEmpty()) ConnectionService.stop(context)
            }
            is Event.Paired -> ConnectionService.start(context)
            else -> {}
        }
    }

    // ---- Files & Recordings ----

    private fun onTransfer(transfer: Transfer) {
        val pc = _state.value.nameOf(transfer.deviceId).orEmpty()
        val pcOnline = _state.value.device(transfer.deviceId)?.online == true
        recordings.onTransferEvent(transfer, pcOnline)
        TransferNotifications.update(context, transfer, pc)
        val done = transfer.status as? TransferStatus.Done ?: return
        if (transfer.direction != TransferDirection.INCOMING) return
        // Out of the app's cache, into Downloads, then tell the user.
        scope.launch(Dispatchers.IO) {
            val published = done.saved.flatMap { ReceivedFiles.publish(context, java.io.File(it)) }
            TransferNotifications.received(context, transfer, published, pc)
        }
    }

    /** Saves a finished voice recording and sends it to the chosen PC. */
    fun onRecordingFinished(
        pcId: String,
        file: java.io.File,
        durationMs: Long,
        markers: List<RecordingMarker>,
    ): SavedRecording {
        val online = _state.value.device(pcId)?.online == true
        val saved = recordings.add(
            file = file,
            durationMs = durationMs,
            markers = markers,
            targetPcId = pcId,
            waiting = !online,
        )
        sendSavedRecording(pcId, saved.id)
        return saved
    }

    /** Sends (or re-sends) a saved voice recording and its markers to `pcId`. */
    fun sendSavedRecording(pcId: String, recordingId: String) {
        val rec = recordings.get(recordingId) ?: return
        val file = recordings.fileFor(rec)
        if (!file.exists()) return
        val online = _state.value.device(pcId)?.online == true
        recordings.updateDelivery(
            id = recordingId,
            pcId = pcId,
            transferId = rec.transferId,
            delivery = if (online) DeliveryState.Sending else DeliveryState.Waiting,
        )
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val node = node ?: return@launch
            try {
                val tid = node.sendRecording(
                    pcId,
                    FileToSend.Path(
                        path = file.absolutePath,
                        name = file.name,
                        folder = null,
                    ),
                    rec.markers,
                )
                val nowOnline = _state.value.device(pcId)?.online == true
                val existing = _state.value.transfers.firstOrNull { it.id == tid }
                val delivery = when (existing?.status) {
                    is TransferStatus.Done -> DeliveryState.Sent
                    is TransferStatus.Running -> DeliveryState.Sending
                    is TransferStatus.Waiting -> DeliveryState.Waiting
                    else -> if (nowOnline) DeliveryState.Sending else DeliveryState.Waiting
                }
                recordings.updateDelivery(
                    id = recordingId,
                    pcId = pcId,
                    transferId = tid,
                    delivery = delivery,
                )
            } catch (e: NectarlinkException) {
                val name = _state.value.nameOf(pcId).orEmpty()
                when (e) {
                    is NectarlinkException.Denied -> {
                        recordings.updateDelivery(recordingId, pcId, null, DeliveryState.Denied)
                        _messages.tryEmit(context.getString(R.string.recorder_denied_text, name))
                    }
                    is NectarlinkException.Offline -> {
                        recordings.updateDelivery(recordingId, pcId, null, DeliveryState.Waiting)
                    }
                    else -> {
                        recordings.updateDelivery(recordingId, pcId, null, DeliveryState.Failed)
                        _messages.tryEmit(describe(e))
                    }
                }
            }
        }
    }

    private fun retryWaitingRecordings(pcId: String) {
        val activeIds = _state.value.transfers.filterNot { it.isFinished() }.map { it.id }.toSet()
        for (rec in recordings.waitingForPc(pcId)) {
            if (rec.transferId == null || rec.transferId !in activeIds) {
                sendSavedRecording(pcId, rec.id)
            }
        }
    }

    /** Opens this phone's screen stream to a PC (after the user agreed). */
    suspend fun mirrorOpen(pcId: String): MirrorStream {
        startJob?.join()
        val node = checkNotNull(node) { "the core isn't running" }
        return node.mirrorOpen(pcId)
    }

    /** Opens this phone's sound stream to a PC that asked for it with the screen. */
    suspend fun mirrorOpenAudio(pcId: String): MirrorStream {
        startJob?.join()
        val node = checkNotNull(node) { "the core isn't running" }
        return node.mirrorOpenAudio(pcId)
    }

    /**
     * Sends picked or shared files to a PC; problems arrive as messages.
     * The files are opened right away: Android's permission to read a
     * shared item ends with the activity that received it.
     */
    fun sendFiles(pcId: String, uris: List<Uri>) = send(pcId, OutgoingFiles.open(context, uris))

    /** Sends a folder the user picked, with everything in it. */
    fun sendFolder(pcId: String, tree: Uri) {
        scope.launch(Dispatchers.IO) {
            val files = try {
                OutgoingFiles.openFolder(context, tree)
            } catch (_: OutgoingFiles.TooManyFiles) {
                _messages.tryEmit(context.getString(R.string.transfer_too_many))
                return@launch
            } catch (e: Exception) {
                Log.w(TAG, "can't read a picked folder", e)
                emptyList()
            }
            send(pcId, files)
        }
    }

    private fun send(pcId: String, files: List<FileToSend>) {
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
        CLIPBOARD_CAPABILITIES +
            toggles.capabilities() +
            (if (notificationAccess) NOTIFICATION_CAPABILITIES + MEDIA_CAPABILITIES else emptyList()) +
            (if (_state.value.photoAccess) PHOTO_CAPABILITIES else emptyList()) +
            (if (PhoneCalls.canFollow(context)) listOf("call.state") else emptyList()) +
            (if (PhoneCalls.canControl(context)) listOf("call.control") else emptyList()) +
            (if (PhoneCalls.canControl(context) && CallCompanion.allowed(context)) listOf("call.incall") else emptyList()) +
            (if (PhoneCalls.canReadLog(context)) listOf("call.log") else emptyList()) +
            (if (PhoneCalls.canDial(context)) listOf("call.dial") else emptyList()) +
            (if (PhoneContacts.canRead(context)) listOf("contacts.read") else emptyList()) +
            (if (PhoneSms.canRead(context)) listOf("sms.read") else emptyList()) +
            (if (PhoneSms.canSend(context)) listOf("sms.send") else emptyList()) +
            (if (InputService.running || Elevated.running) listOf("mirror.input") else emptyList()) +
            // Apps in windows of their own run on displays the Elevated helper makes.
            (if (Elevated.running && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) listOf("mirror.virtual_display") else emptyList()) +
            // Asked for when a PC wants the sound.
            (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) listOf("mirror.audio.playback") else emptyList())

    /**
     * Elevated while the wireless debugging helper runs, Assist when the
     * user turned on control from the PC (docs/PLAN.md §4.6).
     */
    private fun powerLevel(): PowerLevel = when {
        Elevated.running -> PowerLevel.ELEVATED
        InputService.running -> PowerLevel.ASSIST
        else -> PowerLevel.BASIC
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

    /** Locks a PC, or puts it to sleep; says how it went. */
    fun pcPower(id: String, sleep: Boolean) {
        val node = node ?: return
        val name = _state.value.nameOf(id).orEmpty()
        scope.launch {
            val message = try {
                node.pcPower(id, sleep)
                context.getString(if (sleep) R.string.pc_sleeping else R.string.pc_locked, name)
            } catch (e: NectarlinkException) {
                if (e is NectarlinkException.Denied) context.getString(R.string.pc_actions_off, name) else describe(e)
            }
            _messages.tryEmit(message)
        }
    }

    /** Opens a link on a PC; returns what to tell the user. */
    suspend fun openLinkOnPc(id: String, url: String): String {
        startJob?.join()
        val node = node ?: return context.getString(R.string.clip_no_pc)
        val name = _state.value.nameOf(id).orEmpty()
        return try {
            node.openLink(id, url)
            context.getString(R.string.link_opened_on, name)
        } catch (e: NectarlinkException) {
            describe(e)
        }
    }

    // ---- Remote input (phone -> PC) ----

    /** Checks whether a PC accepts mouse, keyboard and presentation input from this phone. */
    suspend fun remoteCheck(id: String): RemoteAccess {
        startJob?.join()
        val node = node ?: return RemoteAccess.Offline
        return try {
            node.remoteCheck(id)
            RemoteAccess.Allowed
        } catch (e: NectarlinkException) {
            when (e) {
                is NectarlinkException.Denied -> RemoteAccess.Denied
                is NectarlinkException.Unsupported -> RemoteAccess.Unsupported
                else -> RemoteAccess.Offline
            }
        }
    }

    /** Moves a PC's cursor by `(dx, dy)` logical pixels over QUIC datagrams. */
    fun remoteMove(id: String, dx: Float, dy: Float) {
        remoteOps.trySend { runCatching { it.remoteMove(id, dx, dy) } }
    }

    /** Scrolls on a PC (`dy` > 0 scrolls down, `dx` > 0 scrolls right, in wheel notches). */
    fun remoteScroll(id: String, dx: Float, dy: Float, fast: Boolean = true) {
        remoteOps.trySend { runCatching { it.remoteScroll(id, dx, dy, fast) } }
    }

    /** Presses, releases or clicks a mouse button (`"left"`, `"right"` or `"middle"`). */
    fun remoteButton(id: String, button: String, action: String, onStatus: ((RemoteAccess) -> Unit)? = null) =
        remoteCommand(onStatus) { it.remoteButton(id, button, action) }

    /** Types Unicode text on a PC, splitting long strings into 256-byte chunks. */
    fun remoteText(id: String, text: String, onStatus: ((RemoteAccess) -> Unit)? = null) =
        remoteCommand(onStatus) { node ->
            for (chunk in app.nectarlink.android.ui.remote.UtteranceJoiner.chunkUtf8(text)) {
                node.remoteText(id, chunk)
            }
        }

    /** Presses a named key or shortcut with optional modifiers on a PC. */
    fun remoteKey(id: String, key: String, mods: List<String> = emptyList(), onStatus: ((RemoteAccess) -> Unit)? = null) =
        remoteCommand(onStatus) { it.remoteKey(id, key, mods) }

    /** Sends a presentation slide command (`"next"`, `"previous"`, `"start"`, `"stop"`, `"black"`). */
    fun remoteSlide(id: String, action: String, onStatus: ((RemoteAccess) -> Unit)? = null) =
        remoteCommand(onStatus) { it.remoteSlide(id, action) }

    /** Moves or hides the laser pointer on a PC (`x`, `y` in `0..1`). */
    fun remoteLaser(id: String, on: Boolean, x: Float = 0.5f, y: Float = 0.5f) {
        remoteOps.trySend { runCatching { it.remoteLaser(id, on, x, y) } }
    }

    private fun remoteCommand(onStatus: ((RemoteAccess) -> Unit)?, block: suspend (NectarlinkNode) -> Unit) {
        remoteOps.trySend { node ->
            try {
                block(node)
                onStatus?.invoke(RemoteAccess.Allowed)
            } catch (e: NectarlinkException) {
                val status = when (e) {
                    is NectarlinkException.Denied -> RemoteAccess.Denied
                    is NectarlinkException.Unsupported -> RemoteAccess.Unsupported
                    else -> RemoteAccess.Offline
                }
                onStatus?.invoke(status)
            }
        }
    }

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

    enum class RemoteAccess { Allowed, Denied, Offline, Unsupported }

    private companion object {
        const val TAG = "Nectarlink"
        /** Offered while notification access is granted (docs/protocol/capabilities.md). */
        val NOTIFICATION_CAPABILITIES = listOf("notify.mirror", "notify.reply")
        /** Sharing this phone's players, which also needs notification access. */
        val MEDIA_CAPABILITIES = listOf("media.control")
        /** Accepting the PC's clipboard, and sending this one when asked. */
        val CLIPBOARD_CAPABILITIES = listOf("clip.write", "clip.share", "mirror.capture")
        /** With access to the phone's photos. */
        val PHOTO_CAPABILITIES = listOf("photos.read")
    }
}
