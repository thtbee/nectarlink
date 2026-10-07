// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.recorder

import android.content.Context
import app.nectarlink.core.RecordingMarker
import app.nectarlink.core.Transfer
import app.nectarlink.core.TransferStatus
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

enum class DeliveryState {
    Idle,
    Waiting,
    Sending,
    Sent,
    Denied,
    Failed,
}

data class SavedRecording(
    val id: String,
    val fileName: String,
    val createdAtMs: Long,
    val durationMs: Long,
    val sizeBytes: Long,
    val markers: List<RecordingMarker>,
    val targetPcId: String?,
    val transferId: String? = null,
    val delivery: DeliveryState = DeliveryState.Idle,
) {
    fun file(dir: File): File = dir.resolve(fileName)
}

/**
 * Keeps voice recordings in `filesDir/recordings` with their markers and
 * delivery status to the chosen PC.
 */
class RecordingsStore(context: Context) {
    private val dir: File = context.applicationContext.filesDir.resolve("recordings").apply { mkdirs() }
    private val indexFile: File = dir.resolve("recordings.json")
    private val lock = Any()
    private val _recordings = MutableStateFlow<List<SavedRecording>>(emptyList())
    val recordings: StateFlow<List<SavedRecording>> = _recordings.asStateFlow()

    init {
        synchronized(lock) {
            _recordings.value = loadLocked()
        }
    }

    fun directory(): File = dir

    /** Picks a unique `Recording YYYY-MM-DD HH.mm.m4a` (or `(2)`, etc.) in `filesDir/recordings`. */
    fun newRecordingFile(nowMs: Long = System.currentTimeMillis()): File {
        synchronized(lock) {
            dir.mkdirs()
            val stem = "Recording " + SimpleDateFormat("yyyy-MM-dd HH.mm", Locale.US).format(Date(nowMs))
            val first = dir.resolve("$stem.m4a")
            if (!first.exists()) return first
            var n = 2
            while (true) {
                val candidate = dir.resolve("$stem ($n).m4a")
                if (!candidate.exists()) return candidate
                n++
            }
        }
    }

    fun get(id: String): SavedRecording? = _recordings.value.firstOrNull { it.id == id }

    fun fileFor(recording: SavedRecording): File = recording.file(dir)

    fun add(
        file: File,
        durationMs: Long,
        markers: List<RecordingMarker>,
        targetPcId: String?,
        waiting: Boolean,
    ): SavedRecording {
        synchronized(lock) {
            val rec = SavedRecording(
                id = "rec-${System.currentTimeMillis()}-${file.name.hashCode().toUInt()}",
                fileName = file.name,
                createdAtMs = System.currentTimeMillis(),
                durationMs = durationMs.coerceAtLeast(0L),
                sizeBytes = file.length().coerceAtLeast(0L),
                markers = markers,
                targetPcId = targetPcId,
                transferId = null,
                delivery = when {
                    targetPcId == null -> DeliveryState.Idle
                    waiting -> DeliveryState.Waiting
                    else -> DeliveryState.Sending
                },
            )
            val updated = listOf(rec) + _recordings.value.filterNot { it.fileName == file.name }
            saveLocked(updated)
            _recordings.value = updated
            return rec
        }
    }

    fun delete(id: String) {
        synchronized(lock) {
            val target = _recordings.value.firstOrNull { it.id == id }
            if (target != null) {
                runCatching { target.file(dir).delete() }
            }
            val updated = _recordings.value.filterNot { it.id == id }
            saveLocked(updated)
            _recordings.value = updated
        }
    }

    fun updateDelivery(
        id: String,
        pcId: String?,
        transferId: String?,
        delivery: DeliveryState,
    ) {
        synchronized(lock) {
            val updated = _recordings.value.map { rec ->
                if (rec.id == id) {
                    rec.copy(targetPcId = pcId, transferId = transferId, delivery = delivery)
                } else {
                    rec
                }
            }
            saveLocked(updated)
            _recordings.value = updated
        }
    }

