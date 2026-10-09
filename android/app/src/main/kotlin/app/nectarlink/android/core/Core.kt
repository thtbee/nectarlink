// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.content.pm.PackageManager
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
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.webcam.WebcamRequests
import app.nectarlink.android.webcam.WebcamService
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
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import app.nectarlink.core.TimelinePage
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferDirection
import app.nectarlink.core.TransferStatus
import app.nectarlink.core.initLogging
import app.nectarlink.android.files.transferTitle
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
        onTask = { pc, task ->
            app.nectarlink.android.notifications.TaskNotifications.show(
                this.context,
                pc,
                _state.value.nameOf(pc).orEmpty(),
                task,
            )
        },
        calls = { calls },
        contacts = { contacts },
        sms = { sms },
        toggles = { toggles },
        storage = { storage },
        onMirror = { pc, request ->
            MirrorRequests.show(this.context, request, _state.value.nameOf(pc).orEmpty())
        },
        onWebcam = { pc, request ->
            val session = WebcamService.session.value
            if (session.active && session.pcId == pc && WebcamService.hasPermission(this.context)) {
                WebcamService.start(this.context, pc, request.height, request.fps, request.camera)
                true
            } else {
                _state.update { it.copy(webcamRequest = request) }
                WebcamRequests.show(this.context, request, _state.value.nameOf(pc).orEmpty())
                true
            }
        },
        appWindows = AppWindows(this.context, scope, open = { pc -> mirrorOpen(pc) }, nameOf = { pc -> _state.value.nameOf(pc).orEmpty() }),
        nameOf = { pc -> _state.value.nameOf(pc).orEmpty() },
        onKeyboardFromPc = { pc, on -> setKeyboardFromPc(pc, on) },
    )
    private val storage = PhoneStorage(this.context) { path ->
        notificationOps.trySend { it.storageChanged(path) }
    }
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
    private val _clipSuggestions = MutableSharedFlow<ClipSuggestionNotice>(extraBufferCapacity = 4)
    private var node: NectarlinkNode? = null
    private var startJob: Job? = null
    private var networkChange: Job? = null
    private val wakeTimers = java.util.concurrent.ConcurrentHashMap<String, Job>()

    private val battery = BatteryMonitor(this.context) { battery ->
        node?.let { scope.launch { it.updateBattery(battery) } }
    }
    private val accent = AccentMonitor(this.context) { seed ->
        node?.let { scope.launch { runCatching { it.updateAccent(seed) } } }
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

    // Holds a multicast lock while LocalSend LAN interop (UDP 224.0.0.167:53317) is enabled.
    private val localsendMulticast: WifiManager.MulticastLock? =
        this.context.getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-localsend")
            ?.apply { setReferenceCounted(false) }

    val state: StateFlow<CoreState> = _state.asStateFlow()

    /** Short messages for the user (a command failed). */
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** Context action suggestions for newly received clipboard text. */
    val clipSuggestions: SharedFlow<ClipSuggestionNotice> = _clipSuggestions.asSharedFlow()

    data class ClipSuggestionNotice(
        val message: String,
        val actionLabel: String,
        val suggestion: app.nectarlink.core.ClipSuggestion,
    )

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
            val storageAllowed = devices.mapNotNull { dev ->
                val toggles = runCatching { started.deviceToggles(dev.id) }.getOrDefault(emptyList())
                if (toggles.any { it.name == PhoneStorage.TOGGLE_NAME && it.enabled }) dev.id else null
            }.toSet()
            val clipHistEnabled = runCatching { started.clipboardHistoryEnabled() }.getOrDefault(true)
            val clipHist = if (clipHistEnabled) {
                runCatching { started.clipboardHistory(null) }.getOrDefault(emptyList())
            } else {
                emptyList()
            }
            val retention = runCatching { started.timelineRetention() }.getOrNull()
            val tlPage = runCatching { started.timelinePage(null, null, null, 0u, 100u) }.getOrNull()
            val lsEnabled = runCatching { started.localsendEnabled() }.getOrDefault(false)
            val lsPeers = if (lsEnabled) {
                runCatching { started.localsendPeers() }.getOrDefault(emptyList())
            } else {
                emptyList()
            }
            localsendMulticast?.runCatching { if (lsEnabled) acquire() else release() }
            pruneOldTempCaches()
            _state.update {
                it.withDevices(devices, storageAllowed).copy(
                    status = CoreStatus.Ready(started.deviceId()),
                    clipboardHistoryEnabled = clipHistEnabled,
                    clipboardHistory = clipHist,
                    timeline = tlPage?.items ?: emptyList(),
                    timelineTotal = tlPage?.total ?: 0u,
                    timelineHasMore = tlPage?.hasMore ?: false,
                    timelineRetentionDays = retention?.maxDays ?: 90u,
                    localsendEnabled = lsEnabled,
                    localsendPeers = lsPeers,
                )
            }
            refreshDataRetention()
            app.nectarlink.android.clipboard.DirectShareTargets.sync(context, _state.value.devices)
            app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, _state.value)
            scope.launch {
                _state.collect { s ->
                    app.nectarlink.android.clipboard.DirectShareTargets.sync(context, s.devices)
                    app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, s)
                }
            }
            if (devices.isNotEmpty()) ConnectionService.start(context)
            scope.launch(Dispatchers.Main) {
                battery.start()
                accent.start()
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
                    storageAllFilesAccess = PhoneStorage.hasAllFilesAccess(context),
                    storageSafFolders = PhoneStorage.safFolders(context).map { f -> f.name },
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
        val storageAllFilesAccess = PhoneStorage.hasAllFilesAccess(context)
        val storageSafFolders = PhoneStorage.safFolders(context).map { it.name }
        if (!storageAllFilesAccess) storage.stopWatching()
        val localNetwork = LocalNetwork.granted(context)
        if (localNetwork && !_state.value.localNetwork) {
            // Just allowed: reach the PCs now rather than at the next retry.
            scope.launch { node?.networkChanged() }
        }
        val prevPhotos = _state.value.photoAccess
        val currentNode = node
        val storageAllowed = if (currentNode != null) {
            _state.value.devices.mapNotNull { dev ->
                val devToggles = runCatching { currentNode.deviceToggles(dev.id) }.getOrDefault(emptyList())
                if (devToggles.any { it.name == PhoneStorage.TOGGLE_NAME && it.enabled }) dev.id else null
            }.toSet()
        } else {
            emptySet()
        }
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
                storageAllFilesAccess = storageAllFilesAccess,
                storageSafFolders = storageSafFolders,
                localNetwork = localNetwork,
                devices = if (currentNode != null) {
                    it.devices.map { d -> d.copy(storageEnabled = d.id in storageAllowed) }
                } else {
                    it.devices
                },
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
            scope.launch(Dispatchers.IO) { refreshDataRetention() }
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
                    wakeTimers.remove(event.id)?.cancel()
                    retryWaitingRecordings(event.id)
                }
            }
            is Event.DeviceAdded -> ConnectionService.start(context)
            is Event.DeviceRemoved -> {
                wakeTimers.remove(event.id)?.cancel()
                pcMedia.update(event.id, "", emptyList())
                if (_state.value.devices.isEmpty()) ConnectionService.stop(context)
            }
            is Event.Paired -> ConnectionService.start(context)
            is Event.ClipboardReceived -> scope.launch(Dispatchers.IO) { onClipboardReceived(event.id) }
            is Event.ClipboardHistoryChanged -> scope.launch(Dispatchers.IO) {
                refreshClipboardHistory()
                refreshTimeline()
            }
            is Event.TimelineChanged -> scope.launch(Dispatchers.IO) { refreshTimeline() }
            is Event.LocalSendChanged -> scope.launch(Dispatchers.IO) { syncLocalSendState() }
            else -> {}
        }
    }

    private fun onClipboardReceived(pcId: String) {
        if (!app.nectarlink.android.ui.Preferences.isSuggestClipboardActionsEnabled(context)) return
        val suggestion = runCatching { node?.lastClipSuggestion() }.getOrNull() ?: return
        val pcName = _state.value.nameOf(pcId).orEmpty()
        val from = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val message = context.getString(R.string.clip_received_from, from)
        val actionLabel = LinkNotifications.actionLabel(context, suggestion.kind)
        _clipSuggestions.tryEmit(ClipSuggestionNotice(message, actionLabel, suggestion))
        LinkNotifications.showClipSuggestion(context, pcName, suggestion)
    }

    /** Executes the action for a smart clipboard suggestion. */
    fun runClipSuggestion(suggestion: app.nectarlink.core.ClipSuggestion) {
        LinkNotifications.runSuggestion(context, suggestion)
    }

    // ---- Files & Recordings ----

    private fun onTransfer(transfer: Transfer) {
        val pc = _state.value.nameOf(transfer.deviceId).orEmpty()
        val pcOnline = _state.value.device(transfer.deviceId)?.online == true
        recordings.onTransferEvent(transfer, pcOnline)
        TransferNotifications.update(context, transfer, pc)
        val done = transfer.status as? TransferStatus.Done ?: return
        if (transfer.direction != TransferDirection.INCOMING) {
            scope.launch(Dispatchers.IO) { refreshDataRetention() }
            return
        }
        // Out of the app's cache, into Downloads, then tell the user.
        scope.launch(Dispatchers.IO) {
            val published = done.saved.flatMap { ReceivedFiles.publish(context, java.io.File(it)) }
            if (published.isNotEmpty()) {
                val title = transferTitle(context.resources, transfer)
                val detail = if (transfer.files > 1u) {
                    context.resources.getQuantityString(R.plurals.transfer_files, transfer.files.toInt(), transfer.files.toInt())
                } else {
                    ""
                }
                node?.updateTimelineByRef(
                    transfer.id,
                    TimelineKind.FILE,
                    title,
                    detail,
                    published.first().uri.toString(),
                    transfer.total,
                )
                refreshTimeline()
            } else {
                refreshDataRetention()
            }
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

    /** Opens this phone's camera stream to a PC (`webcam`), or emits a message on failure. */
    suspend fun webcamOpenOrMessage(pcId: String): MirrorStream? {
        startJob?.join()
        val node = node ?: return null
        return try {
            node.webcamOpen(pcId)
        } catch (e: NectarlinkException) {
            val name = _state.value.nameOf(pcId).orEmpty()
            val msg = when (e) {
                is NectarlinkException.Denied -> context.getString(R.string.webcam_denied_text, name)
                else -> describe(e)
            }
            _messages.tryEmit(msg)
            null
        }
    }

    fun clearWebcamRequest(pcId: String? = null) {
        if (pcId != null) WebcamRequests.dismiss(context, pcId)
        _state.update { s ->
            if (pcId == null || s.webcamRequest?.pcId == pcId) s.copy(webcamRequest = null) else s
        }
    }

    /** Sends a captured photo or scanned document back to the requesting PC (`camera.result`). */
    fun sendCameraCaptureResult(
        pcId: String,
        requestId: String,
        mode: String,
        fileName: String,
        mime: String,
        width: Int,
        height: Int,
        data: ByteArray,
    ) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            try {
                currentNode.sendCameraCaptureResult(
                    pcId = pcId,
                    requestId = requestId,
                    mode = mode,
                    fileName = fileName,
                    mime = mime,
                    width = width.coerceAtLeast(1).toUInt(),
                    height = height.coerceAtLeast(1).toUInt(),
                    data = data,
                )
            } catch (e: NectarlinkException) {
                _messages.tryEmit(describe(e))
            }
        }
    }

    /** Notifies the requesting PC that a Continuity Camera capture was cancelled (`camera.cancel`). */
    fun cancelCameraCapture(pcId: String, requestId: String, reason: String? = null) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.cancelCameraCapture(pcId, requestId, reason) }
        }
    }

    /**
     * Sends picked or shared files to a PC; problems arrive as messages.
     * The files are opened right away: Android's permission to read a
     * shared item ends with the activity that received it.
     */
    fun sendFiles(pcId: String, uris: List<Uri>, handoff: Boolean = false) =
        send(pcId, OutgoingFiles.open(context, uris), handoff)

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

    private fun send(pcId: String, files: List<FileToSend>, handoff: Boolean = false) {
        if (files.isEmpty()) {
            _messages.tryEmit(context.getString(R.string.transfer_nothing))
            return
        }
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val node = node ?: return@launch
            try {
                val singleFileName = if (files.size == 1) {
                    when (val f = files[0]) {
                        is FileToSend.Fd -> f.name.takeIf { f.folder == null }
                        is FileToSend.Path -> f.name.takeIf { f.folder == null }
                    }
                } else {
                    null
                }
                if (handoff && singleFileName != null && app.nectarlink.core.isSafeHandoffDocument(singleFileName)) {
                    node.sendHandoffFiles(pcId, files)
                } else {
                    node.sendFiles(pcId, files)
                }
            } catch (e: NectarlinkException) {
                val name = _state.value.nameOf(pcId).orEmpty()
                _messages.tryEmit(
                    if (e is NectarlinkException.Denied) context.getString(R.string.transfer_off_for, name) else describe(e),
                )
            }
        }
    }

    fun acceptTransfer(id: String) {
        node?.acceptTransfer(id)
    }

    fun cancelTransfer(id: String) {
        node?.cancelTransfer(id)
    }

    // ---- LocalSend interop ----

    fun setLocalSendEnabled(enabled: Boolean) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            try {
                currentNode.setLocalsendEnabled(enabled)
            } catch (e: NectarlinkException) {
                _messages.tryEmit(describe(e))
            }
            localsendMulticast?.runCatching { if (enabled) acquire() else release() }
            syncLocalSendState()
        }
    }

    fun refreshLocalSend() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            runCatching { currentNode.refreshLocalsend() }
            syncLocalSendState()
        }
    }

    private fun syncLocalSendState() {
        val currentNode = node ?: return
        val enabled = runCatching { currentNode.localsendEnabled() }.getOrDefault(false)
        val peers = if (enabled) {
            runCatching { currentNode.localsendPeers() }.getOrDefault(emptyList())
        } else {
            emptyList()
        }
        localsendMulticast?.runCatching { if (enabled) acquire() else release() }
        _state.update { it.copy(localsendEnabled = enabled, localsendPeers = peers) }
    }

    // ---- Clipboard ----

    /**
     * Sends text to every connected PC (or to `pcId` when chosen via Direct Share);
     * returns what to tell the user. Waits for the core when the app was just launched.
     */
    suspend fun sendClipboard(text: String, pcId: String? = null): String =
        sendToPcs(image = false, pcId = pcId) { node, pc -> node.sendClipboard(pc, text) }

    /** Sends a copied image to the connected PCs (or to `pcId`); returns what to tell the user. */
    suspend fun sendClipboardImage(uri: Uri, pcId: String? = null): String {
        startJob?.join()
        val targets = if (pcId != null) {
            _state.value.devices.filter { it.id == pcId && it.online }
        } else {
            _state.value.devices.filter { it.online }
        }
        if (targets.isEmpty()) return context.getString(R.string.clip_no_pc)
        val image = when (val read = withContext(Dispatchers.IO) { PhoneClipboard.readImage(context, uri) }) {
            is PhoneClipboard.ImageResult.Ready -> read.image
            PhoneClipboard.ImageResult.TooLarge -> return context.getString(R.string.clip_image_too_large)
            PhoneClipboard.ImageResult.Unreadable -> return context.getString(R.string.clip_image_unreadable)
        }
        return sendToPcs(image = true, pcId = pcId) { node, pc -> node.sendClipboardImage(pc, image.mime, image.bytes) }
    }

    private suspend fun sendToPcs(
        image: Boolean,
        pcId: String? = null,
        send: suspend (NectarlinkNode, String) -> Unit,
    ): String {
        startJob?.join()
        val node = node ?: return context.getString(R.string.clip_no_pc)
        val pcs = if (pcId != null) {
            _state.value.devices.filter { it.id == pcId && it.online }
        } else {
            _state.value.devices.filter { it.online }
        }
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

    private fun refreshClipboardHistory() {
        val currentNode = node ?: return
        val enabled = runCatching { currentNode.clipboardHistoryEnabled() }.getOrDefault(true)
        val items = if (enabled) {
            runCatching { currentNode.clipboardHistory(null) }.getOrDefault(emptyList())
        } else {
            emptyList()
        }
        _state.update { it.copy(clipboardHistoryEnabled = enabled, clipboardHistory = items) }
        refreshDataRetention()
    }

    fun setClipboardHistoryEnabled(enabled: Boolean) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            runCatching { currentNode.setClipboardHistoryEnabled(enabled) }
            refreshClipboardHistory()
        }
    }

    fun copyClipboardHistory(id: String) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            try {
                currentNode.copyClipboardHistory(id)
                // Android 13 and later confirm copies themselves.
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                    _messages.tryEmit(context.getString(R.string.clipboard_history_copied))
                }
            } catch (e: NectarlinkException) {
                _messages.tryEmit(describe(e))
            }
        }
    }

    fun pinClipboardHistory(id: String, pinned: Boolean) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            node?.pinClipboardHistory(id, pinned)
            refreshClipboardHistory()
        }
    }

    fun deleteClipboardHistory(id: String) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            node?.deleteClipboardHistory(id)
            refreshClipboardHistory()
        }
    }

    fun clearClipboardHistory() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.clearClipboardHistory() }
            refreshClipboardHistory()
        }
    }

    suspend fun clipboardHistoryImage(id: String): ByteArray? = withContext(Dispatchers.IO) {
        startJob?.join()
        node?.clipboardHistoryImage(id)
    }

    // ---- Timeline ----

    private fun refreshTimeline() {
        val currentNode = node ?: return
        val retention = runCatching { currentNode.timelineRetention() }.getOrNull()
        val page = runCatching { currentNode.timelinePage(null, null, null, 0u, 100u) }.getOrNull()
        _state.update {
            it.copy(
                timeline = page?.items ?: it.timeline,
                timelineTotal = page?.total ?: it.timelineTotal,
                timelineHasMore = page?.hasMore ?: it.timelineHasMore,
                timelineRetentionDays = retention?.maxDays ?: it.timelineRetentionDays,
            )
        }
        refreshDataRetention()
    }

    suspend fun queryTimelinePage(
        kind: TimelineKind?,
        deviceId: String?,
        search: String?,
        offset: UInt,
        limit: UInt = 100u,
    ): TimelinePage? = withContext(Dispatchers.IO) {
        startJob?.join()
        runCatching { node?.timelinePage(kind, deviceId, search, offset, limit) }.getOrNull()
    }

    fun setTimelineRetentionDays(days: UInt) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.setTimelineRetention(days, 5_000u) }
            refreshTimeline()
        }
    }

    fun deleteTimelineEntry(id: Long) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.deleteTimelineEntry(id) }
            refreshTimeline()
        }
    }

    fun clearTimeline() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.clearTimeline() }
            refreshTimeline()
        }
    }

    // ---- Data retention & Clear everything ----

    private fun pruneOldTempCaches() {
        val cutoff = System.currentTimeMillis() - 24L * 60 * 60 * 1000
        listOf("continuity_camera", "clipboard").forEach { dirName ->
            java.io.File(context.cacheDir, dirName).listFiles()?.forEach { file ->
                if (file.isFile && file.lastModified() in 1 until cutoff) {
                    runCatching { file.delete() }
                }
            }
        }
    }

    private fun refreshDataRetention() {
        val counts = runCatching { node?.dataRetentionCounts() }.getOrNull()
        val photoFiles = listOf("continuity_camera", "clipboard").sumOf { dirName ->
            java.io.File(context.cacheDir, dirName).listFiles()?.count { it.isFile } ?: 0
        }
        val mmsFiles = java.io.File(context.cacheDir, "mms")
            .listFiles()
            ?.count { it.isFile && it.name.endsWith(".mms", ignoreCase = true) } ?: 0
        _state.update {
            it.copy(
                receivedFileRecords = counts?.receivedFileRecords?.toInt() ?: it.receivedFileRecords,
                cachedPhotoFiles = photoFiles,
                cachedMmsFiles = mmsFiles,
            )
        }
    }

    fun clearMessageCache() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            java.io.File(context.cacheDir, "mms").listFiles()?.forEach { file ->
                if (file.isFile && file.name.endsWith(".mms", ignoreCase = true)) {
                    runCatching { file.delete() }
                }
            }
            runCatching { node?.clearMessageCache() }
            refreshDataRetention()
        }
    }

    fun clearPhotoThumbnailsCache() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            listOf("continuity_camera", "clipboard").forEach { dirName ->
                java.io.File(context.cacheDir, dirName).listFiles()?.forEach { file ->
                    runCatching { file.deleteRecursively() }
                }
            }
            refreshDataRetention()
        }
    }

    fun clearReceivedFileHistory() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            java.io.File(context.cacheDir, "received").listFiles()?.forEach { file ->
                runCatching { file.deleteRecursively() }
            }
            _state.update { s -> s.copy(transfers = s.transfers.filterNot { it.isFinished() }) }
            runCatching { node?.clearReceivedFileHistory() }
            refreshTimeline()
            refreshDataRetention()
        }
    }

    fun clearEverything() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.clearAllLocalData() }
            listOf("mms", "continuity_camera", "clipboard", "received").forEach { dirName ->
                java.io.File(context.cacheDir, dirName).listFiles()?.forEach { file ->
                    runCatching { file.deleteRecursively() }
                }
            }
            _state.update { s -> s.copy(transfers = s.transfers.filterNot { it.isFinished() }) }
            refreshClipboardHistory()
            refreshTimeline()
            refreshDataRetention()
            _messages.tryEmit(context.getString(R.string.data_cleared_everything_toast))
        }
    }

    fun resendTimelineEntry(entry: TimelineEntry) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            val pcName = _state.value.nameOf(entry.deviceId).orEmpty().ifEmpty { context.getString(R.string.your_pc) }
            when (entry.kind) {
                TimelineKind.CLIP -> {
                    val clipId = entry.refId ?: return@launch
                    try {
                        currentNode.resendClipboardHistory(clipId, entry.deviceId)
                        _messages.tryEmit(context.getString(R.string.clip_sent_to, pcName))
                    } catch (e: NectarlinkException) {
                        _messages.tryEmit(describe(e))
                    }
                }
                TimelineKind.LINK -> {
                    val url = entry.target.ifEmpty { entry.title }
                    _messages.tryEmit(openLinkOnPc(entry.deviceId, url))
                }
                TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> {
                    val target = entry.target
                    when {
                        target.startsWith("content://") -> sendFiles(entry.deviceId, listOf(Uri.parse(target)))
                        target.isNotEmpty() && java.io.File(target).exists() -> {
                            val f = java.io.File(target)
                            send(
                                entry.deviceId,
                                listOf(
                                    FileToSend.Path(
                                        path = f.absolutePath,
                                        name = f.name,
                                        folder = null,
                                    ),
                                ),
                            )
                        }
                    }
                }
                TimelineKind.RECORDING -> {
                    val target = entry.target
                    if (target.isNotEmpty() && java.io.File(target).exists()) {
                        val f = java.io.File(target)
                        try {
                            currentNode.sendRecording(
                                entry.deviceId,
                                FileToSend.Path(path = f.absolutePath, name = f.name, folder = null),
                                emptyList(),
                            )
                        } catch (e: NectarlinkException) {
                            _messages.tryEmit(describe(e))
                        }
                    }
                }
                TimelineKind.SESSION -> Unit
            }
        }
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
            PhoneStorage.capabilities(context) +
            (if (WebcamService.hasPermission(context)) listOf("camera.stream") else emptyList()) +
            (if (context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) listOf("camera.capture") else emptyList()) +
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
        scope.launch {
            startJob?.join()
            val node = node ?: return@launch
            val name = _state.value.nameOf(id).orEmpty()
            val message = try {
                node.pcPower(id, sleep)
                context.getString(if (sleep) R.string.pc_sleeping else R.string.pc_locked, name)
            } catch (e: NectarlinkException) {
                if (e is NectarlinkException.Denied) context.getString(R.string.pc_actions_off, name) else describe(e)
            }
            _messages.tryEmit(message)
        }
    }

    /**
     * Sends Wake-on-LAN magic packets to a sleeping or shut-down PC and waits
     * up to ~60 s for it to connect before showing troubleshooting tips.
     */
    fun wake(id: String) {
        wakeTimers.remove(id)?.cancel()
        _state.update { it.withWakeState(id, WakeState.Waking) }
        val timer = scope.launch(Dispatchers.IO) {
            startJob?.join()
            val node = node ?: return@launch
            repeat(15) {
                delay(4_000)
                val dev = _state.value.device(id)
                if (dev == null || dev.online || dev.wakeState != WakeState.Waking) return@launch
                runCatching { node.refresh() }
            }
            _state.update { s ->
                val dev = s.device(id)
                if (dev != null && !dev.online && dev.wakeState == WakeState.Waking) {
                    s.withWakeState(id, WakeState.TimedOut)
                } else {
                    s
                }
            }
        }
        wakeTimers[id] = timer
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val node = node ?: return@launch
            try {
                node.wake(id)
                node.refresh()
            } catch (e: NectarlinkException) {
                wakeTimers.remove(id)?.cancel()
                _state.update { it.withWakeState(id, WakeState.Idle) }
                val name = _state.value.nameOf(id).orEmpty()
                val msg = when (e) {
                    is NectarlinkException.Unsupported -> context.getString(R.string.wake_not_ready, name)
                    else -> describe(e)
                }
                _messages.tryEmit(msg)
            }
        }
    }

    /** Opens a link, video with timestamp, or map location on a PC; returns what to tell the user. */
    suspend fun openLinkOnPc(id: String, url: String): String {
        startJob?.join()
        val node = node ?: return context.getString(R.string.clip_no_pc)
        val name = _state.value.nameOf(id).orEmpty()
        val handoff = app.nectarlink.core.extractHandoffLink(url)
        val finalUrl = if (
            handoff != null &&
            handoff.kind == app.nectarlink.core.HandoffKind.VIDEO_LINK &&
            handoff.timestampSecs == null
        ) {
            val activeSecs = phoneMedia.activePositionSeconds()
            if (activeSecs != null) app.nectarlink.core.withVideoTimestamp(handoff.url, activeSecs) else handoff.url
        } else {
            handoff?.url ?: url
        }
        return try {
            node.openLink(id, finalUrl)
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

    // ---- Deck (phone -> PC) ----

    enum class DeckPressResult { Ok, Denied, Offline, NotFound, Failed }

    /** Presses a Deck tile on a paired PC (`deck.press`). */
    suspend fun deckPress(id: String, tile: String): DeckPressResult {
        startJob?.join()
        val node = node ?: return DeckPressResult.Offline
        return try {
            node.deckPress(id, tile)
            DeckPressResult.Ok
        } catch (e: NectarlinkException) {
            when (e) {
                is NectarlinkException.Denied -> DeckPressResult.Denied
                is NectarlinkException.Offline -> DeckPressResult.Offline
                is NectarlinkException.NotFound -> DeckPressResult.NotFound
                else -> {
                    _messages.tryEmit(describe(e))
                    DeckPressResult.Failed
                }
            }
        }
    }

    /** Sets the PC's master volume (`0..=100`) and/or mute state (`pc.audio.set`). */
    fun setPcAudio(id: String, volume: Int? = null, muted: Boolean? = null) {
        val clampedVol = volume?.coerceIn(0, 100)?.toUByte()
        // Optimistic local state update so the slider and mute button don't snap back while the RPC runs.
        _state.update { s ->
            val current = s.device(id)?.deckState
            if (current != null) {
                s.reduce(
                    Event.DeckState(
                        id = id,
                        state = current.copy(
                            volume = clampedVol ?: current.volume,
                            muted = muted ?: current.muted,
                        ),
                    ),
                )
            } else {
                s
            }
        }
        remoteOps.trySend { node ->
            try {
                node.setPcAudio(id, clampedVol, muted)
            } catch (e: NectarlinkException) {
                if (e is NectarlinkException.Denied) {
                    val name = _state.value.nameOf(id).orEmpty()
                    _messages.tryEmit(context.getString(R.string.remote_denied_title, name))
                } else if (e !is NectarlinkException.Offline) {
                    _messages.tryEmit(describe(e))
                }
            }
        }
    }

    // ---- Phone storage (File Explorer on PC) ----

    /** Turns phone storage access in File Explorer on or off for `pcId` (off by default). */
    fun setStorageEnabled(pcId: String, enabled: Boolean) {
        val currentNode = node ?: return
        runCatching { currentNode.setDeviceToggle(pcId, PhoneStorage.TOGGLE_NAME, enabled) }
        _state.update { it.withStorageEnabled(pcId, enabled) }
        notificationOps.trySend {
            it.updatePower(powerLevel(), capabilities(_state.value.notificationAccess))
        }
    }

    fun dismissStorageRequest() {
        _state.update { it.copy(storageRequestedFrom = null) }
    }

    fun addSafFolder(treeUri: Uri) {
        PhoneStorage.addSafFolder(context, treeUri)
        refreshNotificationAccess()
    }

    fun removeSafFolder(name: String) {
        PhoneStorage.removeSafFolder(context, name)
        refreshNotificationAccess()
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

    /** Records whether `pcId` is currently typing on this phone without mirroring. */
    fun setKeyboardFromPc(pcId: String, on: Boolean) {
        _state.update {
            it.copy(keyboardFromPc = if (on) it.keyboardFromPc + pcId else it.keyboardFromPc - pcId)
        }
    }

    /** Stops remote keyboard typing from `pcId` and notifies the PC. */
    fun stopKeyboardFromPc(pcId: String) {
        _state.update { it.copy(keyboardFromPc = it.keyboardFromPc - pcId) }
        val currentNode = node ?: return
        scope.launch(Dispatchers.IO) {
            runCatching { currentNode.mirrorStop(pcId, 0u) }
        }
    }

    /** Runs a command on the node, turning errors into a message. */
    private fun command(block: suspend (NectarlinkNode) -> Unit) {
        scope.launch {
            startJob?.join()
            val node = node ?: return@launch
            try {
                block(node)
            } catch (e: NectarlinkException) {
                _messages.tryEmit(describe(e))
            }
        }
    }

    private fun describe(e: NectarlinkException): String = when (e) {
        is NectarlinkException.Offline -> context.getString(R.string.error_not_connected)
        is NectarlinkException.NotPaired -> context.getString(R.string.error_not_paired)
        is NectarlinkException.Timeout -> context.getString(R.string.error_timed_out)
        is NectarlinkException.Unsupported -> context.getString(R.string.error_unsupported)
        else -> context.getString(R.string.error_generic)
    }

    enum class RemoteAccess { Allowed, Denied, Offline, Unsupported }

    private companion object {
        const val TAG = "Nectarlink"
        /** Offered while notification access is granted (docs/protocol/capabilities.md). */
        val NOTIFICATION_CAPABILITIES = listOf("notify.mirror", "notify.reply", "notify.live")
        /** Sharing this phone's players, which also needs notification access. */
        val MEDIA_CAPABILITIES = listOf("media.control")
        /** Accepting the PC's clipboard, and sending this one when asked. */
        val CLIPBOARD_CAPABILITIES = listOf("clip.write", "clip.share", "mirror.capture")
        /** With access to the phone's photos. */
        val PHOTO_CAPABILITIES = listOf("photos.read")
    }
}
