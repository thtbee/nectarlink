// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import app.nectarlink.android.webcam.WebcamRequest
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.Event
import app.nectarlink.core.Feature
import app.nectarlink.core.Link
import app.nectarlink.core.LocalSendPeer
import app.nectarlink.core.PairedDevice
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.Transfer

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
    val battery: app.nectarlink.core.Battery? = null,
    val power: PowerLevel = PowerLevel.NOT_APPLICABLE,
    val features: List<Feature> = emptyList(),
    val deckLayout: app.nectarlink.core.DeckLayout? = null,
    val deckState: app.nectarlink.core.DeckState? = null,
    /** Whether this PC is allowed to browse this phone's storage in File Explorer (off by default). */
    val storageEnabled: Boolean = false,
) {
    val online: Boolean get() = link is Link.Online

    /** Whether a feature works with this device now (the capability matrix). */
    fun has(feature: String): Boolean =
        features.any { it.id == feature && it.status is app.nectarlink.core.FeatureStatus.Available }
}

/** Everything the UI renders, folded from core events. */
data class CoreState(
    val status: CoreStatus = CoreStatus.Starting,
    /** Paired devices, in pairing order. */
    val devices: List<Device> = emptyList(),
    val discovered: List<DiscoveredDevice> = emptyList(),
    val pairing: PairingState = PairingState.Idle,
    /** Name of the device making this phone ring, or null. */
    val ringingFrom: String? = null,
    /** Whether the user let Nectarlink read notifications (to mirror them). */
    val notificationAccess: Boolean = false,
    /** Whether Android lets Nectarlink reach the local network (Android 17). */
    val localNetwork: Boolean = true,
    /** Whether the Elevated helper runs (wireless debugging). */
    val elevated: Boolean = false,
    /** Whether the user turned on control from the PC (the accessibility service). */
    val inputAccess: Boolean = false,
    /** Whether the user let Nectarlink read and send texts (for PCs). */
    val smsAccess: Boolean = false,
    /** Whether the user let Nectarlink follow and answer calls (to show them on PCs). */
    val callAccess: Boolean = false,
    /** Whether the user let Nectarlink read contacts (for PCs). */
    val contactsAccess: Boolean = false,
    /** Whether the user let Nectarlink see the phone's photos (to show them on PCs). */
    val photoAccess: Boolean = false,
    /** Whether Android 14+ "Select photos and videos" partial access is active. */
    val photoPartialAccess: Boolean = false,
    /** Whether Android lets Nectarlink run unrestricted in the background. */
    val backgroundUnrestricted: Boolean = true,
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
    /** File transfers, newest first (running ones and the latest finished). */
    val transfers: List<Transfer> = emptyList(),
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
    /** Nearby LocalSend devices discovered on the LAN. */
    val localsendPeers: List<LocalSendPeer> = emptyList(),
) {
    fun device(id: String): Device? = devices.firstOrNull { it.id == id }

    fun nameOf(id: String): String? =
        device(id)?.name
            ?: discovered.firstOrNull { it.id == id }?.name
            ?: localsendPeers.firstOrNull { it.id == id }?.alias

    fun withDevices(paired: List<PairedDevice>, storageAllowed: Set<String> = emptySet()): CoreState {
        val prevById = devices.associateBy { it.id }
        return copy(
            devices = paired.map { p ->
                val prev = prevById[p.id]
                p.toDevice().copy(
                    deckLayout = prev?.deckLayout,
                    deckState = prev?.deckState,
                    storageEnabled = if (p.id in storageAllowed) true else prev?.storageEnabled ?: false,
                )
            }.sortedBy { it.pairedAt },
        )
    }

    fun withWakeState(id: String, state: WakeState): CoreState =
        update(id) { it.copy(wakeState = state) }

    fun withStorageEnabled(id: String, enabled: Boolean): CoreState =
        update(id) { it.copy(storageEnabled = enabled) }.copy(
            storageRequestedFrom = if (enabled && storageRequestedFrom == id) null else storageRequestedFrom,
        )

    /** Folds one core event into the state. */
    fun reduce(event: Event): CoreState = when (event) {
        is Event.DeviceAdded -> {
            val device = event.device.toDevice()
            copy(devices = (devices.filterNot { it.id == device.id } + device).sortedBy { it.pairedAt })
        }
        is Event.DeviceRemoved -> copy(
            devices = devices.filterNot { it.id == event.id },
            ringingFrom = if (ringingFrom == nameOf(event.id)) null else ringingFrom,
            storageRequestedFrom = if (storageRequestedFrom == event.id) null else storageRequestedFrom,
            webcamRequest = if (webcamRequest?.pcId == event.id) null else webcamRequest,
        )
        is Event.LinkChanged -> update(event.id) {
            it.copy(
                link = event.link,
                wakeState = if (event.link is Link.Online) WakeState.Idle else it.wakeState,
            )
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
        is Event.Battery -> update(event.id) { it.copy(battery = event.battery) }
        is Event.Capabilities -> update(event.id) { it.copy(features = event.features) }
        is Event.DeckLayout -> update(event.id) { it.copy(deckLayout = event.layout) }
        is Event.DeckState -> update(event.id) { it.copy(deckState = event.state) }
        is Event.Ring -> copy(ringingFrom = if (event.on) nameOf(event.id) ?: "" else null)
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
        // Android shows its own "copied" confirmation; Core refreshes clipboardHistory/timeline/LocalSend on change.
        is Event.ClipboardReceived,
        is Event.ClipboardHistoryChanged,
        is Event.TimelineChanged,
        is Event.LocalSendChanged,
        -> this
        is Event.Transfer -> copy(transfers = withTransfer(event.transfer))
        // Shown in Android's media controls (see media/PcMedia).
        is Event.MediaChanged -> this
        is Event.StorageRequested -> copy(storageRequestedFrom = event.id)
        // PCs only: phones announce their own photos, calls, contacts, texts, toggles, storage, screen and webcam.
        is Event.PhotoAdded,
        is Event.PhotosChanged,
        is Event.CallChanged,
        is Event.CallLogChanged,
        is Event.ContactsChanged,
        is Event.SmsChanged,
        is Event.PhoneToggles,
        is Event.Mirroring,
        is Event.RemoteInputRequested,
        is Event.StorageChanged,
        is Event.Webcam,
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

    private fun update(id: String, change: (Device) -> Device): CoreState =
        copy(devices = devices.map { if (it.id == id) change(it) else it })
}

/** Finished transfers kept on Home. */
const val MAX_FINISHED_TRANSFERS = 5

fun Transfer.isFinished(): Boolean =
    status !is app.nectarlink.core.TransferStatus.Requested &&
        status !is app.nectarlink.core.TransferStatus.Running &&
        status !is app.nectarlink.core.TransferStatus.Waiting

internal fun PairedDevice.toDevice() = Device(
    id = id,
    name = info.name,
    kind = info.kind,
    model = info.model,
    accent = info.accent,
    pairedAt = pairedAt,
    link = link,
    canWake = canWake,
)
