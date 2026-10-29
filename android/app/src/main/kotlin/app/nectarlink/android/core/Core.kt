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
import app.nectarlink.android.audio.PcSpeakerPlayer
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.links.LinkNotifications
import app.nectarlink.android.media.PcMedia
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.files.OutgoingFiles
import app.nectarlink.android.files.ReceivedFiles
import app.nectarlink.android.files.TransferNotifications
import app.nectarlink.android.R
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.notifications.PcNotificationPoster
import app.nectarlink.android.calls.CallCompanion
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.companion.PcCompanion
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
import app.nectarlink.android.mic.MicService
import app.nectarlink.android.mirror.MirrorService
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.webcam.WebcamRequests
import app.nectarlink.android.webcam.WebcamService
import app.nectarlink.core.FfiQueueStatus
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
import app.nectarlink.core.RemoteStorageEntry
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
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The app's single connection to `nectarlink-core`. Owns the node, folds
 * its events into [state], feeds it the phone's battery and network
 * changes, and runs commands for the UI.
 */
@androidx.compose.runtime.Stable
class Core(context: Context, private val scope: CoroutineScope) : EventListener {
    private val context = context.applicationContext.also { PhoneClipboard.init(it) }
    private val ringer = Ringer(this.context)
    private val pcNotificationPoster = PcNotificationPoster(this.context)
    private val pcSpeakerPlayer = PcSpeakerPlayer()
    /** What plays on this phone, for PCs (needs notification access). */
    private val phoneMedia = PhoneMedia(this.context) { players ->
        syncActiveMediaColor()
        enqueueNotificationOp { it.mediaChanged(players) }
    }
    private val platform = PhonePlatform(
        this.context,
        ringer,
        phoneMedia,
        onLink = { pc, url -> LinkNotifications.show(this.context, _state.value.nameOf(pc).orEmpty(), url) },
        onMic = { pc ->
            // Asked on Home too, so it's seen with notifications off or while the app is open.
            _state.update { it.copy(micRequestFrom = pc) }
            app.nectarlink.android.mic.MicRequests.show(this.context, pc, _state.value.nameOf(pc).orEmpty())
            true
        },
        onMicStop = { pc -> clearMicRequest(pc) },
        onWatch = { pc, player, title, file ->
            app.nectarlink.android.media.WatchOffers.show(
                this.context,
                pc,
                _state.value.nameOf(pc).orEmpty(),
                player,
                title,
                file,
            )
        },
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
        enqueueNotificationOp { it.storageChanged(path) }
    }
    private val toggles = PhoneToggles(
        this.context,
        onChanged = { state ->
            enqueueNotificationOp { node ->
                runCatching { node.togglesChanged(state) }.onFailure { Log.i(TAG, "toggles weren't reported", it) }
            }
        },
        onCapabilitiesChanged = { refreshNotificationAccess() },
    )
    private val sms = PhoneSms(this.context) {
        enqueueNotificationOp { it.smsChanged(null) }
    }
    private val calls = PhoneCalls(
        this.context,
        onChange = { call ->
            enqueueNotificationOp { node ->
                runCatching { node.callChanged(call) }.onFailure { Log.i(TAG, "a call wasn't reported", it) }
            }
        },
        onLogChanged = {
            enqueueNotificationOp { it.callLogChanged() }
        },
    )
    private val contacts = PhoneContacts(this.context) {
        enqueueNotificationOp { it.contactsChanged() }
    }
    private val photos = RecentPhotos(
        this.context,
        onPhoto = { photo ->
            enqueueNotificationOp { node ->
                runCatching { node.photoTaken(photo) }.onFailure { Log.i(TAG, "a new photo wasn't announced", it) }
            }
        },
        onChanged = {
            enqueueNotificationOp { it.photosChanged() }
        },
    )

    /** What plays on the PCs, in Android's media controls. */
    val pcMedia = PcMedia(
        context = this.context,
        onSeedColorChanged = { syncActiveMediaColor() },
        setPcAudio = { pc, volume, muted -> setPcAudio(pc, volume, muted) },
    ) { pc, player, action, position ->
        command { it.mediaCommand(pc, player, action, position) }
    }.also { it.volumeKeys(app.nectarlink.android.ui.Preferences.isVolumeKeysControlPcEnabled(this.context)) }

