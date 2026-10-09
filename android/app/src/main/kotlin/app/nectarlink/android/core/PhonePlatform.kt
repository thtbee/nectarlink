// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.app.ActivityOptions
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.camera.ContinuityCameraRequests
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mirror.AppWindows
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.mirror.MirrorRequest
import app.nectarlink.android.mirror.MirrorRequests
import app.nectarlink.android.mirror.MirrorService
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.webcam.WebcamRequest
import app.nectarlink.android.webcam.WebcamRequests
import app.nectarlink.android.webcam.WebcamService
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallLogEntry
import app.nectarlink.core.Contact
import app.nectarlink.core.SmsAttachment
import app.nectarlink.core.SmsMessage
import app.nectarlink.core.SmsPartData
import app.nectarlink.core.SmsThread
import app.nectarlink.core.FileToSend
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.MirrorOptions
import app.nectarlink.core.PhoneApp
import app.nectarlink.core.PhoneToggleValue
import app.nectarlink.core.PhotoAlbum
import app.nectarlink.core.PhotoItem
import app.nectarlink.core.PhotoThumb
import app.nectarlink.core.NotificationFailure
import app.nectarlink.core.Platform
import app.nectarlink.core.StorageEntry
import app.nectarlink.core.StorageReadFile
import app.nectarlink.core.StorageWriteDone
import app.nectarlink.core.TaskNotify
import app.nectarlink.core.WebcamOptions

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
    /** Shows or updates a watched task from a PC: (PC's ID, task) → shown. */
    private val onTask: (String, TaskNotify) -> Boolean,
    /** The phone's calls (created after this). */
    private val calls: () -> PhoneCalls,
    /** The phone's contacts (created after this). */
    private val contacts: () -> PhoneContacts,
    /** The phone's texts (created after this). */
    private val sms: () -> PhoneSms,
    /** The phone's quick settings (created after this). */
    private val toggles: () -> PhoneToggles,
    /** The phone's shared storage (created after this). */
    private val storage: () -> PhoneStorage,
    /** Asks the user to share the screen with a PC: (PC's ID, request) → asked. */
    private val onMirror: (String, MirrorRequest) -> Boolean,
    /** Asks the user to stream the camera as a webcam to a PC: (PC's ID, request) → asked. */
    private val onWebcam: (String, WebcamRequest) -> Boolean,
    /** Apps in windows of their own on PCs (Elevated). */
    private val appWindows: AppWindows,
    /** Resolves a paired PC's display name by ID. */
    private val nameOf: (String) -> String = { "" },
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

    override fun taskNotify(fromId: String, task: TaskNotify): Boolean = onTask(fromId, task)

    override fun openPhoto(id: String): FileToSend? = RecentPhotos.open(context, id)

    override fun photoAlbums(): List<PhotoAlbum> = RecentPhotos.albums(context)

    override fun photoList(album: String?, before: Long?, beforeId: String, limit: UInt): List<PhotoItem> =
        RecentPhotos.list(context, album, before, beforeId, limit.toInt())

    override fun photoThumbs(ids: List<String>): List<PhotoThumb> = RecentPhotos.thumbs(context, ids)

    override fun callCommand(id: String, command: CallCommand): Boolean = calls().command(id, command)

    override fun callLog(before: Long?, limit: UInt): List<CallLogEntry> =
        calls().callLog(before, limit.toInt())

    override fun callDial(number: String): Boolean = calls().dial(number)

    override fun contacts(query: String?, offset: UInt, limit: UInt): List<Contact> =
        contacts().list(query, offset.toInt(), limit.toInt())

    override fun mirrorRequested(pcId: String, options: MirrorOptions): Boolean =
        if (options.app != null) {
            appWindows.open(pcId, options)
        } else {
            val request = MirrorRequest(pcId, options.maxSize.toInt(), options.fps.toInt(), options.bitrate.toInt(), options.audio)
            onMirror(pcId, request)
        }

    override fun mirrorStopRequested(pcId: String, session: UInt) {
        if (session != SCREEN) return appWindows.close(pcId, session)
        MirrorRequests.dismiss(context, pcId)
        MirrorService.stop(pcId)
    }

    override fun mirrorKeyframeRequested(pcId: String, session: UInt) =
        if (session == SCREEN) MirrorService.keyframe(pcId) else appWindows.keyframe(pcId, session)

    override fun mirrorResizeRequested(pcId: String, session: UInt, width: UInt, height: UInt) {
        if (session != SCREEN) appWindows.resize(pcId, session, width.toInt(), height.toInt())
    }

    // Real events when Elevated runs; gestures through the accessibility service otherwise.
    override fun mirrorInput(pcId: String, session: UInt, input: MirrorInputEvent) {
        if (session != SCREEN) return appWindows.input(pcId, session, input)
        if (!Elevated.handle(input)) InputService.handle(input)
    }

    override fun webcamRequested(pcId: String, options: WebcamOptions): Boolean {
        val request = WebcamRequest(
            pcId = pcId,
            width = options.width.toInt(),
            height = options.height.toInt(),
            fps = options.fps.toInt(),
            bitrate = options.bitrate.toInt(),
            camera = options.camera,
        )
        return onWebcam(pcId, request)
    }

    override fun webcamStopRequested(pcId: String) {
        WebcamRequests.dismiss(context, pcId)
        WebcamService.stopForPc(context, pcId)
    }

    override fun webcamKeyframeRequested(pcId: String) =
        WebcamService.keyframe(pcId)

    override fun cameraCaptureRequested(pcId: String, requestId: String, mode: String): Boolean =
        ContinuityCameraRequests.show(context, pcId, requestId, mode, nameOf(pcId))

    override fun cameraCaptureCancelled(pcId: String, requestId: String) {
        ContinuityCameraRequests.cancelFromPc(context, pcId, requestId)
    }

    override fun phoneApps(): List<PhoneApp> = AppWindows.list(context)

    override fun smsThreads(limit: UInt): List<SmsThread> = sms().threads(limit.toInt())

    override fun smsMessages(thread: String, before: Long?, limit: UInt): List<SmsMessage> =
        sms().messages(thread, before, limit.toInt())

    override fun smsSend(to: List<String>, body: String): Boolean = sms().send(to, body)

    override fun sendSmsWithAttachments(to: List<String>, body: String, attachments: List<SmsAttachment>): Boolean =
        sms().send(to, body, attachments)

    override fun smsPart(id: String): SmsPartData? = sms().part(id)

    override fun setPhoneToggle(id: String, value: PhoneToggleValue): Boolean =
        toggles().set(id, value)

    override fun storageList(path: String): List<StorageEntry> = storage().list(path)

    override fun storageOpenRead(path: String): StorageReadFile = storage().openRead(path)

    override fun storageWrite(path: String, stagedPath: String, modified: Long?): StorageWriteDone =
        storage().write(path, stagedPath, modified)

    override fun storageMkdir(path: String) = storage().mkdir(path)

    override fun storageRename(from: String, to: String) = storage().rename(from, to)

    override fun storageDelete(path: String, confirmed: Boolean) = storage().delete(path, confirmed)

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

    private companion object {
        /** The phone's own screen's mirroring session. */
        const val SCREEN = 0u
    }
}
