// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import app.nectarlink.android.webcam.WebcamRequest
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.Event
import app.nectarlink.core.Feature
import app.nectarlink.core.FfiQueueItem
import app.nectarlink.core.FfiQueueKind
import app.nectarlink.core.FfiQueueStatus
import app.nectarlink.core.Link
import app.nectarlink.core.LocalSendPeer
import app.nectarlink.core.PairedDevice
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.Transfer

/** An item in the persistent outbound Send Queue as rendered by the UI. */
@androidx.compose.runtime.Immutable
data class QueueItemUi(
    val id: String,
    val peerId: String,
    val peerName: String,
    val kind: FfiQueueKind,
    val title: String,
    val detail: String,
    val totalBytes: Long,
    val bytesDone: Long = 0L,
    val filesTotal: Int = 0,
    val filesDone: Int = 0,
    val createdAtMs: Long,
    val expiresAtMs: Long,
    val attempts: Int,
    val status: FfiQueueStatus,
    val error: String?,
    val activeTransferId: String? = null,
)

internal fun FfiQueueKind.isFileBacked(): Boolean = when (this) {
    FfiQueueKind.FILES,
    FfiQueueKind.FOLDER,
    FfiQueueKind.HANDOFF_DOCUMENT,
    FfiQueueKind.RECORDING,
    -> true
    FfiQueueKind.CLIPBOARD,
    FfiQueueKind.CLIPBOARD_IMAGE,
    FfiQueueKind.LINK,
    FfiQueueKind.TASK_NOTIFY,
    FfiQueueKind.CAMERA_REQUEST,
    -> false
}

/** Normalizes multi-line queue titles to a single clean line for display. */
fun formatQueueItemTitle(title: String, fallback: String, maxChars: Int = 44): String {
    val singleLine = title
        .replace("\r\n", " ")
        .replace('\n', ' ')
        .replace('\r', ' ')
        .replace(Regex("\\s+"), " ")
        .trim()
    if (singleLine.isEmpty()) {
        return fallback
    }
    return if (singleLine.length > maxChars && maxChars > 3) {
        singleLine.take(maxChars - 3).trimEnd() + "..."
    } else {
        singleLine
    }
}

internal fun FfiQueueItem.toUi(): QueueItemUi = QueueItemUi(
    id = id,
    peerId = deviceId,
    peerName = deviceName,
    kind = kind,
    title = title,
    detail = detail,
    totalBytes = bytesTotal.toLong().coerceAtLeast(0L),
    bytesDone = bytesDone.toLong().coerceAtLeast(0L),
    filesTotal = filesTotal.toInt().coerceAtLeast(0),
    filesDone = filesDone.toInt().coerceAtLeast(0),
    createdAtMs = createdAt * 1000L,
    expiresAtMs = expiresAt * 1000L,
    attempts = attempts.toInt(),
    status = status,
    error = when (val s = status) {
        is FfiQueueStatus.Failed -> s.error
        is FfiQueueStatus.Expired -> s.error
        else -> null
    },
    activeTransferId = if (kind.isFileBacked()) id else null,
)

/** Whether the core is running. */
sealed interface CoreStatus {
    data object Starting : CoreStatus
    data class Ready(val deviceId: String) : CoreStatus
    data class Failed(val message: String) : CoreStatus
}

/** The pairing screen's state. */
sealed interface PairingState {
    data object Idle : PairingState
    /** Joining a PC from a scanned QR code. */
    data object Joining : PairingState
    /** Dialing a nearby PC that is in pairing mode. */
    data class Connecting(val peer: String) : PairingState
    /** Both screens show this code; the user confirms it matches. */
    data class Comparing(val peer: String, val code: String) : PairingState
    /** This user confirmed; waiting for the other device. */
    data class Confirmed(val peer: String, val code: String) : PairingState
    data class Paired(val name: String) : PairingState
    data class Failed(val failure: PairingFailure) : PairingState
}

/** Wake-on-LAN progress for an offline PC. */
enum class WakeState {
    Idle,
    /** Magic packets sent; waiting up to ~60 s for the PC to connect. */
    Waking,
    /** ~60 s passed without the PC connecting; show what to check. */
    TimedOut,
}

