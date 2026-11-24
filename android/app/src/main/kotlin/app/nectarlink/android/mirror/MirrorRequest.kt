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
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R

/** A PC asking for the screen (and its sound), or only the sound, with what it wants. */
data class MirrorRequest(
    val pcId: String,
    val maxSize: Int,
    val fps: Int,
    val bitrate: Int,
    val audio: Boolean,
    val stayAwake: Boolean = false,
    val screenOff: Boolean = false,
    /** Only the sound: the PC plays it without a window. */
    val soundOnly: Boolean = false,
) {
    fun toIntent(intent: Intent): Intent = intent
        .putExtra("pc", pcId)
        .putExtra("maxSize", maxSize)
        .putExtra("fps", fps)
        .putExtra("bitrate", bitrate)
        .putExtra("audio", audio)
        .putExtra("stayAwake", stayAwake)
        .putExtra("screenOff", screenOff)
        .putExtra("soundOnly", soundOnly)

    companion object {
        fun of(intent: Intent): MirrorRequest? = intent.getStringExtra("pc")?.let {
            MirrorRequest(
                it,
                intent.getIntExtra("maxSize", 1920),
                intent.getIntExtra("fps", 60),
                intent.getIntExtra("bitrate", 8_000_000),
                intent.getBooleanExtra("audio", false),
                intent.getBooleanExtra("stayAwake", false),
                intent.getBooleanExtra("screenOff", false),
                intent.getBooleanExtra("soundOnly", false),
            )
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
        val activityIntent = request.toIntent(Intent(context, MirrorConsentActivity::class.java))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val open = PendingIntent.getActivity(
            context,
            request.pcId.hashCode(),
            activityIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(
                context.getString(
                    if (request.soundOnly) R.string.mirror_request_title_sound else R.string.mirror_request_title,
                    pcName.ifEmpty { context.getString(R.string.your_pc) },
                ),
            )
            .setContentText(
                context.getString(if (request.soundOnly) R.string.mirror_request_text_sound else R.string.mirror_request_text),
            )
            .setContentIntent(open)
            .setAutoCancel(true)
            .setTimeoutAfter(REQUEST_TIMEOUT_MS)
            // Not a call: a request the user answers (calls are the only CATEGORY_CALL).
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        val posted = runCatching { NotificationManagerCompat.from(context).notify(TAG, request.pcId.hashCode(), notification) }.isSuccess
        val am = context.getSystemService(android.app.ActivityManager::class.java)
        val inForeground = am?.runningAppProcesses?.any {
            it.pid == android.os.Process.myPid() &&
                it.importance == android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
        } == true
        if (inForeground) {
            runCatching { context.startActivity(activityIntent) }
        }
        return posted
    }

    fun dismiss(context: Context, pcId: String) {
        NotificationManagerCompat.from(context).cancel(TAG, pcId.hashCode())
    }

    /** A request nobody answered goes away after a while. */
    private const val REQUEST_TIMEOUT_MS = 2 * 60 * 1000L
}

/**
 * Shows Android's screen capture prompt for a PC's request, then starts
 * sharing. A PC that wants the sound too first gets the permission asked
 * for (once; without it, the screen is shared alone).
 */
class MirrorConsentActivity : ComponentActivity() {
    private var request: MirrorRequest? = null

    private val sound = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        // Only the sound was asked for, and it can't be captured without this.
        if (!granted && request?.soundOnly == true) finish() else askForScreen()
    }

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
        if (savedInstanceState != null) return
        val wantsSound = asked.audio && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED
        if (wantsSound) sound.launch(Manifest.permission.RECORD_AUDIO) else askForScreen()
    }

    private fun askForScreen() {
        prompt.launch(getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
    }
}
