// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

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
import app.nectarlink.android.ui.MainActivity

/** A PC asking for this phone's camera as a webcam (`webcam.start`). */
data class WebcamRequest(
    val pcId: String,
    val width: Int = 1280,
    val height: Int = 720,
    val fps: Int = 30,
    val bitrate: Int = 4_000_000,
    val camera: String = "back",
)

/**
 * Posts a high-priority notification when a paired PC asks to use this
 * phone's camera (`webcam.start`). Tapping the notification opens
 * [MainActivity] on the Webcam screen for that PC so the camera only turns
 * on with the user's explicit action.
 */
object WebcamRequests {
    private const val CHANNEL = "webcam_requests"
    private const val TAG = "webcam_req"
    private const val REQUEST_TIMEOUT_MS = 2 * 60 * 1000L

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(
                CHANNEL,
                context.getString(R.string.channel_webcam_requests),
                NotificationManager.IMPORTANCE_HIGH,
            ),
        )
    }

    /** Shows the request notification; returns true if shown. */
    fun show(context: Context, request: WebcamRequest, pcName: String): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        createChannel(context)
        val openIntent = Intent(context, MainActivity::class.java).apply {
            putExtra("webcam_pc", request.pcId)
            putExtra("webcam_height", request.height)
            putExtra("webcam_camera", request.camera)
            putExtra("webcam_auto_start", true)
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val open = PendingIntent.getActivity(
            context,
            request.pcId.hashCode(),
            openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val title = context.getString(
            R.string.webcam_request_title,
            pcName.ifEmpty { context.getString(R.string.your_pc) },
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(context.getString(R.string.webcam_request_text))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setTimeoutAfter(REQUEST_TIMEOUT_MS)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching {
            NotificationManagerCompat.from(context).notify(TAG, request.pcId.hashCode(), notification)
        }.isSuccess
    }

    fun dismiss(context: Context, pcId: String) {
        NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode())
    }
}
