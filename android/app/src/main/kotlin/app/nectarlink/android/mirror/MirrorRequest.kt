// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.Manifest
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R

/** A PC asking for the screen, with what it wants. */
data class MirrorRequest(val pcId: String, val maxSize: Int, val fps: Int, val bitrate: Int) {
    fun toIntent(intent: Intent): Intent = intent
        .putExtra("pc", pcId)
        .putExtra("maxSize", maxSize)
        .putExtra("fps", fps)
        .putExtra("bitrate", bitrate)

    companion object {
        fun of(intent: Intent): MirrorRequest? = intent.getStringExtra("pc")?.let {
            MirrorRequest(it, intent.getIntExtra("maxSize", 1920), intent.getIntExtra("fps", 60), intent.getIntExtra("bitrate", 8_000_000))
        }
    }
}

/**
 * Asking the user: Android shows its screen capture prompt only from a
 * screen the user is on, so a PC's request waits in a notification; tapping
 * it opens [MirrorConsentActivity], which shows the prompt.
 */
object MirrorRequests {
    private const val CHANNEL = "mirror_requests"
    private const val TAG = "mirror"

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_mirror_requests), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** Shows the request; false if notifications are off for the app. */
    fun show(context: Context, request: MirrorRequest, pcName: String): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return false
        }
        createChannel(context)
        val open = PendingIntent.getActivity(
            context,
            request.pcId.hashCode(),
            request.toIntent(Intent(context, MirrorConsentActivity::class.java)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.mirror_request_title, pcName.ifEmpty { context.getString(R.string.your_pc) }))
            .setContentText(context.getString(R.string.mirror_request_text))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setTimeoutAfter(REQUEST_TIMEOUT_MS)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(TAG, request.pcId.hashCode(), notification) }.isSuccess
    }

    fun dismiss(context: Context, pcId: String) {
        NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode())
    }

    /** A request nobody answered goes away after a while. */
    private const val REQUEST_TIMEOUT_MS = 2 * 60 * 1000L
}

/** Shows Android's screen capture prompt for a PC's request, then starts sharing. */
class MirrorConsentActivity : ComponentActivity() {
    private var request: MirrorRequest? = null

    private val prompt = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val data = result.data
        val asked = request
        if (result.resultCode == Activity.RESULT_OK && data != null && asked != null) {
            MirrorService.start(this, asked, result.resultCode, data)
        }
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        request = MirrorRequest.of(intent)
        val asked = request ?: return finish()
        MirrorRequests.dismiss(this, asked.pcId)
        if (savedInstanceState == null) {
            prompt.launch(getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
        }
    }
}
