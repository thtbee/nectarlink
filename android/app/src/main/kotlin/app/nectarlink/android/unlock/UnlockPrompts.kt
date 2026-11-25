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
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.core.LinuxUnlockAttempt
import java.security.Signature
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.Cipher
import kotlin.random.Random
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.future.asDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withTimeoutOrNull

internal suspend fun <T> awaitUnlockResult(
    result: CompletableFuture<T?>,
    waitCancelled: suspend () -> Unit,
): T? = coroutineScope {
    val answer = result.asDeferred()
    val cancelled = async(start = CoroutineStart.UNDISPATCHED) { waitCancelled() }
    try {
        select {
            cancelled.onAwait { null }
            answer.onAwait { it }
        }
    } finally {
        cancelled.cancel()
    }
}

/**
 * Asks the phone's owner to use a PC's unlock key (ADR 0009): a high-priority notification
 * that opens [UnlockPromptActivity], which shows Android's biometric prompt naming the PC
 * and its account. The core's thread waits for the authorized crypto operation.
 */
object UnlockPrompts {
    private const val CHANNEL = "unlock_requests"
    private const val TAG = "unlock_req"

    @RequiresApi(Build.VERSION_CODES.R)
    internal class Pending(
        val pcName: String,
        val userName: String,
        val enroll: Boolean,
        val crypto: BiometricPrompt.CryptoObject,
        val confirmed: CompletableFuture<String?>?,
    ) {
        val result = CompletableFuture<BiometricPrompt.CryptoObject?>()
        val showing = AtomicBoolean(false)
        val cancellationSignal = CancellationSignal()
        @Volatile var close: (() -> Unit)? = null
    }

    private val pending = ConcurrentHashMap<Int, Pending>()
    // A task can remain in Recents across process restarts.
    private val ids = AtomicInteger(Random.nextInt(Int.MAX_VALUE / 2))

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
        attempt: LinuxUnlockAttempt?,
        confirmed: CompletableFuture<String?>? = null,
    ): Signature? = ask(
        context,
        Pending(pcName, userName, confirmed != null, BiometricPrompt.CryptoObject(signature), confirmed),
        timeoutMs,
        attempt,
    )?.signature

    @RequiresApi(Build.VERSION_CODES.R)
    private fun ask(
        context: Context,
        request: Pending,
        timeoutMs: Long,
        attempt: LinuxUnlockAttempt? = null,
    ): BiometricPrompt.CryptoObject? {
        if (attempt?.isCancelled() == true) return null
        val id = ids.incrementAndGet()
        pending[id] = request
        try {
            show(context, id, request)
            if (attempt == null) return runCatching { request.result.get(timeoutMs, TimeUnit.MILLISECONDS) }.getOrNull()
            return runBlocking {
                withTimeoutOrNull(timeoutMs) { awaitUnlockResult(request.result, attempt::waitCancelled) }
            }
        } finally {
            pending.remove(id)
            if (request.result.complete(null)) {
                Handler(Looper.getMainLooper()).post {
                    request.cancellationSignal.cancel()
                    request.close?.invoke()
                }
            }
            runCatching { NotificationManagerCompat.from(context).cancel(TAG, id) }
        }
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun show(context: Context, id: Int, request: Pending) {
        // Give each request its own task so an open comparison can't intercept the next prompt.
        val open = Intent(context, UnlockPromptActivity::class.java)
            .setData(Uri.fromParts("nectarlink-unlock", id.toString(), null))
            .putExtra(UnlockPromptActivity.EXTRA_ID, id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NEW_DOCUMENT or Intent.FLAG_ACTIVITY_NO_ANIMATION)
        if (request.confirmed == null) open.addFlags(Intent.FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS)
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

    @RequiresApi(Build.VERSION_CODES.R)
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
            // Not on Play, which allows full-screen intents only to calling and alarm apps.
            .apply { if (!BuildConfig.PLAY) setFullScreenIntent(tap, true) }
            .setAutoCancel(true)
            // Not a call: a request the user answers (calls are the only CATEGORY_CALL).
            .setCategory(NotificationCompat.CATEGORY_STATUS)
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
class UnlockPromptActivity : ComponentActivity() {
    companion object {
        const val EXTRA_ID = "unlock_request"
    }

    private val comparison by lazy { ViewModelProvider(this)[EnrollmentComparison::class.java] }
    private var fingerprintDialog: AlertDialog? = null
    private var cancelPrompt: (() -> Unit)? = null

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
        if (!request.showing.compareAndSet(false, true) || request.result.isDone) {
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
        val description = when {
            request.confirmed != null -> getString(R.string.linux_unlock_enroll_text)
            request.enroll -> getString(R.string.unlock_enroll_text, request.pcName)
            else -> getString(R.string.unlock_prompt_text)
        }
        val executor = ContextCompat.getMainExecutor(this)
        val prompt = BiometricPrompt.Builder(this)
            .setTitle(title)
            .setSubtitle(getString(R.string.unlock_request_text, request.userName))
            .setDescription(description)
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .setConfirmationRequired(true)
            .setNegativeButton(getString(R.string.action_cancel), executor) { _, _ -> done(request, null) }
            .build()
        request.close = ::finish
        if (request.result.isDone) {
            finish()
            return
        }
        cancelPrompt = {
            request.cancellationSignal.cancel()
            request.result.complete(null)
        }
        runCatching {
            prompt.authenticate(
                request.crypto,
                request.cancellationSignal,
                executor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        done(request, result.cryptoObject)
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        done(request, null)
                    }
                },
            )
        }.onFailure { done(request, null) }
    }

    @RequiresApi(Build.VERSION_CODES.R)
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
                R.string.linux_unlock_confirm_fingerprint, presentation.pcName, presentation.userName,
                fingerprint.chunked(8).joinToString(" "),
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

    override fun onStop() {
        super.onStop()
        if (comparison.presentation == null && !isFinishing) {
            cancelPrompt?.invoke()
            finish()
        }
    }

    override fun onDestroy() {
        fingerprintDialog?.setOnDismissListener(null)
        fingerprintDialog?.dismiss()
        super.onDestroy()
    }
}
