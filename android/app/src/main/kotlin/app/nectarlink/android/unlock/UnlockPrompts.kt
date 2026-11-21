// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.Cipher

/**
 * Asks the phone's owner to use a PC's unlock key (ADR 0009): a high-priority notification
 * that opens [UnlockPromptActivity], which shows Android's biometric prompt naming the PC
 * and the Windows user. The caller (the core's thread) waits for the answer.
 */
object UnlockPrompts {
    private const val CHANNEL = "unlock_requests"
    private const val TAG = "unlock_req"

    internal class Pending(
        val pcName: String,
        val userName: String,
        val enroll: Boolean,
        val cipher: Cipher,
    ) {
        val result = CompletableFuture<Cipher?>()
        val showing = AtomicBoolean(false)
        @Volatile var cancel: CancellationSignal? = null
    }

    private val pending = ConcurrentHashMap<Int, Pending>()
    private val ids = AtomicInteger()

    internal fun pending(id: Int): Pending? = pending[id]

    /**
     * Asks, and waits up to [timeoutMs]: the cipher, now authorized for one use, or null when
     * the owner declined or didn't answer.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    fun ask(context: Context, pcName: String, userName: String, enroll: Boolean, cipher: Cipher, timeoutMs: Long): Cipher? {
        val id = ids.incrementAndGet()
        val request = Pending(pcName, userName, enroll, cipher)
        pending[id] = request
        try {
            show(context, id, request)
            return runCatching { request.result.get(timeoutMs, TimeUnit.MILLISECONDS) }.getOrNull()
        } finally {
            pending.remove(id)
            request.cancel?.cancel()
            runCatching { NotificationManagerCompat.from(context).cancel(TAG, id) }
        }
    }

    private fun show(context: Context, id: Int, request: Pending) {
        val open = Intent(context, UnlockPromptActivity::class.java)
            .putExtra(UnlockPromptActivity.EXTRA_ID, id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
        // Straight to the prompt when Android lets this app open a screen now (the app is in
        // front, may display over other apps, or is the PC's companion app); the notification
        // otherwise. Android drops a blocked start silently, so when it may well have worked the
        // notification waits a moment and only comes if the prompt didn't show.
        runCatching { context.startActivity(open) }
        if (Settings.canDrawOverlays(context)) {
            Handler(Looper.getMainLooper()).postDelayed({
                if (pending[id] === request && !request.showing.get()) notify(context, id, request, open)
            }, 1_500L)
        } else {
            notify(context, id, request, open)
        }
    }

    private fun notify(context: Context, id: Int, request: Pending, open: Intent) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_unlock_requests), NotificationManager.IMPORTANCE_HIGH),
        )
        val tap = PendingIntent.getActivity(context, id, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val title = if (request.enroll) {
            context.getString(R.string.unlock_enroll_title, request.pcName)
        } else {
            context.getString(R.string.unlock_request_title, request.pcName)
        }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(context.getString(R.string.unlock_request_text, request.userName))
            .setContentIntent(tap)
            .setFullScreenIntent(tap, true)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(TAG, id, notification) }
    }
}

/** Shows the biometric prompt for one [UnlockPrompts] request, then closes. */
class UnlockPromptActivity : ComponentActivity() {
    companion object {
        const val EXTRA_ID = "unlock_request"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val request = UnlockPrompts.pending(intent.getIntExtra(EXTRA_ID, -1))
        // Gone (answered, timed out) or already on screen from the other way in.
        if (request == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.R || !request.showing.compareAndSet(false, true)) {
            finish()
            return
        }
        prompt(request)
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun prompt(request: UnlockPrompts.Pending) {
        val title = if (request.enroll) {
            getString(R.string.unlock_enroll_title, request.pcName)
        } else {
            getString(R.string.unlock_request_title, request.pcName)
        }
        val description = if (request.enroll) {
            getString(R.string.unlock_enroll_text, request.pcName)
        } else {
            getString(R.string.unlock_prompt_text)
        }
        val executor = ContextCompat.getMainExecutor(this)
        val prompt = BiometricPrompt.Builder(this)
            .setTitle(title)
            .setSubtitle(getString(R.string.unlock_request_text, request.userName))
            .setDescription(description)
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            // Face unlock must still be confirmed with a tap: nothing unlocks by just looking.
            .setConfirmationRequired(true)
            .setNegativeButton(getString(R.string.action_cancel), executor) { _, _ -> done(request, null) }
            .build()
        val cancel = CancellationSignal()
        request.cancel = cancel
        prompt.authenticate(
            BiometricPrompt.CryptoObject(request.cipher),
            cancel,
            executor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    done(request, result.cryptoObject?.cipher)
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    done(request, null)
                }
            },
        )
    }

    private fun done(request: UnlockPrompts.Pending, cipher: Cipher?) {
        request.result.complete(cipher)
        finish()
    }
}
