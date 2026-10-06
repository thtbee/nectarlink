// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.app.ActivityOptions
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.core.CallCommand
import app.nectarlink.core.SmsMessage
import app.nectarlink.core.SmsPartData
import app.nectarlink.core.SmsThread
import app.nectarlink.core.FileToSend
import app.nectarlink.core.MediaAction
import app.nectarlink.core.NotificationFailure
import app.nectarlink.core.Platform

/**
 * What the core asks of the phone: ring it, and act on its notifications
 * for a PC (dismiss, reply, run an action).
 */
internal class PhonePlatform(
    context: Context,
    private val ringer: Ringer,
    private val media: PhoneMedia,
    /** Shows a link a PC sent: (PC's ID, link) → shown. */
    private val onLink: (String, String) -> Boolean,
    /** Answers, declines or silences a call: (call ID, command) → done. */
    private val onCall: (String, CallCommand) -> Boolean,
    /** The phone's texts (created after this). */
    private val sms: () -> PhoneSms,
) : Platform {
    private val context = context.applicationContext

    override fun startRinging() = ringer.startRinging()

    override fun stopRinging() = ringer.stopRinging()

    override fun setClipboard(text: String): Boolean = PhoneClipboard.write(context, text)

    override fun setClipboardImage(mime: String, bytes: ByteArray): Boolean =
        PhoneClipboard.writeImage(context, mime, bytes)

    override fun mediaCommand(player: String, action: MediaAction, position: ULong?) =
        media.command(player, action, position)

    override fun openLink(fromId: String, url: String): Boolean = onLink(fromId, url)

    override fun openPhoto(id: String): FileToSend? = RecentPhotos.open(context, id)

    override fun callCommand(id: String, command: CallCommand): Boolean = onCall(id, command)

    override fun smsThreads(limit: UInt): List<SmsThread> = sms().threads(limit.toInt())

    override fun smsMessages(thread: String, before: Long?, limit: UInt): List<SmsMessage> =
        sms().messages(thread, before, limit.toInt())

    override fun smsSend(to: List<String>, body: String): Boolean = sms().send(to, body)

    override fun smsPart(id: String): SmsPartData? = sms().part(id)

    override fun dismissNotification(key: String) {
        val listener = NotificationListener.instance ?: throw NotificationFailure.Unsupported()
        listener.dismiss(key)
    }

    override fun runNotificationAction(key: String, action: String, reply: String?) {
        val listener = NotificationListener.instance ?: throw NotificationFailure.Unsupported()
        val sbn = listener.find(key) ?: throw NotificationFailure.NotFound()
        val target = action.toIntOrNull()?.let { sbn.notification.actions?.getOrNull(it) }
            ?: throw NotificationFailure.NotFound()
        val intent = target.actionIntent ?: throw NotificationFailure.NotFound()

        // A reply goes to the app the way the notification shade sends it:
        // the text as the result of the action's free-form input.
        val fillIn = Intent()
        if (reply != null) {
            val inputs = target.remoteInputs.orEmpty().filter { it.allowFreeFormInput }
            if (inputs.isEmpty()) throw NotificationFailure.NotFound()
            val results = Bundle().apply { inputs.forEach { putCharSequence(it.resultKey, reply) } }
            RemoteInput.addResultsToIntent(inputs.toTypedArray(), fillIn, results)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                RemoteInput.setResultsSource(fillIn, RemoteInput.SOURCE_FREE_FORM_INPUT)
            }
        }
        try {
            intent.send(context, 0, fillIn, null, null, null, sendOptions())
        } catch (e: PendingIntent.CanceledException) {
            throw NotificationFailure.NotFound()
        }
    }

    /** Lets an action open its app's screen, as tapping it on the phone would. */
    private fun sendOptions(): Bundle? {
        val mode = when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA -> ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE -> {
                @Suppress("DEPRECATION")
                ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED
            }
            else -> return null
        }
        return ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(mode).toBundle()
    }
}
