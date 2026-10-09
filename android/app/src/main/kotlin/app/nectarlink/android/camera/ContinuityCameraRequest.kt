// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.camera

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** A paired PC asking this phone to capture a photo or scan a document (`camera.capture`). */
data class ContinuityCameraRequest(
    val pcId: String,
    val requestId: String,
    /** `"photo"` or `"scan"`. */
    val mode: String,
    val pcName: String,
)

/**
 * Coordinates Continuity Camera requests from paired PCs: launches
 * [ContinuityCameraActivity] directly when allowed, posts a high-priority
 * heads-up notification with full-screen intent, and propagates cancellations
 * from the PC so any open capture screen closes cleanly.
 */
object ContinuityCameraRequests {
    private const val CHANNEL = "continuity_camera_requests"
    private const val TAG = "continuity_camera_req"
    private const val REQUEST_TIMEOUT_MS = 2 * 60 * 1000L

    const val EXTRA_PC_ID = "continuity_pc_id"
    const val EXTRA_REQUEST_ID = "continuity_request_id"
    const val EXTRA_MODE = "continuity_mode"
    const val EXTRA_PC_NAME = "continuity_pc_name"

    private val _active = MutableStateFlow<ContinuityCameraRequest?>(null)
    val active: StateFlow<ContinuityCameraRequest?> = _active.asStateFlow()

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(
                CHANNEL,
                context.getString(R.string.channel_camera_requests),
                NotificationManager.IMPORTANCE_HIGH,
            ),
        )
    }

    /**
     * Handles an incoming `camera.capture` request from `pcId`: updates active
     * state, starts [ContinuityCameraActivity] when foregrounded, and posts a
     * heads-up notification with full-screen intent.
     */
    fun show(
        context: Context,
        pcId: String,
        requestId: String,
        mode: String,
        pcName: String,
    ): Boolean {
        val cleanMode = if (mode == "scan") "scan" else "photo"
        val fromName = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val req = ContinuityCameraRequest(
            pcId = pcId,
            requestId = requestId,
            mode = cleanMode,
            pcName = fromName,
        )
        _active.value = req

        val openIntent = Intent(context, ContinuityCameraActivity::class.java).apply {
            putExtra(EXTRA_PC_ID, pcId)
            putExtra(EXTRA_REQUEST_ID, requestId)
            putExtra(EXTRA_MODE, cleanMode)
            putExtra(EXTRA_PC_NAME, fromName)
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or
                Intent.FLAG_ACTIVITY_SINGLE_TOP or
                Intent.FLAG_ACTIVITY_CLEAR_TOP
        }

        // Launch directly if the app is currently foregrounded or allowed to start activities.
        runCatching { context.startActivity(openIntent) }

        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            createChannel(context)
            val openPending = PendingIntent.getActivity(
                context,
                requestId.hashCode(),
                openIntent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val title = if (cleanMode == "scan") {
                context.getString(R.string.continuity_camera_request_scan_title, fromName)
            } else {
                context.getString(R.string.continuity_camera_request_photo_title, fromName)
            }
            val text = if (cleanMode == "scan") {
                context.getString(R.string.continuity_camera_request_scan_text)
            } else {
                context.getString(R.string.continuity_camera_request_photo_text)
            }
            val notification = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(text)
                .setContentIntent(openPending)
                .setFullScreenIntent(openPending, true)
                .setAutoCancel(true)
                .setTimeoutAfter(REQUEST_TIMEOUT_MS)
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .build()
            runCatching {
                NotificationManagerCompat.from(context).notify(TAG, pcId.hashCode(), notification)
            }
        }
        return true
    }

    /** Called when the PC cancels an active `camera.capture` request. */
    fun cancelFromPc(context: Context, pcId: String, requestId: String) {
        NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode())
        _active.update { current ->
            if (current != null && current.pcId == pcId && (requestId.isEmpty() || current.requestId == requestId)) {
                null
            } else {
                current
            }
        }
    }

    /** Dismisses the heads-up notification once the activity opens or completes. */
    fun dismissNotification(context: Context, pcId: String) {
        NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode())
    }

    /** Clears active request tracking when the phone finishes or cancels capture. */
    fun clearActive(pcId: String, requestId: String) {
        _active.update { current ->
            if (current != null && current.pcId == pcId && current.requestId == requestId) null else current
        }
    }
}
