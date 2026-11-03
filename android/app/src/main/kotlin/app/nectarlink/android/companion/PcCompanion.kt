// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.companion

import android.app.Activity
import android.bluetooth.le.ScanFilter
import android.companion.AssociationInfo
import android.companion.AssociationRequest
import android.companion.BluetoothLeDeviceFilter
import android.companion.CompanionDeviceManager
import android.content.Context
import android.content.IntentSender
import android.os.Build
import java.util.concurrent.Executor

/**
 * Companion Device Manager (CDM) integration for waking NectarLink when a
 * paired PC's BLE proximity beacon (`NCTR\x01` + 8-byte device ID prefix) is
 * nearby, and granting OS-level background execution / foreground-service start
 * exemptions while associated.
 */
object PcCompanion {
    const val MANUFACTURER_ID: Int = 0xFFFF

    val BEACON_MAGIC: ByteArray = byteArrayOf(
        'N'.code.toByte(),
        'C'.code.toByte(),
        'T'.code.toByte(),
        'R'.code.toByte(),
        1,
    )

    /** z-base-32, as device IDs are written (`core/nectarlink-protocol/src/id.rs`). */
    private const val ZBASE32 = "ybndrfg8ejkmcpqxot1uwisza345h769"

    /**
     * The bytes of a z-base-32 string, most significant bit first, or null if it
     * has a character outside the alphabet. Trailing bits that don't make a whole
     * byte are dropped, as the encoder pads with them.
     */
    fun decodeZBase32(text: String): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        var buffer = 0
        var bits = 0
        for (c in text.trim().lowercase()) {
            val value = ZBASE32.indexOf(c)
            if (value < 0) return null
            buffer = (buffer shl 5) or value
            bits += 5
            if (bits >= 8) {
                bits -= 8
                out.write((buffer shr bits) and 0xFF)
            }
        }
        return out.toByteArray()
    }

    /**
     * Builds the manufacturer-specific BLE beacon payload a PC broadcasts
     * (`desktop/app/src/win/ble_beacon.rs`): the 5-byte `NCTR\x01` magic followed
     * by the first 8 bytes of the PC's 32-byte device ID (13 bytes in all), or
     * just the magic when [deviceId] isn't a valid device ID.
     */
    fun buildBeaconPayload(deviceId: String?): ByteArray {
        val id = deviceId?.let(::decodeZBase32)
        if (id == null || id.size != 32) {
            return BEACON_MAGIC.copyOf()
        }
        return BEACON_MAGIC + id.copyOfRange(0, 8)
    }

    /**
     * Builds the scan filter mask matching [buildBeaconPayload]: 13 `0xFF`
     * bytes when a specific 8-byte device ID prefix is present, or 5 `0xFF`
     * bytes to match any NectarLink PC beacon.
     */
    fun buildBeaconMask(hasDeviceId: Boolean): ByteArray =
        ByteArray(if (hasDeviceId) 13 else 5) { 0xFF.toByte() }

    fun isAssociated(context: Context): Boolean = runCatching {
        val cdm = context.getSystemService(CompanionDeviceManager::class.java) ?: return false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            cdm.myAssociations.isNotEmpty()
        } else {
            @Suppress("DEPRECATION")
            cdm.associations.isNotEmpty()
        }
    }.getOrDefault(false)

    fun startObservingPresence(context: Context) {
        runCatching {
            val cdm = context.getSystemService(CompanionDeviceManager::class.java) ?: return
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                for (info in cdm.myAssociations) {
                    runCatching {
                        val addr = info.deviceMacAddress?.toString()
                        if (!addr.isNullOrBlank()) {
                            @Suppress("DEPRECATION")
                            cdm.startObservingDevicePresence(addr)
                        }
                    }
                }
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                @Suppress("DEPRECATION")
                for (addr in cdm.associations) {
                    runCatching {
                        cdm.startObservingDevicePresence(addr)
                    }
                }
            }
        }
    }

    /** Removes CompanionDeviceManager associations for a removed PC (or all when no PCs remain paired). */
    fun disassociateForRemovedPc(context: Context, pcName: String?, noDevicesRemaining: Boolean) {
        runCatching {
            val cdm = context.getSystemService(CompanionDeviceManager::class.java) ?: return
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                for (info in cdm.myAssociations) {
                    val matchesName = !pcName.isNullOrBlank() && info.displayName?.toString() == pcName
                    if (noDevicesRemaining || matchesName) {
                        val addr = info.deviceMacAddress?.toString()
                        if (!addr.isNullOrBlank()) {
                            @Suppress("DEPRECATION")
                            runCatching { cdm.stopObservingDevicePresence(addr) }
                        }
                        runCatching { cdm.disassociate(info.id) }
                    }
                }
            } else if (noDevicesRemaining) {
                @Suppress("DEPRECATION")
                for (addr in cdm.associations) {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                        runCatching { cdm.stopObservingDevicePresence(addr) }
                    }
                    runCatching { cdm.disassociate(addr) }
                }
            }
        }
    }

    fun requestAssociation(
        activity: Activity,
        pcName: String,
        pcDeviceId: String? = null,
        onResult: (Boolean) -> Unit = {},
        onLauncherFallback: (IntentSender) -> Unit,
    ) {
        runCatching {
            val cdm = activity.getSystemService(CompanionDeviceManager::class.java) ?: run {
                onResult(false)
                return
            }
            val payload = buildBeaconPayload(pcDeviceId)
            val mask = buildBeaconMask(payload.size == 13)
            val scanFilter = ScanFilter.Builder()
                .setManufacturerData(MANUFACTURER_ID, payload, mask)
                .build()
            val leFilter = BluetoothLeDeviceFilter.Builder()
                .setScanFilter(scanFilter)
                .build()
            val requestBuilder = AssociationRequest.Builder()
                .addDeviceFilter(leFilter)
                .setSingleDevice(false)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && pcName.isNotBlank()) {
                runCatching { requestBuilder.setDisplayName(pcName) }
            }
            val request = requestBuilder.build()
            val mainExecutor = Executor { runnable -> activity.runOnUiThread(runnable) }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                cdm.associate(
                    request,
                    mainExecutor,
                    object : CompanionDeviceManager.Callback() {
                        override fun onAssociationPending(intentSender: IntentSender) {
                            onLauncherFallback(intentSender)
                        }

                        override fun onAssociationCreated(associationInfo: AssociationInfo) {
                            startObservingPresence(activity)
                            onResult(true)
                        }

                        override fun onFailure(error: CharSequence?) {
                            onResult(false)
                        }
                    },
                )
            } else {
                cdm.associate(
                    request,
                    object : CompanionDeviceManager.Callback() {
                        @Deprecated("Deprecated in Java")
                        override fun onDeviceFound(chooserLauncher: IntentSender) {
                            onLauncherFallback(chooserLauncher)
                        }

                        override fun onFailure(error: CharSequence?) {
                            onResult(false)
                        }
                    },
                    null,
                )
            }
        }.onFailure {
            onResult(false)
        }
    }
}