/** A paired device as the UI shows it. */
@androidx.compose.runtime.Immutable
data class Device(
    val id: String,
    val name: String,
    val kind: app.nectarlink.core.DeviceKind,
    val model: String?,
    val accent: UInt? = null,
    val pairedAt: Long,
    val link: Link,
    val canWake: Boolean = false,
    val wakeState: WakeState = WakeState.Idle,
    /** Whether this phone reconnects to it by itself; off after "Stop reconnecting". */
    val reconnect: Boolean = true,
    val battery: app.nectarlink.core.Battery? = null,
    val power: PowerLevel = PowerLevel.NOT_APPLICABLE,
    val features: List<Feature> = emptyList(),
    val deckLayout: app.nectarlink.core.DeckLayout? = null,
    val deckState: app.nectarlink.core.DeckState? = null,
    /** Whether this PC is allowed to browse this phone's storage in File Explorer (off by default). */
    val storageEnabled: Boolean = false,
    /** Whether this PC may ask for this phone's microphone (on by default). */
    val micEnabled: Boolean = true,
) {
    /** A paired phone or tablet rather than a PC. */
    val isAndroid: Boolean
        get() = kind == app.nectarlink.core.DeviceKind.PHONE || kind == app.nectarlink.core.DeviceKind.TABLET

    val online: Boolean get() = link is Link.Online

    /** Whether a feature works with this device now (the capability matrix). */
    fun has(feature: String): Boolean =
        features.any { it.id == feature && it.status is app.nectarlink.core.FeatureStatus.Available }

    /** Whether a feature works at all now, fully or in a limited form (e.g. send by tap only). */
    fun usable(feature: String): Boolean =
        features.any {
            it.id == feature &&
                (it.status is app.nectarlink.core.FeatureStatus.Available || it.status is app.nectarlink.core.FeatureStatus.Partial)
        }
}

/** The output a PC's deck state lists while it plays its sound on this phone. */
const val PHONE_SPEAKER_OUTPUT = "nectarlink:phone_speaker"

private val deviceOrder: Comparator<Device> =
    compareByDescending<Device> { it.online }.thenBy { it.pairedAt }