    fun onTransferEvent(transfer: Transfer, pcOnline: Boolean) {
        synchronized(lock) {
            val current = _recordings.value
            if (current.none { it.transferId == transfer.id }) return
            val updated = current.map { rec ->
                if (rec.transferId != transfer.id) return@map rec
                val nextDelivery = when (val status = transfer.status) {
                    is TransferStatus.Waiting -> DeliveryState.Waiting
                    is TransferStatus.Running -> DeliveryState.Sending
                    is TransferStatus.Done -> DeliveryState.Sent
                    is TransferStatus.Cancelled -> DeliveryState.Idle
                    is TransferStatus.Failed -> when (status.reason) {
                        "denied" -> DeliveryState.Denied
                        "unreachable" -> if (!pcOnline) DeliveryState.Waiting else DeliveryState.Failed
                        else -> if (!pcOnline) DeliveryState.Waiting else DeliveryState.Failed
                    }
                }
                rec.copy(delivery = nextDelivery)
            }
            saveLocked(updated)
            _recordings.value = updated
        }
    }

    /** Recordings queued for `pcId` while it was offline. */
    fun waitingForPc(pcId: String): List<SavedRecording> =
        _recordings.value.filter { it.targetPcId == pcId && it.delivery == DeliveryState.Waiting }

    private fun loadLocked(): List<SavedRecording> {
        if (!indexFile.exists()) return emptyList()
        return runCatching {
            val arr = JSONArray(indexFile.readText())
            buildList {
                for (i in 0 until arr.length()) {
                    val obj = arr.optJSONObject(i) ?: continue
                    val fileName = obj.optString("fileName", "")
                    if (fileName.isEmpty()) continue
                    val file = dir.resolve(fileName)
                    if (!file.exists()) continue
                    val markersArr = obj.optJSONArray("markers") ?: JSONArray()
                    val markers = buildList {
                        for (j in 0 until markersArr.length()) {
                            val m = markersArr.optJSONObject(j) ?: continue
                            val atMs = m.optLong("atMs", -1L)
                            if (atMs < 0L) continue
                            val label = m.optString("label", "").trim().ifEmpty { null }
                            add(RecordingMarker(atMs = atMs.toULong(), label = label))
                        }
                    }
                    val delivery = runCatching {
                        DeliveryState.valueOf(obj.optString("delivery", DeliveryState.Idle.name))
                    }.getOrDefault(DeliveryState.Idle)
                    add(
                        SavedRecording(
                            id = obj.optString("id", "rec-$i"),
                            fileName = fileName,
                            createdAtMs = obj.optLong("createdAtMs", file.lastModified()),
                            durationMs = obj.optLong("durationMs", 0L),
                            sizeBytes = file.length(),
                            markers = markers,
                            targetPcId = obj.optString("targetPcId", "").ifEmpty { null },
                            transferId = obj.optString("transferId", "").ifEmpty { null },
                            delivery = delivery,
                        ),
                    )
                }
            }
        }.getOrDefault(emptyList())
    }

    private fun saveLocked(items: List<SavedRecording>) {
        runCatching {
            dir.mkdirs()
            val arr = JSONArray()
            for (rec in items) {
                val markersArr = JSONArray()
                for (m in rec.markers) {
                    markersArr.put(
                        JSONObject().apply {
                            put("atMs", m.atMs.toLong())
                            if (m.label != null) put("label", m.label)
                        },
                    )
                }
                arr.put(
                    JSONObject().apply {
                        put("id", rec.id)
                        put("fileName", rec.fileName)
                        put("createdAtMs", rec.createdAtMs)
                        put("durationMs", rec.durationMs)
                        put("sizeBytes", rec.sizeBytes)
                        put("markers", markersArr)
                        if (rec.targetPcId != null) put("targetPcId", rec.targetPcId)
                        if (rec.transferId != null) put("transferId", rec.transferId)
                        put("delivery", rec.delivery.name)
                    },
                )
            }
            val tmp = dir.resolve("recordings.json.tmp")
            tmp.writeText(arr.toString())
            if (!tmp.renameTo(indexFile)) {
                indexFile.writeText(arr.toString())
                tmp.delete()
            }
        }
    }
}
