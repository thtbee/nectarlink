// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.Event
import app.nectarlink.core.Feature
import app.nectarlink.core.Link
import app.nectarlink.core.PairedDevice
import app.nectarlink.core.PairingFailure
import app.nectarlink.core.PowerLevel
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

/** A paired device as the UI shows it. */
data class Device(
    val id: String,
    val name: String,
    val kind: app.nectarlink.core.DeviceKind,
    val model: String?,
    val pairedAt: Long,
    val link: Link,
    val battery: app.nectarlink.core.Battery? = null,
    val power: PowerLevel = PowerLevel.NOT_APPLICABLE,
    val features: List<Feature> = emptyList(),
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
    /** Whether the user let Nectarlink read and send texts (for PCs). */
    val smsAccess: Boolean = false,
    /** Whether the user let Nectarlink follow and answer calls (to show them on PCs). */
    val callAccess: Boolean = false,
    /** Whether the user let Nectarlink see the phone's photos (to show new ones on PCs). */
    val photoAccess: Boolean = false,
    /** Whether Android lets Nectarlink run unrestricted in the background. */
    val backgroundUnrestricted: Boolean = true,
    /** File transfers, newest first (running ones and the latest finished). */
    val transfers: List<Transfer> = emptyList(),
) {
    fun device(id: String): Device? = devices.firstOrNull { it.id == id }

    fun nameOf(id: String): String? =
        device(id)?.name ?: discovered.firstOrNull { it.id == id }?.name

    fun withDevices(paired: List<PairedDevice>): CoreState =
        copy(devices = paired.map { it.toDevice() }.sortedBy { it.pairedAt })

    /** Folds one core event into the state. */
    fun reduce(event: Event): CoreState = when (event) {
        is Event.DeviceAdded -> {
            val device = event.device.toDevice()
            copy(devices = (devices.filterNot { it.id == device.id } + device).sortedBy { it.pairedAt })
        }
        is Event.DeviceRemoved -> copy(
            devices = devices.filterNot { it.id == event.id },
            ringingFrom = if (ringingFrom == nameOf(event.id)) null else ringingFrom,
        )
        is Event.LinkChanged -> update(event.id) { it.copy(link = event.link) }
        is Event.PeerInfoChanged -> update(event.id) {
            it.copy(name = event.info.name, kind = event.info.kind, model = event.info.model)
        }
        is Event.PeerPowerChanged -> update(event.id) { it.copy(power = event.power) }
        is Event.Battery -> update(event.id) { it.copy(battery = event.battery) }
        is Event.Capabilities -> update(event.id) { it.copy(features = event.features) }
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
        // Android shows its own "copied" confirmation.
        is Event.ClipboardReceived -> this
        is Event.Transfer -> copy(transfers = withTransfer(event.transfer))
        // Shown in Android's media controls (see media/PcMedia).
        is Event.MediaChanged -> this
        // PCs only: phones announce their own photos, calls, texts and screen.
        is Event.PhotoAdded, is Event.CallChanged, is Event.SmsChanged, is Event.Mirroring -> this
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
    status !is app.nectarlink.core.TransferStatus.Running && status !is app.nectarlink.core.TransferStatus.Waiting

internal fun PairedDevice.toDevice() = Device(
    id = id,
    name = info.name,
    kind = info.kind,
    model = info.model,
    pairedAt = pairedAt,
    link = link,
)