/** Everything the UI renders, folded from core events. */
@androidx.compose.runtime.Immutable
data class CoreState(
    val status: CoreStatus = CoreStatus.Starting,
    /** Paired devices, online first then in pairing order. */
    val devices: List<Device> = emptyList(),
    val discovered: List<DiscoveredDevice> = emptyList(),
    val pairing: PairingState = PairingState.Idle,
    /** ID of the device making this phone ring, or null. */
    val ringingPeer: String? = null,
    val ringingDevices: Set<String> = emptySet(),
    /** Whether the user let NectarLink read notifications (to mirror them). */
    val notificationAccess: Boolean = false,
    /** Whether Android lets NectarLink reach the local network (Android 17). */
    val localNetwork: Boolean = true,
    /** Whether the Elevated helper runs (wireless debugging). */
    val elevated: Boolean = false,
    /** Whether the user turned on control from the PC (the accessibility service). */
    val inputAccess: Boolean = false,
    /** Whether a paired PC tried to inject mouse/keyboard input while input access is off. */
    val remoteInputRequested: Boolean = false,
    /** Whether the user let NectarLink read and send texts (for PCs). */
    val smsAccess: Boolean = false,
    /** Whether the user let NectarLink follow and answer calls (to show them on PCs). */
    val callAccess: Boolean = false,
    /** Whether the user let NectarLink read contacts (for PCs). */
    val contactsAccess: Boolean = false,
    /** Whether the user let NectarLink see the phone's photos (to show them on PCs). */
    val photoAccess: Boolean = false,
    /** Whether Android 14+ "Select photos and videos" partial access is active. */
    val photoPartialAccess: Boolean = false,
    /** Whether Android lets NectarLink run unrestricted in the background. */
    val backgroundUnrestricted: Boolean = true,
    /** Whether Companion Device Manager association is active for waking when a PC BLE beacon is nearby. */
    val companionAssociated: Boolean = false,
    /** Whether the user allowed Do Not Disturb / silent mode control from a PC. */
    val dndAccess: Boolean = false,
    /** Whether the user allowed screen brightness changes from a PC. */
    val writeSettingsAccess: Boolean = false,
    /** Whether Android "All files access" (`MANAGE_EXTERNAL_STORAGE`) is granted. */
    val storageAllFilesAccess: Boolean = false,
    /** Names of folders picked with the Storage Access Framework (when All files access is off). */
    val storageSafFolders: List<String> = emptyList(),
    /** ID of a paired PC that asked to browse this phone's storage while `storage` is off. */
    val storageRequestedFrom: String? = null,
    /** Pending request from a paired PC asking to use this phone's camera as a webcam. */
    val webcamRequest: WebcamRequest? = null,
    /** ID of a paired PC asking for this phone's microphone, until the user answers. */
    val micRequestFrom: String? = null,
    /** File transfers, newest first (running ones and the latest finished). */
    val transfers: List<Transfer> = emptyList(),
    /** Persistent outbound send queue items waiting to send, sending, failed, or expired. */
    val sendQueue: List<QueueItemUi> = emptyList(),
    /** Whether the encrypted local clipboard history is enabled. */
    val clipboardHistoryEnabled: Boolean = true,
    /** The last 50 clips exchanged with paired PCs (pinned first, then newest first). */
    val clipboardHistory: List<ClipboardHistoryEntry> = emptyList(),
    /** Recent timeline entries (first page, newest first). */
    val timeline: List<TimelineEntry> = emptyList(),
    /** Total matching entries in the local timeline. */
    val timelineTotal: UInt = 0u,
    /** Whether more timeline pages exist beyond `timeline`. */
    val timelineHasMore: Boolean = false,
    /** Timeline auto-purge retention in days (`0` = keep up to the entry cap regardless of age). */
    val timelineRetentionDays: UInt = 90u,
    /** Number of received-file transfer records kept in local history. */
    val receivedFileRecords: Int = 0,
    /** Number of temporary Continuity Camera / clipboard image files in the phone's cache. */
    val cachedPhotoFiles: Int = 0,
    /** Number of temporary MMS staging files in the phone's cache. */
    val cachedMmsFiles: Int = 0,
    /** Whether LocalSend LAN interop is enabled (off by default). */
    val localsendEnabled: Boolean = false,
    /** Whether staying connected over mobile data (LTE/5G) or relay when away from Wi-Fi is enabled. */
    val awayMode: Boolean = false,
    /** Nearby LocalSend devices discovered on the LAN. */
    val localsendPeers: List<LocalSendPeer> = emptyList(),
    /** IDs of paired PCs currently typing on this phone without mirroring. */
    val keyboardFromPc: Set<String> = emptySet(),
    /** Vibrant seed color (`0xRRGGBB`) extracted from currently playing phone or PC media artwork. */
    val activeMediaColor: UInt? = null,
) {
    fun device(id: String): Device? = devices.firstOrNull { it.id == id }

    /**
     * Whether a connected PC has "Phone as speaker" on: its deck state lists
     * this phone as an output. The PC stops sending audio during silence, so
     * this stays true across pauses where no frames arrive.
     */
    val pcSpeakerRelayOn: Boolean
        get() = devices.any { d ->
            d.online && d.deckState?.outputDevices?.any { it.id == PHONE_SPEAKER_OUTPUT } == true
        }

    fun nameOf(id: String): String? =
        device(id)?.name
            ?: discovered.firstOrNull { it.id == id }?.name
            ?: localsendPeers.firstOrNull { it.id == id }?.alias

    fun withDevices(
        paired: List<PairedDevice>,
        storageAllowed: Set<String> = emptySet(),
        micOff: Set<String> = emptySet(),
    ): CoreState {
        val prevById = devices.associateBy { it.id }
        return copy(
            devices = paired.map { p ->
                val prev = prevById[p.id]
                p.toDevice().copy(
                    deckLayout = prev?.deckLayout,
                    deckState = prev?.deckState,
                    storageEnabled = if (p.id in storageAllowed) true else prev?.storageEnabled ?: false,
                    micEnabled = p.id !in micOff,
                )
            }.sortedWith(deviceOrder),
        )
    }

    fun withWakeState(id: String, state: WakeState): CoreState =
        update(id) { it.copy(wakeState = state) }

    fun withMicEnabled(id: String, enabled: Boolean): CoreState =
        update(id) { it.copy(micEnabled = enabled) }.copy(
            micRequestFrom = if (!enabled && micRequestFrom == id) null else micRequestFrom,
        )

    fun withStorageEnabled(id: String, enabled: Boolean): CoreState =
        update(id) { it.copy(storageEnabled = enabled) }.copy(
            storageRequestedFrom = if (enabled && storageRequestedFrom == id) null else storageRequestedFrom,
        )

    val ringingFrom: String? get() = ringingPeer?.let { nameOf(it).orEmpty() }

    fun withDevicesOffline(): CoreState = copy(
        ringingPeer = null,
        ringingDevices = emptySet(),
        devices = devices.map { d ->
            if (d.online) d.copy(link = Link.Offline(System.currentTimeMillis() / 1000L), wakeState = WakeState.Idle) else d
        }.sortedWith(deviceOrder),
        keyboardFromPc = emptySet(),
        micRequestFrom = null,
    )

    /** Folds one core event into the state. */
    fun reduce(event: Event): CoreState = when (event) {
        is Event.DeviceAdded -> {
            val device = event.device.toDevice()
            copy(devices = (devices.filterNot { it.id == device.id } + device).sortedWith(deviceOrder))
        }
        is Event.DeviceRemoved -> copy(
            devices = devices.filterNot { it.id == event.id },
            sendQueue = sendQueue.filterNot { it.peerId == event.id },
            ringingPeer = ringingPeer?.takeUnless { it == event.id },
            ringingDevices = ringingDevices - event.id,
            storageRequestedFrom = if (storageRequestedFrom == event.id) null else storageRequestedFrom,
            webcamRequest = if (webcamRequest?.pcId == event.id) null else webcamRequest,
            micRequestFrom = if (micRequestFrom == event.id) null else micRequestFrom,
            keyboardFromPc = keyboardFromPc - event.id,
        )
        is Event.LinkChanged -> {
            val prev = device(event.id)
            if (prev == null) {
                this
            } else {
                val oldLink = prev.link
                val newLink = event.link
                if (oldLink is Link.Online && newLink is Link.Online &&
                    oldLink.relayed == newLink.relayed &&
                    oldLink.usb == newLink.usb &&
                    (oldLink.rttMs >= 1u) == (newLink.rttMs >= 1u) &&
                    kotlin.math.abs(oldLink.rttMs.toInt() - newLink.rttMs.toInt()) < 3 &&
                    prev.wakeState == WakeState.Idle
                ) {
                    this
                } else {
                    val onlineChanged = (oldLink is Link.Online) != (newLink is Link.Online)
                    update(event.id) {
                        it.copy(
                            link = newLink,
                            wakeState = if (newLink is Link.Online) WakeState.Idle else it.wakeState,
                        )
                    }.let { updated ->
                        updated.copy(
                            devices = if (onlineChanged) updated.devices.sortedWith(deviceOrder) else updated.devices,
                            keyboardFromPc = if (newLink is Link.Offline) updated.keyboardFromPc - event.id else updated.keyboardFromPc,
                            // A PC that went away can't take the microphone it asked for.
                            micRequestFrom = updated.micRequestFrom.takeUnless { newLink is Link.Offline && it == event.id },
                        )
                    }
                }
            }
        }
        is Event.PeerInfoChanged -> update(event.id) {
            it.copy(
                name = event.info.name,
                kind = event.info.kind,
                model = event.info.model,
                accent = event.info.accent ?: it.accent,
            )
        }
        is Event.PeerPowerChanged -> update(event.id) { it.copy(power = event.power) }
        is Event.WakeInfoChanged -> update(event.id) {
            it.copy(
                canWake = event.canWake,
                wakeState = if (!event.canWake) WakeState.Idle else it.wakeState,
            )
        }
        is Event.ReconnectChanged -> update(event.id) { it.copy(reconnect = event.on) }
        is Event.Battery -> update(event.id) { it.copy(battery = event.battery) }
        is Event.Capabilities -> update(event.id) { it.copy(features = event.features) }
        is Event.DeckLayout -> update(event.id) { it.copy(deckLayout = event.layout) }
        is Event.DeckState -> update(event.id) { it.copy(deckState = event.state) }
        is Event.Ring -> copy(ringingPeer = if (event.on) event.id else ringingPeer?.takeUnless { it == event.id })
        is Event.RemoteRing -> copy(ringingDevices = if (event.on) ringingDevices + event.id else ringingDevices - event.id)
        is Event.Discovered -> copy(discovered = discovered.filterNot { it.id == event.device.id } + event.device)
        is Event.DiscoveryExpired -> copy(discovered = discovered.filterNot { it.id == event.id })
        is Event.PairingCode -> copy(pairing = PairingState.Comparing(event.peer, event.code))
        is Event.Paired -> copy(pairing = PairingState.Paired(event.device.info.name))
        // Failures of attempts the UI isn't showing (a stranger's wrong code)
        // don't open the pairing screen.
        is Event.PairingFailed ->
            if (pairing == PairingState.Idle) this else copy(pairing = PairingState.Failed(event.failure))
        // PCs don't send notifications; nothing for the phone to show.
        is Event.NotificationsReset, is Event.NotificationPosted, is Event.NotificationRemoved -> this
        // The updater acts on it (Core); nothing shows.
        is Event.UpdateHint -> this
        // Android shows its own "copied" confirmation; Core refreshes clipboardHistory/timeline/LocalSend/sendQueue on change.
        is Event.ClipboardReceived,
        is Event.ClipboardHistoryChanged,
        is Event.TimelineChanged,
        is Event.LocalSendChanged,
        is Event.QueueChanged,
        is Event.QueueDroppedOnUnpair,
        -> this
        is Event.Transfer -> copy(
            transfers = withTransfer(event.transfer),
            sendQueue = withTransferQueueProgress(event.transfer),
        )
        // Shown in Android's media controls (see media/PcMedia).
        is Event.MediaChanged -> this
        is Event.StorageRequested -> copy(storageRequestedFrom = event.id)
        is Event.RemoteInputRequested -> copy(remoteInputRequested = !inputAccess && !elevated)
        // PCs only: phones announce their own photos, calls, contacts, texts, toggles, storage, screen and webcam.
        // Live PC audio frames are played directly by PcSpeakerPlayer in Core.
        is Event.PhotoAdded,
        is Event.PhotosChanged,
        is Event.CallChanged,
        is Event.CallLogChanged,
        is Event.ContactsChanged,
        is Event.SmsChanged,
        is Event.PhoneToggles,
        is Event.Mirroring,
        is Event.StorageChanged,
        is Event.Webcam,
        is Event.PcAudioFrame,
        is Event.PcMirrorConfig,
        is Event.PcMirrorFrame,
        is Event.PcMirrorEnded,
        -> this
    }

    private fun withTransfer(transfer: Transfer): List<Transfer> {
        val updated = if (transfers.any { it.id == transfer.id }) {
            transfers.map { if (it.id == transfer.id) transfer else it }
        } else {
            listOf(transfer) + transfers
        }
        val finished = updated.filter { it.isFinished() }.take(MAX_FINISHED_TRANSFERS).toSet()
        return updated.filter { !it.isFinished() || it in finished }
    }

    private fun withTransferQueueProgress(transfer: Transfer): List<QueueItemUi> {
        if (transfer.direction != app.nectarlink.core.TransferDirection.OUTGOING || sendQueue.isEmpty()) {
            return sendQueue
        }
        var changed = false
        val updated = sendQueue.map { item ->
            val targetId = item.activeTransferId ?: item.id
            if (targetId == transfer.id && item.kind.isFileBacked()) {
                val nextTotal = if (transfer.total > 0uL) {
                    transfer.total.toLong().coerceAtLeast(0L)
                } else {
                    item.totalBytes
                }
                val nextDone = transfer.done.toLong().coerceIn(0L, nextTotal.coerceAtLeast(0L))
                if (nextDone != item.bytesDone || nextTotal != item.totalBytes || item.activeTransferId != transfer.id) {
                    changed = true
                    item.copy(
                        totalBytes = nextTotal,
                        bytesDone = nextDone,
                        activeTransferId = transfer.id,
                    )
                } else {
                    item
                }
            } else {
                item
            }
        }
        return if (changed) updated else sendQueue
    }

    private fun update(id: String, change: (Device) -> Device): CoreState {
        var changed = false
        val next = devices.map {
            if (it.id == id) {
                val u = change(it)
                if (u != it) changed = true
                u
            } else {
                it
            }
        }
        return if (changed) copy(devices = next) else this
    }
}

