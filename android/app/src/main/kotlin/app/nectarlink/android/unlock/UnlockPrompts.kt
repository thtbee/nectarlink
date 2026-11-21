// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import android.Manifest
import android.app.AlertDialog
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.widget.TextView
import androidx.annotation.RequiresApi
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.R
import java.security.Signature
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.Cipher
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.future.asDeferred
import kotlinx.coroutines.launch

/**
 * Asks the phone's owner to use a PC's unlock key (ADR 0009): a high-priority notification
 * that opens [UnlockPromptActivity], which shows Android's biometric prompt naming the PC
 * and its account. The core's thread waits for the authorized crypto operation.
 */
object UnlockPrompts {
    private const val CHANNEL = "unlock_requests"
    private const val TAG = "unlock_req"

    internal class Pending(
        val pcName: String,
        val userName: String,
        val enroll: Boolean,
        val crypto: BiometricPrompt.CryptoObject,
        val confirmed: CompletableFuture<String?>?,
    ) {
        val result = CompletableFuture<BiometricPrompt.CryptoObject?>()
        val showing = AtomicBoolean(false)
        @Volatile var cancel: (() -> Unit)? = null
    }

    private val pending = ConcurrentHashMap<Int, Pending>()
    private val ids = AtomicInteger()

    internal fun pending(id: Int): Pending? = pending[id]

    /**
     * Asks, and waits up to [timeoutMs]: the cipher, now authorized for one use, or null when
     * the owner declined or didn't answer.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    fun ask(context: Context, pcName: String, userName: String, enroll: Boolean, cipher: Cipher, timeoutMs: Long): Cipher? =
        ask(context, Pending(pcName, userName, enroll, BiometricPrompt.CryptoObject(cipher), null), timeoutMs)?.cipher

    @RequiresApi(Build.VERSION_CODES.R)
    fun askToSign(
        context: Context,
        pcName: String,
        userName: String,
        signature: Signature,
        timeoutMs: Long,
        confirmed: CompletableFuture<String?>? = null,
    ): Signature? = ask(
        context,
        Pending(pcName, userName, confirmed != null, BiometricPrompt.CryptoObject(signature), confirmed),
        timeoutMs,
    )?.signature

    private fun ask(context: Context, request: Pending, timeoutMs: Long): BiometricPrompt.CryptoObject? {
        val id = ids.incrementAndGet()
        pending[id] = request
        try {
            show(context, id, request)
            return runCatching { request.result.get(timeoutMs, TimeUnit.MILLISECONDS) }.getOrNull()
        } finally {
            pending.remove(id)
            if (request.result.complete(null)) {
                Handler(Looper.getMainLooper()).post { request.cancel?.invoke() }
            }
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

internal class EnrollmentComparison : ViewModel() {
    data class Presentation(val pcName: String, val userName: String, val fingerprint: Deferred<String?>)

    var presentation: Presentation? = null
}

/** Shows the biometric prompt for one [UnlockPrompts] request, then closes. */
class UnlockPromptActivity : FragmentActivity() {
    companion object {
        const val EXTRA_ID = "unlock_request"
    }

    private val comparison by lazy { ViewModelProvider(this)[EnrollmentComparison::class.java] }
    private var fingerprintDialog: AlertDialog? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        comparison.presentation?.let {
            showEnrollmentFingerprint(it)
            return
        }
        val request = UnlockPrompts.pending(intent.getIntExtra(EXTRA_ID, -1))
        // Gone (answered, timed out) or already on screen from the other way in.
        if (request == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            finish()
            return
        }
        val first = request.showing.compareAndSet(false, true)
        if ((!first && savedInstanceState == null) || request.result.isDone) {
            finish()
            return
        }
        prompt(request, first)
    }

    private fun prompt(request: UnlockPrompts.Pending, first: Boolean) {
        val title = if (request.enroll) {
            getString(R.string.unlock_enroll_title, request.pcName)
        } else {
            getString(R.string.unlock_request_title, request.pcName)
        }
        val description = when {
            request.confirmed != null -> getString(R.string.linux_unlock_enroll_text)
            request.enroll -> getString(R.string.unlock_enroll_text, request.pcName)
            else -> getString(R.string.unlock_prompt_text)
        }
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(getString(R.string.unlock_request_text, request.userName))
            .setDescription(description)
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .setConfirmationRequired(true)
            .setNegativeButtonText(getString(R.string.action_cancel))
            .build()
        val prompt = BiometricPrompt(
            this,
            ContextCompat.getMainExecutor(this),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    done(request, result.cryptoObject)
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    done(request, null)
                }
            },
        )
        request.cancel = {
            prompt.cancelAuthentication()
            finish()
        }
        if (request.result.isDone) {
            finish()
        } else if (first) {
            runCatching { prompt.authenticate(info, request.crypto) }.onFailure { done(request, null) }
        }
    }

    private fun done(request: UnlockPrompts.Pending, crypto: BiometricPrompt.CryptoObject?) {
        if (!request.result.complete(crypto)) return
        val confirmed = request.confirmed
        if (crypto?.signature != null && confirmed != null) {
            // A recreated activity can resume waiting without canceling the phone's confirmation.
            val presentation = EnrollmentComparison.Presentation(
                request.pcName, request.userName, confirmed.asDeferred(),
            )
            comparison.presentation = presentation
            showEnrollmentFingerprint(presentation)
        } else {
            finish()
        }
    }

    private fun showEnrollmentFingerprint(presentation: EnrollmentComparison.Presentation) {
        lifecycleScope.launch {
            val fingerprint = presentation.fingerprint.await()
            if (isFinishing || isDestroyed) return@launch
            if (fingerprint == null) {
                finish()
                return@launch
            }
            val message = getString(
                R.string.linux_unlock_confirm_fingerprint, presentation.pcName, presentation.userName, fingerprint,
            )
            fingerprintDialog = AlertDialog.Builder(this@UnlockPromptActivity)
                .setTitle(R.string.linux_unlock_key_fingerprint)
                .setMessage(message)
                .setPositiveButton(android.R.string.ok, null)
                .setOnDismissListener { finish() }
                .show()
            fingerprintDialog?.findViewById<TextView>(android.R.id.message)?.setTextIsSelectable(true)
        }
    }

    override fun onDestroy() {
        fingerprintDialog?.setOnDismissListener(null)
        fingerprintDialog?.dismiss()
        super.onDestroy()
    }
}