    private fun syncActiveMediaColor() {
        val nextColor = phoneMedia.activeSeedColor ?: pcMedia.activeSeedColor
        if (_state.value.activeMediaColor != nextColor) {
            _state.update { if (it.activeMediaColor != nextColor) it.copy(activeMediaColor = nextColor) else it }
        }
        accent.updateMediaColor(nextColor)
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

    private val opsLock = Any()
    @Volatile private var isRunning = false

    private fun enqueueNotificationOp(op: suspend (NectarlinkNode) -> Unit) {
        synchronized(opsLock) {
            if (isRunning && node != null) {
                notificationOps.trySend(op)
            }
        }
    }

    private fun enqueueRemoteOp(op: suspend (NectarlinkNode) -> Unit) {
        synchronized(opsLock) {
            if (isRunning && node != null) {
                remoteOps.trySend(op)
            }
        }
    }

    private val _state = MutableStateFlow(CoreState())
    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    private val _queuedNotices = MutableSharedFlow<OfflineQueuedNotice>(extraBufferCapacity = 8)
    private val _clipSuggestions = MutableSharedFlow<ClipSuggestionNotice>(extraBufferCapacity = 4)
    @Volatile private var node: NectarlinkNode? = null
    @Volatile private var startJob: Job? = null
    @Volatile private var runJob: Job? = null
    @Volatile private var runScope: CoroutineScope? = null
    @Volatile private var networkChange: Job? = null
    private val lifecycleMutex = Mutex()
    private val startGuard = StartGuard()
    private val wakeTimers = java.util.concurrent.ConcurrentHashMap<String, Job>()

    @Volatile private var lastBattery: app.nectarlink.core.Battery? = null
    @Volatile private var lastSignal: SignalSnapshot = SignalSnapshot()
    @Volatile private var lastReportedBattery: app.nectarlink.core.Battery? = null

    private fun pushBatteryAndSignal() {
        val base = lastBattery ?: return
        val sig = lastSignal
        val merged = base.copy(
            cellBars = sig.cellBars,
            cellType = sig.cellType,
            wifiBars = sig.wifiBars,
        )
        if (merged != lastReportedBattery) {
            lastReportedBattery = merged
            node?.let { scope.launch { it.updateBattery(merged) } }
        }
    }

    private val battery = BatteryMonitor(this.context) { battery ->
        lastBattery = battery
        pushBatteryAndSignal()
    }
    private val signal = SignalMonitor(this.context) { snapshot ->
        val hadWifi = lastSignal.wifiBars != null
        val hasWifi = snapshot.wifiBars != null
        lastSignal = snapshot
        pushBatteryAndSignal()
        if (hadWifi != hasWifi && (hasWifi || _state.value.awayMode)) {
            node?.let { scope.launch { runCatching { it.networkChanged() } } }
        }
    }
    private val accent = AccentMonitor(this.context) { seed ->
        node?.let { scope.launch { runCatching { it.updateAccent(seed) } } }
    }
    private val network = NetworkMonitor(this.context) {
        // Network callbacks come in bursts; tell the core once they settle.
        networkChange?.cancel()
        networkChange = scope.launch {
            delay(150)
            if (lastSignal.wifiBars != null || _state.value.awayMode) {
                node?.networkChanged()
            }
            // A new Wi-Fi network turns wireless debugging off; Elevated turns it back on.
            if (!Elevated.running && lastSignal.wifiBars != null) Elevated.onNetworkChanged()
        }
    }

    // Android drops multicast (local discovery) unless a lock is held. The
    // connection service holds one while paired; this one covers pairing.
    private val pairingMulticast: WifiManager.MulticastLock? by lazy {
        this.context.getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-pairing")
            ?.apply { setReferenceCounted(false) }
    }

    // Holds a multicast lock while LocalSend LAN interop (UDP 224.0.0.167:53317) is enabled.
    private val localsendMulticast: WifiManager.MulticastLock? by lazy {
        this.context.getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-localsend")
            ?.apply { setReferenceCounted(false) }
    }

    val state: StateFlow<CoreState> = _state.asStateFlow()

    /** True while a user-facing stream or transfer is active. */
    val busy: StateFlow<Boolean> = combine(
        _state.map { s ->
            s.transfers.any { !it.isFinished() } ||
                s.sendQueue.any { it.status is FfiQueueStatus.Sending || it.status is FfiQueueStatus.Waiting }
        }.distinctUntilChanged(),
        MirrorService.session.map { it.active }.distinctUntilChanged(),
        WebcamService.session.map { it.active || it.connecting }.distinctUntilChanged(),
        MicService.streaming.map { it != null }.distinctUntilChanged(),
        // Phone as speaker: frames arriving, or the PC still has it on
        // (frames stop while the PC is silent, for example a paused song).
        combine(pcSpeakerPlayer.playing, _state.map { it.pcSpeakerRelayOn }) { playing, relayOn ->
            playing || relayOn
        }.distinctUntilChanged(),
    ) { hasTransfers: Boolean, isMirroring: Boolean, isWebcam: Boolean, isMic: Boolean, isSpeaker: Boolean ->
        hasTransfers || isMirroring || isWebcam || isMic || isSpeaker
    }.stateIn(scope, SharingStarted.Eagerly, false)

    /** Short messages for the user (a command failed). */
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** Undoable notices emitted when an outbound action is queued for an offline PC. */
    val queuedNotices: SharedFlow<OfflineQueuedNotice> = _queuedNotices.asSharedFlow()

    /** Context action suggestions for newly received clipboard text. */
    val clipSuggestions: SharedFlow<ClipSuggestionNotice> = _clipSuggestions.asSharedFlow()

    data class OfflineQueuedNotice(
        val message: String,
        val queueId: String?,
        val peerId: String?,
    )

    data class ClipSuggestionNotice(
        val message: String,
        val actionLabel: String,
        val suggestion: app.nectarlink.core.ClipSuggestion,
    )

    data class PcMirrorConfigState(
        val pcId: String,
        val codec: String,
        val width: Int,
        val height: Int,
    )

    data class PcMirrorPacket(
        val pcId: String,
        val keyframe: Boolean,
        val timeUs: Long,
        val data: ByteArray,
    )

    private val _pcMirrorConfig = MutableStateFlow<PcMirrorConfigState?>(null)
    val pcMirrorConfig: StateFlow<PcMirrorConfigState?> = _pcMirrorConfig.asStateFlow()

    @Volatile private var lastPcMirrorKeyframe: PcMirrorPacket? = null

    fun latestPcMirrorKeyframe(pcId: String): PcMirrorPacket? =
        lastPcMirrorKeyframe?.takeIf { it.pcId == pcId }

    private val _pcMirrorPackets = MutableSharedFlow<PcMirrorPacket>(
        extraBufferCapacity = 32,
        onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST,
    )
    val pcMirrorPackets: SharedFlow<PcMirrorPacket> = _pcMirrorPackets.asSharedFlow()

    /**
     * Starts the node (once). Safe to call repeatedly, from any thread: the
     * app calls it twice as it opens, and two calls at once used to start
     * two nodes with one identity, which kept taking the PC from each other.
     */
    fun start() {
        scope.launch(Dispatchers.IO) {
            lifecycleMutex.withLock {
                if (node != null || startJob?.isActive == true) return@withLock
                val token = startGuard.beginStart()
                val currentRun = Job()
                runJob = currentRun
                val currentRunScope = CoroutineScope(scope.coroutineContext + currentRun)
                runScope = currentRunScope
                val job = currentRunScope.launch(Dispatchers.IO) {
            initLogging(if (BuildConfig.DEBUG) "debug,iroh=info,swarm_discovery=error" else "info,iroh=warn,swarm_discovery=error")
            val initialAwayMode = app.nectarlink.android.ui.Preferences.isAwayModeEnabled(context)
            val options = NodeOptions(
                dataDir = context.noBackupFilesDir.resolve("core").absolutePath,
                device = deviceInfo(context),
                appVersion = BuildConfig.VERSION_NAME,
                power = PowerLevel.BASIC,
                awayMode = initialAwayMode,
                downloadsDir = context.cacheDir.resolve("received").absolutePath,
                usbPort = 53319u.toUShort(),
            )
            val started = try {
                NectarlinkNode.start(options, platform, KeystoreKeyProtector(), this@Core)
            } catch (e: NectarlinkException) {
                Log.e(TAG, "core failed to start", e)
                _state.update { it.copy(status = CoreStatus.Failed(e.message ?: e.javaClass.simpleName)) }
                return@launch
            }
            if (!currentRun.isActive || startGuard.isCancelled(token)) {
                shutdownAndCloseNode(started)
                return@launch
            }
            synchronized(opsLock) {
                node = started
                isRunning = true
            }
            val devices = runCatching { started.pairedDevices() }.getOrDefault(emptyList())
            val pcToggles = perPcToggles(started, devices.map { it.id })
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
            val initialQueue = runCatching { started.sendQueue(null).map { it.toUi() } }.getOrDefault(emptyList())
            localsendMulticast?.runCatching { if (lsEnabled) acquire() else release() }
            pruneOldTempCaches()
            _state.update {
                it.withDevices(devices, pcToggles.storageAllowed, pcToggles.micOff).copy(
                    status = CoreStatus.Ready(started.deviceId()),
                    sendQueue = initialQueue,
                    clipboardHistoryEnabled = clipHistEnabled,
                    clipboardHistory = clipHist,
                    timeline = tlPage?.items ?: emptyList(),
                    timelineTotal = tlPage?.total ?: 0u,
                    timelineHasMore = tlPage?.hasMore ?: false,
                    timelineRetentionDays = retention?.maxDays ?: 90u,
                    localsendEnabled = lsEnabled,
                    awayMode = initialAwayMode,
                    localsendPeers = lsPeers,
                )
            }
            NotificationListener.instance?.refreshActive()
            refreshDataRetention()
            app.nectarlink.android.clipboard.DirectShareTargets.sync(context, _state.value.devices)
            app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, _state.value)
            currentRunScope.launch(Dispatchers.IO) {
                // A rename in Android's settings reaches the PCs right away,
                // not at the next start (#87).
                callbackFlow {
                    val observer = object : android.database.ContentObserver(null) {
                        override fun onChange(selfChange: Boolean) {
                            trySend(Unit)
                        }
                    }
                    context.contentResolver.registerContentObserver(
                        android.provider.Settings.Global.getUriFor(android.provider.Settings.Global.DEVICE_NAME),
                        false,
                        observer,
                    )
                    awaitClose { context.contentResolver.unregisterContentObserver(observer) }
                }.collect {
                    runCatching { started.updateDeviceInfo(deviceInfo(context, _state.value.activeMediaColor)) }
                        .onFailure { Log.w(TAG, "can't send the new device name", it) }
                }
            }
            currentRunScope.launch(Dispatchers.IO) {
                _state
                    .map { s ->
                        s.devices.map { d -> listOf(d.id, d.name, d.online, d.canWake, d.wakeState) }
                    }
                    .distinctUntilChanged()
                    .collect {
                        val s = _state.value
                        app.nectarlink.android.clipboard.DirectShareTargets.sync(context, s.devices)
                        app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, s)
                    }
            }
            currentRunScope.launch(Dispatchers.Default) {
                // The volume buttons reach a PC whose volume this phone may set.
                _state
                    .map { s ->
                        s.devices
                            .filter { it.online && it.has("media.pc_audio") }
                            .mapNotNull { d -> d.deckState?.let { d.id to PcMedia.PcLevel(it.volume.toInt(), it.muted) } }
                            .toMap()
                    }
                    .distinctUntilChanged()
                    .collect { pcMedia.levels(it) }
            }
            currentRunScope.launch(Dispatchers.IO) {
                var first = true
                _state
                    .map { s ->
                        s.devices.map { d -> listOf(d.id, d.online, d.deckState?.volume, d.deckState?.muted) }
                    }
                    .distinctUntilChanged()
                    .collect {
                        if (!first) delay(120L)
                        first = false
                        app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, _state.value)
                    }
            }
            if (devices.isNotEmpty()) ConnectionService.start(context)
            val companionAssociated = PcCompanion.isAssociated(context)
            if (companionAssociated) {
                PcCompanion.startObservingPresence(context)
            }
            val initialInputEnabled = InputService.running || InputService.enabled(context)
            val initialElevated = Elevated.running
            val initialInputAccess = InputService.running || initialElevated || initialInputEnabled
            _state.update {
                it.copy(
                    companionAssociated = companionAssociated,
                    photoAccess = RecentPhotos.hasAccess(context),
                    photoPartialAccess = RecentPhotos.hasPartialAccess(context),
                    callAccess = PhoneCalls.hasAll(context),
                    contactsAccess = PhoneContacts.canRead(context),
                    smsAccess = PhoneSms.hasAll(context),
                    inputAccess = initialInputAccess,
                    elevated = initialElevated,
                    dndAccess = PhoneToggles.hasDndAccess(context),
                    writeSettingsAccess = PhoneToggles.hasWriteSettings(context),
                    storageAllFilesAccess = PhoneStorage.hasAllFilesAccess(context),
                    storageSafFolders = PhoneStorage.safFolders(context).map { f -> f.name },
                )
            }
            started.updatePower(powerLevel(), capabilities(NotificationListener.hasAccess(context)))
            // Elevated, when set up and wireless debugging is on.
            currentRunScope.launch { Elevated.start() }
            currentRunScope.launch(Dispatchers.Main) {
                // Allow first Compose frame to draw before registering BroadcastReceivers and ContentObservers on Main.
                delay(48L)
                battery.start()
                accent.start()
                network.start()
                if (NotificationListener.hasAccess(context)) phoneMedia.start()
                if (PhoneCalls.canFollow(context) || PhoneCalls.canReadLog(context)) calls.start()
                if (PhoneContacts.canRead(context)) contacts.start()
                if (PhoneSms.canRead(context)) sms.start()
                _state
                    .map { s -> s.devices.any { it.online } to s.photoAccess }
                    .distinctUntilChanged()
                    .collect { (anyOnline, hasPhotoAccess) ->
                        if (anyOnline) {
                            signal.start()
                            toggles.start()
                            if (hasPhotoAccess) photos.start() else photos.stop()
                        } else {
                            signal.stop()
                            toggles.stop()
                            photos.stop()
                        }
                    }
            }
            currentRunScope.launch {
                for (op in notificationOps) {
                    runCatching { op(started) }.onFailure { Log.w(TAG, "notification update failed", it) }
                }
            }
            currentRunScope.launch {
                for (op in remoteOps) {
                    runCatching { op(started) }.onFailure { Log.w(TAG, "remote input failed", it) }
                }
            }
        }
        startJob = job
    }
}
}

    /**
     * Shuts down the active node and all per-run collectors, monitors, and loops.
     * Sets status back to [CoreStatus.Starting] and clears node references.
     * Non-blocking and safe to call from any thread.
     */
    fun stop() {
        startGuard.stop()
        scope.launch(Dispatchers.IO) {
            lifecycleMutex.withLock {
                val activeJob = startJob
                val activeRun = runJob
                if (activeJob == null && node == null && activeRun == null) return@withLock

                activeRun?.cancelAndJoin()
                activeJob?.cancelAndJoin()

                val activeNode = node
                synchronized(opsLock) {
                    isRunning = false
                    node = null
                    // Drain any pending operations so they don't replay on the next run.
                    while (notificationOps.tryReceive().isSuccess) {}
                    while (remoteOps.tryReceive().isSuccess) {}
                }
                startJob = null
                runJob = null
                runScope = null

                activeNode?.let { shutdownAndCloseNode(it) }
                runCatching { Elevated.stop() }

                withContext(Dispatchers.Main) {
                    battery.stop()
                    accent.stop()
                    network.stop()
                    calls.stop()
                    contacts.stop()
                    sms.stop()
                    signal.stop()
                    toggles.stop()
                    photos.stop()
                    phoneMedia.stop()
                    pcSpeakerPlayer.stop()
                }

                lastReportedBattery = null
                localsendMulticast?.runCatching { release() }
                wakeTimers.values.forEach { it.cancel() }
                wakeTimers.clear()

                _state.update { it.withDevicesOffline().copy(status = CoreStatus.Starting) }
                app.nectarlink.android.widget.PcWidgetProvider.refreshAll(context, _state.value)
            }
        }
    }

    private suspend fun shutdownAndCloseNode(node: NectarlinkNode) {
        try {
            node.shutdown()
        } finally {
            runCatching { node.close() }
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
        runScope?.launch(Dispatchers.Main) { if (granted) phoneMedia.start() else phoneMedia.stop() }
        enqueueNotificationOp { node ->
            node.updatePower(powerLevel(), capabilities(granted))
            node.notificationsReset(showing)
        }
    }

    fun notificationPosted(notification: Notification) {
        enqueueNotificationOp { it.notificationPosted(notification) }
    }

    fun notificationRemoved(key: String) {
        enqueueNotificationOp { it.notificationRemoved(key) }
    }

    fun notificationsRefreshed(showing: List<Notification>) {
        enqueueNotificationOp { it.notificationsReset(showing) }
    }

    @Volatile private var lastSensitiveAccess = false
    private val refreshAccessRunning = java.util.concurrent.atomic.AtomicBoolean(false)
    private val refreshAccessPending = java.util.concurrent.atomic.AtomicBoolean(false)
    private val lastTransferUiEmitMs = java.util.concurrent.ConcurrentHashMap<String, Long>()

    /**
     * Re-checks notification access and background restrictions (the user
     * may have changed them in Settings) and asks Android to reconnect the listener if it's allowed
     * but not running. Coalesces rapid concurrent calls into at most one follow-up pass.
     */
    fun refreshNotificationAccess() {
        refreshAccessPending.set(true)
        if (!refreshAccessRunning.compareAndSet(false, true)) return
        scope.launch(Dispatchers.IO) {
            try {
                while (refreshAccessPending.compareAndSet(true, false)) {
                    val granted = NotificationListener.hasAccess(context)
                    val sensitive = granted && NotificationListener.canReadSensitive(context)
                    val becameSensitive = sensitive && !lastSensitiveAccess
                    lastSensitiveAccess = sensitive
                    val unrestricted = BackgroundAccess.isUnrestricted(context)
                    val companionAssociated = PcCompanion.isAssociated(context)
                    if (companionAssociated) {
                        PcCompanion.startObservingPresence(context)
                    }
                    val photoAccess = RecentPhotos.hasAccess(context)
                    val photoPartialAccess = RecentPhotos.hasPartialAccess(context)
                    val callAccess = PhoneCalls.hasAll(context)
                    val contactsAccess = PhoneContacts.canRead(context)
                    val smsAccess = PhoneSms.hasAll(context)
                    val inputEnabled = InputService.enabled(context)
                    val elevated = Elevated.running
                    val inputAccess = InputService.running || elevated || inputEnabled
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
                    val pcToggles = currentNode?.let { perPcToggles(it, _state.value.devices.map { d -> d.id }) }
                    _state.update {
                        it.copy(
                            notificationAccess = granted,
                            backgroundUnrestricted = unrestricted,
                            companionAssociated = companionAssociated,
                            photoAccess = photoAccess,
                            photoPartialAccess = photoPartialAccess,
                            callAccess = callAccess,
                            contactsAccess = contactsAccess,
                            smsAccess = smsAccess,
                            inputAccess = inputAccess,
                            elevated = elevated,
                            remoteInputRequested = if (inputAccess) false else it.remoteInputRequested,
                            dndAccess = dndAccess,
                            writeSettingsAccess = writeSettingsAccess,
                            storageAllFilesAccess = storageAllFilesAccess,
                            storageSafFolders = storageSafFolders,
                            localNetwork = localNetwork,
                            devices = if (pcToggles != null) {
                                it.devices.map { d ->
                                    d.copy(storageEnabled = d.id in pcToggles.storageAllowed, micEnabled = d.id !in pcToggles.micOff)
                                }
                            } else {
                                it.devices
                            },
                        )
                    }
                    if (granted && NotificationListener.instance == null) {
                        NotificationListenerService.requestRebind(NotificationListener.component(context))
                    } else if (becameSensitive) {
                        NotificationListener.instance?.refreshActive()
                    }
                    val currentScope = runScope
                    if (node != null && currentScope != null) {
                        currentScope.launch(Dispatchers.Main) {
                            if (granted) phoneMedia.start() else phoneMedia.stop()
                            if (_state.value.devices.any { it.online }) toggles.start() else toggles.stop()
                            if (photoAccess) photos.start() else photos.stop()
                            if (PhoneCalls.canFollow(context) || PhoneCalls.canReadLog(context)) calls.start() else calls.stop()
                            if (contactsAccess) contacts.start() else contacts.stop()
                            if (PhoneSms.canRead(context)) sms.start() else sms.stop()
                        }
                        enqueueNotificationOp {
                            it.updatePower(powerLevel(), capabilities(_state.value.notificationAccess))
                            if (photoAccess && (!prevPhotos || photoPartialAccess)) it.photosChanged()
                        }
                    }
                }
            } finally {
                refreshAccessRunning.set(false)
                if (refreshAccessPending.get()) {
                    refreshNotificationAccess()
                }
            }
        }
    }

    /** Voice recordings stored on this phone and their delivery status to PCs. */
    val recordings = RecordingsStore(this.context)

    override fun onEvent(event: Event) {
        if (event is Event.Transfer) {
            val transfer = event.transfer
            val isRunningProgress = transfer.status is TransferStatus.Running &&
                transfer.done > 0uL &&
                (transfer.total == 0uL || transfer.done < transfer.total)
            if (isRunningProgress) {
                val now = android.os.SystemClock.elapsedRealtime()
                val prev = lastTransferUiEmitMs[transfer.id] ?: 0L
                val wasRunning = _state.value.transfers.firstOrNull { it.id == transfer.id }?.status is TransferStatus.Running
                if (wasRunning && now - prev < 100L) {
                    return
                }
                lastTransferUiEmitMs[transfer.id] = now
            } else {
                lastTransferUiEmitMs.remove(transfer.id)
            }
        }
        if (event.affectsCoreState()) {
            _state.update { it.reduce(event) }
        }
        when (event) {
            is Event.Transfer -> onTransfer(event.transfer)
            is Event.MediaChanged -> {
                val pcName = _state.value.nameOf(event.id).orEmpty()
                scope.launch(Dispatchers.Default) {
                    pcMedia.update(event.id, pcName, event.players)
                }
            }
            is Event.NotificationPosted -> scope.launch(Dispatchers.Default) {
                val (peer, name) = peerOf(event.id)
                pcNotificationPoster.post(event.id, event.notification, peer, name)
            }
            is Event.NotificationRemoved -> scope.launch(Dispatchers.Default) {
                pcNotificationPoster.cancel(event.id, event.key)
            }
            is Event.NotificationsReset -> scope.launch(Dispatchers.Default) {
                val (peer, name) = peerOf(event.id)
                pcNotificationPoster.reset(event.id, event.items, peer, name)
            }
            is Event.PcAudioFrame -> pcSpeakerPlayer.playFrame(
                event.sampleRate.toInt(),
                event.channels.toInt(),
                event.pcm,
            )
            is Event.PcMirrorConfig -> {
                _pcMirrorConfig.value = PcMirrorConfigState(
                    pcId = event.id,
                    codec = event.codec,
                    width = event.width.toInt(),
                    height = event.height.toInt(),
                )
            }
            is Event.PcMirrorFrame -> {
                val pkt = PcMirrorPacket(
                    pcId = event.id,
                    keyframe = event.keyframe,
                    timeUs = event.timeUs.toLong(),
                    data = event.data,
                )
                if (pkt.keyframe) {
                    lastPcMirrorKeyframe = pkt
                }
                _pcMirrorPackets.tryEmit(pkt)
            }
            is Event.PcMirrorEnded -> {
                if (_pcMirrorConfig.value?.pcId == event.id) {
                    _pcMirrorConfig.value = null
                }
                if (lastPcMirrorKeyframe?.pcId == event.id) {
                    lastPcMirrorKeyframe = null
                }
            }
            // A PC that's gone isn't playing for this phone anymore; it
            // sends what plays when it's back.
            is Event.LinkChanged -> {
                if (event.link is Link.Offline) {
                    scope.launch(Dispatchers.Default) {
                        pcMedia.update(event.id, "", emptyList())
                    }
                    pcSpeakerPlayer.stop()
                    if (_pcMirrorConfig.value?.pcId == event.id) {
                        _pcMirrorConfig.value = null
                    }
                    if (lastPcMirrorKeyframe?.pcId == event.id) {
                        lastPcMirrorKeyframe = null
                    }
                } else if (event.link is Link.Online) {
                    wakeTimers.remove(event.id)?.cancel()
                    retryWaitingRecordings(event.id)
                    // Plugged into a PC with USB debugging allowed: it can start Elevated.
                    if (!Elevated.running) scope.launch(Dispatchers.IO) { Elevated.startOverUsb() }
                    scope.launch(Dispatchers.Default) {
                        accent.syncNow()
                    }
                }
            }
            is Event.DeviceAdded -> ConnectionService.start(context)
            is Event.DeviceRemoved -> {
                wakeTimers.remove(event.id)?.cancel()
                scope.launch(Dispatchers.Default) {
                    pcMedia.update(event.id, "", emptyList())
                    pcNotificationPoster.cancelAll(event.id)
                }
                pcSpeakerPlayer.stop()
                if (_pcMirrorConfig.value?.pcId == event.id) {
                    _pcMirrorConfig.value = null
                }
                if (lastPcMirrorKeyframe?.pcId == event.id) {
                    lastPcMirrorKeyframe = null
                }
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
            is Event.QueueChanged -> refreshSendQueue()
            is Event.QueueDroppedOnUnpair -> {
                if (event.droppedCount > 0u) {
                    val name = event.deviceName.ifBlank { context.getString(R.string.your_pc) }
                    _messages.tryEmit(
                        context.getString(
                            R.string.send_queue_dropped_unpair,
                            event.droppedCount.toInt(),
                            name,
                        ),
                    )
                }
                refreshSendQueue()
            }
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
                if (published.size == 1 && transfer.files <= 1u && !transfer.recording) {
                    val first = published.first()
                    val label = transfer.names.firstOrNull()?.takeIf(String::isNotBlank)
                        ?: first.name.ifBlank { "File" }
                    PhoneClipboard.setFileUri(context, first.uri, label)
                }
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

    /**
     * Elevated over USB (`docs/protocol/elevated-usb.md`): asks each connected
     * PC to start this phone's helper; only one it's plugged into with USB
     * debugging allowed can. Returns the helper's port, or `null`.
     */
    suspend fun elevatedUsbStart(apk: String, token: String, grants: List<String>): Int? {
        startJob?.join()
        val node = node ?: return null
        for (pc in _state.value.devices.filter { it.online }) {
            val port = runCatching { node.elevatedUsbStart(pc.id, apk, token, grants) }.getOrNull()
            if (port != null) return port.toInt()
        }
        return null
    }

    /** Accepted codecs from the connected PC; an unknown receiver gets H.264. */
    suspend fun webcamCodecs(pcId: String): List<String> {
        startJob?.join()
        return runCatching { node?.webcamCodecs(pcId) }.getOrNull() ?: listOf("h264")
    }

    /** Accepted screen codecs from the connected PC; unknown receivers use H.264. */
    suspend fun mirrorCodecs(pcId: String): List<String> {
        startJob?.join()
        return runCatching { node?.mirrorCodecs(pcId) }.getOrNull() ?: listOf("h264")
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

    /** Opens this phone's microphone stream to [pcId] (docs/protocol/mic.md); null if it can't. */
    suspend fun micOpen(pcId: String): MirrorStream? {
        startJob?.join()
        val node = node ?: return null
        return try {
            node.micOpen(pcId)
        } catch (e: NectarlinkException) {
            _messages.tryEmit(describe(e))
            null
        }
    }

    /** Whether [id] is a paired phone or tablet (not a PC), and its name. */
    private fun peerOf(id: String): Pair<Boolean, String> {
        val device = _state.value.devices.firstOrNull { it.id == id }
        return (device?.isAndroid == true) to device?.name.orEmpty()
    }

    /** Dismisses a paired phone's or tablet's notification there (dismissed here). */
    fun dismissPeerNotification(id: String, key: String) {
        command { it.dismissPeerNotification(id, key) }
    }

    /** Stops this phone's microphone stream to [pcId] and tells the PC. */
    fun micStop(pcId: String) {
        command { it.micStop(pcId) }
    }

    /** The PC's request for the microphone is answered or withdrawn: take it off Home and the shade. */
    fun clearMicRequest(pcId: String) {
        app.nectarlink.android.mic.MicRequests.dismiss(context, pcId)
        _state.update { if (it.micRequestFrom == pcId) it.copy(micRequestFrom = null) else it }
    }

    /** Not now: the PC stops waiting. */
    fun declineMicRequest(pcId: String) {
        clearMicRequest(pcId)
        micStop(pcId)
    }

    /**
     * Lets [pcId] ask for this phone's microphone, or not (on by default). Turning it off
     * also ends a stream to that PC, and the core tells the PC.
     */
    fun setMicEnabled(pcId: String, enabled: Boolean) {
        _state.update { it.withMicEnabled(pcId, enabled) }
        if (!enabled) app.nectarlink.android.mic.MicRequests.dismiss(context, pcId)
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            runCatching { currentNode.setDeviceToggle(pcId, app.nectarlink.android.mic.MicService.TOGGLE_NAME, enabled) }
                .onFailure { Log.w(TAG, "can't change the microphone toggle", it) }
        }
    }

    /** Which PCs may browse this phone's storage, and which may not ask for its microphone. */
    private class PerPcToggles(val storageAllowed: Set<String>, val micOff: Set<String>)

    private fun perPcToggles(node: NectarlinkNode, ids: List<String>): PerPcToggles {
        val storage = mutableSetOf<String>()
        val micOff = mutableSetOf<String>()
        for (id in ids) {
            val toggles = runCatching { node.deviceToggles(id) }.getOrNull() ?: continue
            if (toggles.any { it.name == PhoneStorage.TOGGLE_NAME && it.enabled }) storage += id
            if (toggles.any { it.name == app.nectarlink.android.mic.MicService.TOGGLE_NAME && !it.enabled }) micOff += id
        }
        return PerPcToggles(storage, micOff)
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
     */
    fun sendFiles(pcId: String, uris: List<Uri>, handoff: Boolean = false) {
        scope.launch(Dispatchers.IO) {
            send(pcId, OutgoingFiles.open(context, uris), handoff)
        }
    }

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
            val wasOnline = _state.value.device(pcId)?.online == true
            try {
                val singleFileName = if (files.size == 1) {
                    when (val f = files[0]) {
                        is FileToSend.Fd -> f.name.takeIf { f.folder == null }
                        is FileToSend.Path -> f.name.takeIf { f.folder == null }
                    }
                } else {
                    null
                }
                val returnedId = if (handoff && singleFileName != null && app.nectarlink.core.isSafeHandoffDocument(singleFileName)) {
                    node.sendHandoffFiles(pcId, files)
                } else {
                    node.sendFiles(pcId, files)
                }
                refreshSendQueue()
                if (!wasOnline) {
                    val name = _state.value.nameOf(pcId).orEmpty()
                    val qid = runCatching { node.latestQueuedIdForPeer(pcId) }.getOrNull()
                        ?: returnedId.takeIf { it.isNotBlank() }
                    emitOfflineQueuedNotice(pcId, name, qid)
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
        scope.launch(Dispatchers.IO) { runCatching { node?.acceptTransfer(id) } }
    }

    fun cancelTransfer(id: String) {
        scope.launch(Dispatchers.IO) { runCatching { node?.cancelTransfer(id) } }
    }

    fun pauseTransfer(id: String) {
        scope.launch(Dispatchers.IO) { runCatching { node?.pauseTransfer(id) } }
    }

    fun resumeTransfer(id: String) {
        scope.launch(Dispatchers.IO) { runCatching { node?.resumeTransfer(id) } }
    }

    // ---- Send Queue ----

    private val refreshQueueRunning = java.util.concurrent.atomic.AtomicBoolean(false)
    private val refreshQueuePending = java.util.concurrent.atomic.AtomicBoolean(false)

    fun refreshSendQueue() {
        refreshQueuePending.set(true)
        if (!refreshQueueRunning.compareAndSet(false, true)) return
        scope.launch(Dispatchers.IO) {
            try {
                while (refreshQueuePending.compareAndSet(true, false)) {
                    val currentNode = node ?: break
                    val items = runCatching { currentNode.sendQueue(null).map { it.toUi() } }.getOrDefault(emptyList())
                    _state.update { prev ->
                        val merged = items.map { item ->
                            if (item.kind.isFileBacked() && item.status is app.nectarlink.core.FfiQueueStatus.Sending) {
                                val targetId = item.activeTransferId ?: item.id
                                val liveTransfer = prev.transfers.firstOrNull {
                                    it.id == targetId && it.direction == TransferDirection.OUTGOING
                                }
                                val prevItem = prev.sendQueue.firstOrNull { it.id == item.id }
                                val liveDone = maxOf(
                                    item.bytesDone,
                                    liveTransfer?.done?.toLong() ?: 0L,
                                    prevItem?.bytesDone ?: 0L,
                                )
                                val liveTotal = if (item.totalBytes > 0L) {
                                    item.totalBytes
                                } else {
                                    liveTransfer?.total?.toLong() ?: prevItem?.totalBytes ?: 0L
                                }
                                item.copy(
                                    totalBytes = liveTotal.coerceAtLeast(0L),
                                    bytesDone = liveDone.coerceIn(0L, liveTotal.coerceAtLeast(0L)),
                                )
                            } else {
                                item
                            }
                        }
                        prev.copy(sendQueue = merged)
                    }
                }
            } finally {
                refreshQueueRunning.set(false)
                if (refreshQueuePending.get()) {
                    refreshSendQueue()
                }
            }
        }
    }

    fun cancelQueueItem(id: String) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.cancelQueueItem(id) }
            refreshSendQueue()
        }
    }

    fun cancelAllQueueItems(peerId: String? = null) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.cancelAllQueueItems(peerId) }
            refreshSendQueue()
        }
    }

    fun retryQueueItem(id: String) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.retryQueueItem(id) }
            refreshSendQueue()
        }
    }

    fun clearFinishedQueueItems(peerId: String? = null) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.clearFinishedQueueItems(peerId) }
            refreshSendQueue()
        }
    }

    private fun emitOfflineQueuedNotice(pcId: String, pcName: String, queueId: String? = null) {
        val resolvedQueueId = queueId ?: runCatching { node?.latestQueuedIdForPeer(pcId) }.getOrNull()
        val displayName = pcName.ifBlank { context.getString(R.string.your_pc) }
        val message = context.getString(R.string.send_queue_offline_toast, displayName)
        _queuedNotices.tryEmit(
            OfflineQueuedNotice(
                message = message,
                queueId = resolvedQueueId,
                peerId = pcId,
            ),
        )
    }

    // ---- LocalSend interop & Mobile Data (Away mode) ----

    fun setAwayMode(enabled: Boolean) {
        app.nectarlink.android.ui.Preferences.setAwayModeEnabled(context, enabled)
        _state.update { it.copy(awayMode = enabled) }
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            runCatching { node?.setAwayMode(enabled) }
        }
    }

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

    @Volatile private var lastAutoSentClip: Pair<String, String?>? = null
    @Volatile private var lastAutoSentAtMs: Long = 0L

    /**
     * Sends text (and optional HTML) to every connected PC (or to `pcId` when chosen via Direct Share);
     * returns what to tell the user. Waits for the core when the app was just launched.
     */
    suspend fun sendClipboard(text: String, pcId: String? = null, html: String? = null): String =
        sendToPcs(image = false, pcId = pcId) { node, pc -> node.sendClipboardRich(pc, text, html, null) }

    /**
     * Automatically sends non-sensitive text copied in the background while Elevated mode is running.
     * Skips clips NectarLink itself wrote from a PC (`PhoneClipboard.isRecentEcho`).
     */
    fun onElevatedClipboardChanged(text: String, html: String?) {
        if (text.isBlank() || PhoneClipboard.isRecentEcho(text, html)) return
        val now = android.os.SystemClock.elapsedRealtime()
        val key = text to html
        if (lastAutoSentClip == key && (now - lastAutoSentAtMs) in 0..1_500L) return
        lastAutoSentClip = key
        lastAutoSentAtMs = now
        PhoneClipboard.recordLocalCopy(text, html)
        scope.launch(Dispatchers.IO) {
            if (Build.FINGERPRINT.startsWith("generic") || Build.HARDWARE.contains("ranchu")) {
                delay(80)
            }
            startJob?.join()
            val currentNode = node ?: return@launch
            val onlinePcs = _state.value.devices.filter { it.online }
            for (pc in onlinePcs) {
                runCatching { currentNode.sendClipboardRich(pc.id, text, html, null) }
                    .onFailure { Log.d(TAG, "auto clipboard not sent to ${pc.id}: $it") }
            }
        }
    }

    private fun targetPcsForSend(pcId: String?): List<Device> {
        val all = _state.value.devices
        if (pcId != null) {
            return all.filter { it.id == pcId }
        }
        val online = all.filter { it.online }
        return if (online.isNotEmpty()) online else all
    }

    /** Sends a copied image to the connected PCs (or to `pcId`, queueing if offline); returns what to tell the user. */
    suspend fun sendClipboardImage(uri: Uri, pcId: String? = null): String {
        startJob?.join()
        val targets = targetPcsForSend(pcId)
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
        val pcs = targetPcsForSend(pcId)
        if (pcs.isEmpty()) return context.getString(R.string.clip_no_pc)
        val sentTo = mutableListOf<String>()
        val queuedTo = mutableListOf<Device>()
        var lastQueueId: String? = null
        var failure: String? = null
        for (pc in pcs) {
            val wasOnline = pc.online
            try {
                send(node, pc.id)
                if (wasOnline) {
                    sentTo += pc.name
                } else {
                    queuedTo += pc
                    lastQueueId = runCatching { node.latestQueuedIdForPeer(pc.id) }.getOrNull()
                }
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
        if (queuedTo.isNotEmpty()) {
            refreshSendQueue()
            val names = queuedTo.joinToString { it.name }
            val msg = context.getString(R.string.send_queue_offline_toast, names)
            _queuedNotices.tryEmit(
                OfflineQueuedNotice(
                    message = msg,
                    queueId = if (queuedTo.size == 1) lastQueueId else null,
                    peerId = queuedTo.singleOrNull()?.id,
                ),
            )
            if (sentTo.isEmpty()) {
                return msg
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
            runCatching { node?.clearTimeline() }.onFailure {
                Log.w(TAG, "couldn't clear the Timeline", it)
                _messages.tryEmit(context.getString(R.string.data_clear_failed))
            }
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
            runCatching { node?.clearMessageCache() }.onFailure {
                Log.w(TAG, "couldn't clear the message cache", it)
                _messages.tryEmit(context.getString(R.string.data_clear_failed))
            }
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
            runCatching { node?.clearReceivedFileHistory() }.onFailure {
                Log.w(TAG, "couldn't clear the received file history", it)
                _messages.tryEmit(context.getString(R.string.data_clear_failed))
            }
            refreshTimeline()
            refreshDataRetention()
        }
    }

    fun clearEverything() {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val failed = runCatching { node?.clearAllLocalData() }
                .onFailure { Log.w(TAG, "couldn't clear local data", it) }
                .isFailure
            listOf("mms", "continuity_camera", "clipboard", "received").forEach { dirName ->
                java.io.File(context.cacheDir, dirName).listFiles()?.forEach { file ->
                    runCatching { file.deleteRecursively() }
                }
            }
            _state.update { s -> s.copy(transfers = s.transfers.filterNot { it.isFinished() }) }
            refreshClipboardHistory()
            refreshTimeline()
            refreshSendQueue()
            refreshDataRetention()
            _messages.tryEmit(
                context.getString(if (failed) R.string.data_clear_failed else R.string.data_cleared_everything_toast),
            )
        }
    }

    fun resendTimelineEntry(entry: TimelineEntry) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            val pcName = _state.value.nameOf(entry.deviceId).orEmpty().ifEmpty { context.getString(R.string.your_pc) }
            val wasOnline = _state.value.device(entry.deviceId)?.online == true
            when (entry.kind) {
                TimelineKind.CLIP -> {
                    val clipId = entry.refId ?: return@launch
                    try {
                        currentNode.resendClipboardHistory(clipId, entry.deviceId)
                        if (wasOnline) {
                            _messages.tryEmit(context.getString(R.string.clip_sent_to, pcName))
                        } else {
                            refreshSendQueue()
                            emitOfflineQueuedNotice(entry.deviceId, pcName)
                        }
                    } catch (e: NectarlinkException) {
                        _messages.tryEmit(describe(e))
                    }
                }
                TimelineKind.LINK -> {
                    val url = entry.target.ifEmpty { entry.title }
                    val msg = openLinkOnPc(entry.deviceId, url)
                    if (wasOnline) {
                        _messages.tryEmit(msg)
                    }
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
            // Play on phone: shows a PC's playback, streamed from the PC.
            listOf("media.watch") +
            // Shows a paired phone's or tablet's notifications.
            listOf("notify.show") +
            // Phone as microphone: asks for the permission when a PC first wants it.
            (if (context.packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE)) listOf("mic.stream") else emptyList()) +
            toggles.capabilities() +
            (if (notificationAccess) NOTIFICATION_CAPABILITIES + MEDIA_CAPABILITIES else emptyList()) +
            (if (notificationAccess && NotificationListener.canReadSensitive(context)) listOf("notify.sensitive") else emptyList()) +
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
            (if (Elevated.running) listOf("clip.read.auto") else emptyList()) +
            // Apps in windows of their own run on displays the Elevated helper makes.
            (if (Elevated.running && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) listOf("mirror.virtual_display") else emptyList()) +
            // Asked for when a PC wants the sound, or streamed via Elevated helper.
            (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q || Elevated.running) listOf("mirror.audio.playback") else emptyList()) +
            // The same sound on its own, without the screen.
            (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q || Elevated.running) listOf("mirror.sound") else emptyList()) +
            (if (app.nectarlink.android.mirror.ScreenEncoder.supportsHevcDecoder()) listOf("mirror.decode.hevc") else emptyList())

    /**
     * Triggers an immediate network/presence reconnect (e.g. when woken by
     * CompanionDeviceService on BLE proximity beacon appearance).
     */
    fun reconnectNow() {
        start()
        scope.launch {
            node?.networkChanged()
            if (!Elevated.running) Elevated.start()
        }
    }

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
                is NectarlinkException.InvalidPairingLink -> PairingFailure.Other("not a NectarLink code")
                is NectarlinkException.Denied -> PairingFailure.Rejected
                else -> PairingFailure.Unreachable
            }
            _state.update { s -> s.copy(pairing = PairingState.Failed(failure)) }
        }
    }

    /**
     * A pairing link for another phone or tablet to scan (this device hosts the pairing,
     * as a PC does); null if the core can't make one now.
     */
    suspend fun pairingLink(): String? {
        startJob?.join()
        val node = node ?: return null
        return try {
            node.pairingStartQr()
        } catch (e: NectarlinkException) {
            _messages.tryEmit(describe(e))
            null
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
        _state.update {
            it.copy(
                pairing = if (matches) PairingState.Confirmed(comparing.peer, comparing.code)
                else PairingState.Failed(PairingFailure.Declined),
            )
        }
        scope.launch(Dispatchers.IO) { runCatching { node?.pairingConfirm(matches) } }
    }

    /** Keeps local discovery working while the pairing screen is open. */
    fun setPairingScreenOpen(open: Boolean) {
        pairingMulticast?.runCatching { if (open) acquire() else release() }
    }

    fun resetPairing() {
        _state.update { it.copy(pairing = PairingState.Idle) }
        scope.launch(Dispatchers.IO) { runCatching { node?.pairingCancel() } }
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
        val wasOnline = _state.value.device(id)?.online == true
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
            if (wasOnline) {
                context.getString(R.string.link_opened_on, name)
            } else {
                refreshSendQueue()
                val qid = runCatching { node.latestQueuedIdForPeer(id) }.getOrNull()
                val displayName = name.ifBlank { context.getString(R.string.your_pc) }
                val msg = context.getString(R.string.send_queue_offline_toast, displayName)
                _queuedNotices.tryEmit(
                    OfflineQueuedNotice(
                        message = msg,
                        queueId = qid,
                        peerId = id,
                    ),
                )
                msg
            }
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

    private val pendingMoveLock = Any()
    private var pendingMovePcId: String? = null
    private var pendingMoveDx: Float = 0f
    private var pendingMoveDy: Float = 0f
    private var pendingMoveQueued: Boolean = false

    /** Moves a PC's cursor by `(dx, dy)` logical pixels over QUIC datagrams, coalescing rapid 120Hz touch samples. */
    fun remoteMove(id: String, dx: Float, dy: Float) {
        val shouldQueue = synchronized(pendingMoveLock) {
            if (pendingMovePcId != null && pendingMovePcId != id) {
                pendingMoveDx = 0f
                pendingMoveDy = 0f
            }
            pendingMovePcId = id
            pendingMoveDx += dx
            pendingMoveDy += dy
            if (!pendingMoveQueued) {
                pendingMoveQueued = true
                true
            } else {
                false
            }
        }
        if (shouldQueue) {
            enqueueRemoteOp { node ->
                val (targetId, totalDx, totalDy) = synchronized(pendingMoveLock) {
                    val t = pendingMovePcId
                    val x = pendingMoveDx
                    val y = pendingMoveDy
                    pendingMoveDx = 0f
                    pendingMoveDy = 0f
                    pendingMoveQueued = false
                    Triple(t, x, y)
                }
                if (targetId != null && (totalDx != 0f || totalDy != 0f)) {
                    runCatching { node.remoteMove(targetId, totalDx, totalDy) }
                }
            }
        }
    }

    /** Scrolls on a PC (`dy` > 0 scrolls down, `dx` > 0 scrolls right, in wheel notches). */
    fun remoteScroll(id: String, dx: Float, dy: Float, fast: Boolean = true) {
        enqueueRemoteOp { runCatching { it.remoteScroll(id, dx, dy, fast) } }
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
        enqueueRemoteOp { runCatching { it.remoteLaser(id, on, x, y) } }
    }

    /**
     * Asks a paired PC to start streaming its desktop screen to this phone (`mirror.start`,
     * session 0), or with [watch], what that PC player plays, with its sound (Play on phone).
     */
    fun startPcMirror(
        id: String,
        maxSize: Int = 1280,
        fps: Int = 30,
        bitrate: Int = 4_000_000,
        watch: String? = null,
    ) {
        command {
            it.startPcMirror(
                id,
                maxSize.coerceIn(320, 1920).toUInt(),
                fps.coerceIn(10, 60).toUInt(),
                bitrate.coerceIn(500_000, 20_000_000).toUInt(),
                watch,
            )
        }
    }

    /** Stops an active PC-to-phone screen mirror session (`mirror.stop`, session 0). */
    fun stopPcMirror(id: String) {
        if (_pcMirrorConfig.value?.pcId == id) {
            _pcMirrorConfig.value = null
        }
        if (lastPcMirrorKeyframe?.pcId == id) {
            lastPcMirrorKeyframe = null
        }
        scope.launch(Dispatchers.IO) {
            runCatching { node?.mirrorStop(id, 0u) }
        }
    }

    /** Requests an immediate IDR keyframe from a paired PC streaming its screen. */
    fun requestPcMirrorKeyframe(id: String) {
        enqueueRemoteOp { runCatching { it.requestPcMirrorKeyframe(id) } }
    }

    /** Sends a normalized touch, scroll, key, or text event to a paired PC during reverse mirroring. */
    fun sendPcMirrorInput(id: String, input: app.nectarlink.core.MirrorInputEvent) {
        enqueueRemoteOp { runCatching { it.sendPcMirrorInput(id, input) } }
    }

    private fun remoteCommand(onStatus: ((RemoteAccess) -> Unit)?, block: suspend (NectarlinkNode) -> Unit) {
        enqueueRemoteOp { node ->
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

    /** Sets the PC's master volume (`0..=100` or `200`/`201` for phone speaker relay) and/or mute state (`pc.audio.set`). */
    fun setPcAudio(id: String, volume: Int? = null, muted: Boolean? = null) {
        val isSpeakerRelayToggle = volume == 200 || volume == 201
        val wireVol = if (isSpeakerRelayToggle) {
            volume.toUByte()
        } else {
            volume?.coerceIn(0, 100)?.toUByte()
        }
        // Optimistic local state update so the slider, mute button, and phone speaker switch don't snap back while the RPC runs.
        _state.update { s ->
            val current = s.device(id)?.deckState
            if (current != null) {
                val updatedOutputs = when (volume) {
                    201 -> if (current.outputDevices.any { it.id == PHONE_SPEAKER_OUTPUT }) {
                        current.outputDevices
                    } else {
                        current.outputDevices + app.nectarlink.core.AudioOutputDevice(
                            id = PHONE_SPEAKER_OUTPUT,
                            name = "Phone speaker (NectarLink)",
                            isDefault = false,
                        )
                    }
                    200 -> current.outputDevices.filterNot { it.id == PHONE_SPEAKER_OUTPUT }
                    else -> current.outputDevices
                }
                s.reduce(
                    Event.DeckState(
                        id = id,
                        state = current.copy(
                            volume = if (isSpeakerRelayToggle) current.volume else (wireVol ?: current.volume),
                            muted = muted ?: current.muted,
                            outputDevices = updatedOutputs,
                        ),
                    ),
                )
            } else {
                s
            }
        }
        enqueueRemoteOp { node ->
            try {
                node.setPcAudio(id, wireVol, muted)
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
    /** Stops reconnecting to a paired PC by itself ("Stop reconnecting"), or starts again. */
    fun setReconnect(pcId: String, on: Boolean) {
        _state.update { it.reduce(Event.ReconnectChanged(pcId, on)) }
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            runCatching { currentNode.setReconnect(pcId, on) }
                .onFailure { Log.w(TAG, "set reconnect failed", it) }
        }
    }

    fun setStorageEnabled(pcId: String, enabled: Boolean) {
        _state.update { it.withStorageEnabled(pcId, enabled) }
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node ?: return@launch
            runCatching { currentNode.setDeviceToggle(pcId, PhoneStorage.TOGGLE_NAME, enabled) }
            enqueueNotificationOp {
                it.updatePower(powerLevel(), capabilities(_state.value.notificationAccess))
            }
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

    /**
     * Play on phone: up to [length] bytes from [offset] of the video file a PC offered.
     * Blocks; call off the main thread.
     */
    fun readWatchFile(pcId: String, offset: Long, length: Long): ByteArray {
        val running = node ?: throw java.io.IOException("NectarLink isn't running")
        return running.watchFileRead(pcId, offset.toULong(), length.toULong()).data
    }

    /** Lists files and folders in `path` on a paired PC (`storage.list`). */
    fun listPcFiles(
        deviceId: String,
        path: String,
        onResult: (PcFolder) -> Unit,
    ) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val folder = try {
                PcFolder(node?.listRemoteStorage(deviceId, path).orEmpty())
            } catch (e: NectarlinkException) {
                Log.i(TAG, "can't list a PC's folder: $e")
                PcFolder(
                    emptyList(),
                    when (e) {
                        is NectarlinkException.Denied -> R.string.pc_files_not_shared
                        is NectarlinkException.Offline, is NectarlinkException.NotPaired -> R.string.pc_files_offline
                        is NectarlinkException.Unsupported -> R.string.pc_files_update
                        is NectarlinkException.NotFound -> R.string.pc_files_not_found
                        else -> R.string.pc_files_error
                    },
                )
            }
            withContext(Dispatchers.Main) {
                onResult(folder)
            }
        }
    }

    /** Downloads `remotePath` from a paired PC (`storage.read`) and saves it into Downloads. */
    fun downloadPcFile(
        deviceId: String,
        remotePath: String,
        fileName: String,
        onSaved: (Uri?) -> Unit,
    ) {
        scope.launch(Dispatchers.IO) {
            startJob?.join()
            val currentNode = node
            if (currentNode == null) {
                withContext(Dispatchers.Main) { onSaved(null) }
                return@launch
            }
            val cacheDir = java.io.File(context.cacheDir, "pc_downloads").apply { mkdirs() }
            val safeName = fileName.replace(Regex("""[\\/:*?"<>|]"""), "_").ifBlank { "file" }
            val tempFile = java.io.File(cacheDir, "${System.currentTimeMillis()}_$safeName")
            val ok = runCatching {
                currentNode.downloadRemoteStorage(deviceId, remotePath, tempFile.absolutePath)
            }.getOrDefault(false)
            val savedUri = if (ok && tempFile.exists()) {
                ReceivedFiles.saveToDownloads(context, tempFile, safeName)
            } else {
                null
            }
            runCatching { tempFile.delete() }
            withContext(Dispatchers.Main) {
                onSaved(savedUri)
            }
        }
    }

    /** Asks an online relay peer to broadcast a Wake-on-LAN magic packet (`wake.relay`). */
    fun wakeViaRelay(relayPeerId: String, mac: String, broadcast: String? = null) {
        command { it.wakeViaRelay(relayPeerId, mac, broadcast) }
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
        is NectarlinkException.QueueFull -> e.reason.ifBlank { context.getString(R.string.send_queue_full) }
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
        val CLIPBOARD_CAPABILITIES = listOf("clip.write", "clip.share", "mirror.capture", "mirror.view")
        /** With access to the phone's photos. */
        val PHOTO_CAPABILITIES = listOf("photos.read")
    }
}

/** A folder on a PC: its files, or why they can't be listed (a string resource). */
data class PcFolder(
    val entries: List<RemoteStorageEntry>,
    @androidx.annotation.StringRes val problem: Int? = null,
)