internal fun Event.affectsCoreState(): Boolean = when (this) {
    is Event.UpdateHint,
    is Event.NotificationsReset,
    is Event.NotificationPosted,
    is Event.NotificationRemoved,
    is Event.ClipboardReceived,
    is Event.ClipboardHistoryChanged,
    is Event.TimelineChanged,
    is Event.LocalSendChanged,
    is Event.QueueChanged,
    is Event.QueueDroppedOnUnpair,
    is Event.MediaChanged,
    is Event.PhotoAdded,
    is Event.PhotosChanged,
    is Event.CallChanged,
    is Event.CallLogChanged,
    is Event.ContactsChanged,
    is Event.SmsChanged,
    is Event.PhoneToggles,
    is Event.Mirroring,
    is Event.StorageChanged,
    is Event.Webcam,
    is Event.PcAudioFrame,
    is Event.PcMirrorConfig,
    is Event.PcMirrorFrame,
    is Event.PcMirrorEnded,
    -> false
    else -> true
}

/** Finished transfers kept on Home. */
const val MAX_FINISHED_TRANSFERS = 5

fun Transfer.isFinished(): Boolean =
    status !is app.nectarlink.core.TransferStatus.Requested &&
        status !is app.nectarlink.core.TransferStatus.Running &&
        status !is app.nectarlink.core.TransferStatus.Waiting &&
        status !is app.nectarlink.core.TransferStatus.Paused

internal fun PairedDevice.toDevice() = Device(
    id = id,
    name = info.name,
    kind = info.kind,
    model = info.model,
    accent = info.accent,
    pairedAt = pairedAt,
    link = link,
    canWake = canWake,
    reconnect = reconnect,
)
