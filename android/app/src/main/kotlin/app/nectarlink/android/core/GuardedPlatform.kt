// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import app.nectarlink.android.diag.Crashes
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallLogEntry
import app.nectarlink.core.CallSim
import app.nectarlink.core.Contact
import app.nectarlink.core.FileToSend
import app.nectarlink.core.LinuxUnlockAttempt
import app.nectarlink.core.MediaAction
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.MirrorOptions
import app.nectarlink.core.PhoneApp
import app.nectarlink.core.PhoneToggleValue
import app.nectarlink.core.Platform
import app.nectarlink.core.PhotoAlbum
import app.nectarlink.core.PhotoItem
import app.nectarlink.core.PhotoThumb
import app.nectarlink.core.PointerPosition
import app.nectarlink.core.PointerScreen
import app.nectarlink.core.ReportDevice
import app.nectarlink.core.SmsAttachment
import app.nectarlink.core.SmsMessage
import app.nectarlink.core.SmsPartData
import app.nectarlink.core.SmsThread
import app.nectarlink.core.StorageEntry
import app.nectarlink.core.StorageReadFile
import app.nectarlink.core.StorageWriteDone
import app.nectarlink.core.TaskNotify
import app.nectarlink.core.WatchFileOffer
import app.nectarlink.core.WebcamOptions

/**
 * The phone's [Platform] as the core calls it: an exception in one of its
 * answers would come back to the core as a panic (#597), so each one is
 * caught here, recorded with where it happened, and answered the way
 * "can't" is answered. The ones with an error of their own (notifications,
 * media, storage, unlock) already turn any exception into that error.
 */
internal class GuardedPlatform(context: Context, private val inner: Platform) : Platform {
    private val context = context.applicationContext

    private inline fun <T> guard(what: String, fallback: T, block: () -> T): T =
        try {
            block()
        } catch (e: Exception) {
            Crashes.caught(context, "Platform.$what", e)
            fallback
        }

    override fun startRinging() = guard("startRinging", Unit) { inner.startRinging() }

    override fun stopRinging() = guard("stopRinging", Unit) { inner.stopRinging() }

    override fun dismissNotification(key: String) = inner.dismissNotification(key)

    override fun runNotificationAction(key: String, action: String, reply: String?) = inner.runNotificationAction(key, action, reply)

    override fun setClipboard(text: String): Boolean = guard("setClipboard", false) { inner.setClipboard(text) }

    override fun setClipboardRich(text: String, html: String?, rtf: String?): Boolean = guard("setClipboardRich", false) { inner.setClipboardRich(text, html, rtf) }

    override fun setClipboardPrivate(text: String): Boolean = guard("setClipboardPrivate", false) { inner.setClipboardPrivate(text) }

    override fun clearClipboardPrivate(text: String) = guard("clearClipboardPrivate", Unit) { inner.clearClipboardPrivate(text) }

    override fun setClipboardImage(mime: String, bytes: ByteArray): Boolean = guard("setClipboardImage", false) { inner.setClipboardImage(mime, bytes) }

    override fun mediaCommand(player: String, action: MediaAction, position: ULong?) = inner.mediaCommand(player, action, position)

    override fun adbMayBeConnected(): Boolean = guard("adbMayBeConnected", false) { inner.adbMayBeConnected() }

    override fun openLink(fromId: String, url: String): Boolean = guard("openLink", false) { inner.openLink(fromId, url) }

    override fun watchOffered(fromId: String, player: String, title: String?, file: WatchFileOffer?): Boolean = guard("watchOffered", false) { inner.watchOffered(fromId, player, title, file) }

    override fun taskNotify(fromId: String, task: TaskNotify): Boolean = guard("taskNotify", false) { inner.taskNotify(fromId, task) }

    override fun openPhoto(id: String): FileToSend? = guard("openPhoto", null) { inner.openPhoto(id) }

    override fun photoAlbums(): List<PhotoAlbum> = guard("photoAlbums", emptyList()) { inner.photoAlbums() }

    override fun photoList(album: String?, before: Long?, beforeId: String, limit: UInt): List<PhotoItem> = guard("photoList", emptyList()) { inner.photoList(album, before, beforeId, limit) }

    override fun photoThumbs(ids: List<String>): List<PhotoThumb> = guard("photoThumbs", emptyList()) { inner.photoThumbs(ids) }

    override fun photoDelete(ids: List<String>): UInt = guard("photoDelete", 0u) { inner.photoDelete(ids) }

    override fun callCommand(id: String, command: CallCommand): Boolean = guard("callCommand", false) { inner.callCommand(id, command) }

    override fun callLog(before: Long?, limit: UInt): List<CallLogEntry> = guard("callLog", emptyList()) { inner.callLog(before, limit) }

    override fun callDial(number: String): Boolean = guard("callDial", false) { inner.callDial(number) }

    override fun callSims(): List<CallSim> = guard("callSims", emptyList()) { inner.callSims() }

    override fun callDialOn(number: String, sim: String): Boolean = guard("callDialOn", false) { inner.callDialOn(number, sim) }

    override fun contacts(query: String?, offset: UInt, limit: UInt): List<Contact> = guard("contacts", emptyList()) { inner.contacts(query, offset, limit) }

    override fun diagFacts(): ReportDevice? = guard("diagFacts", null) { inner.diagFacts() }

    override fun mirrorRequested(pcId: String, options: MirrorOptions): Boolean = guard("mirrorRequested", false) { inner.mirrorRequested(pcId, options) }

    override fun appUninstallRequested(pcId: String, pkg: String): Boolean = guard("appUninstallRequested", false) { inner.appUninstallRequested(pcId, pkg) }

    override fun mirrorStopRequested(pcId: String, session: UInt) = guard("mirrorStopRequested", Unit) { inner.mirrorStopRequested(pcId, session) }

    override fun mirrorKeyframeRequested(pcId: String, session: UInt) = guard("mirrorKeyframeRequested", Unit) { inner.mirrorKeyframeRequested(pcId, session) }

    override fun mirrorResizeRequested(pcId: String, session: UInt, width: UInt, height: UInt, zoom: UInt) = guard("mirrorResizeRequested", Unit) { inner.mirrorResizeRequested(pcId, session, width, height, zoom) }

    override fun mirrorPowerRequested(pcId: String, stayAwake: Boolean, screenOff: Boolean) = guard("mirrorPowerRequested", Unit) { inner.mirrorPowerRequested(pcId, stayAwake, screenOff) }

    override fun mirrorRotateRequested(pcId: String, landscape: Boolean) = guard("mirrorRotateRequested", Unit) { inner.mirrorRotateRequested(pcId, landscape) }

    override fun mirrorSoundRequested(pcId: String, on: Boolean) = guard("mirrorSoundRequested", Unit) { inner.mirrorSoundRequested(pcId, on) }

    override fun mirrorVolumeChanged(pcId: String, level: UByte) = guard("mirrorVolumeChanged", Unit) { inner.mirrorVolumeChanged(pcId, level) }

    override fun mirrorInput(pcId: String, session: UInt, input: MirrorInputEvent) = guard("mirrorInput", Unit) { inner.mirrorInput(pcId, session, input) }

    override fun pointerEnter(pcId: String, edge: String, y: Float): PointerScreen? = guard("pointerEnter", null) { inner.pointerEnter(pcId, edge, y) }

    override fun pointerMove(pcId: String, dx: Int, dy: Int, buttons: UByte, wheel: Byte) = guard("pointerMove", Unit) { inner.pointerMove(pcId, dx, dy, buttons, wheel) }
    override fun deviceInput(peer: String, input: app.nectarlink.core.DeviceInput): Boolean = guard("deviceInput", false) { inner.deviceInput(peer, input) }

    override fun pointerWhere(pcId: String): PointerPosition? = guard("pointerWhere", null) { inner.pointerWhere(pcId) }

    override fun pointerLeave(pcId: String) = guard("pointerLeave", Unit) { inner.pointerLeave(pcId) }

    override fun webcamRequested(pcId: String, options: WebcamOptions): Boolean = guard("webcamRequested", false) { inner.webcamRequested(pcId, options) }

    override fun webcamStopRequested(pcId: String) = guard("webcamStopRequested", Unit) { inner.webcamStopRequested(pcId) }

    override fun micRequested(pcId: String): Boolean = guard("micRequested", false) { inner.micRequested(pcId) }

    override fun micStopRequested(pcId: String) = guard("micStopRequested", Unit) { inner.micStopRequested(pcId) }

    override fun callAudioRequested(pcId: String, callId: String): Boolean = guard("callAudioRequested", false) { inner.callAudioRequested(pcId, callId) }

    override fun callAudioStopRequested(pcId: String) = guard("callAudioStopRequested", Unit) { inner.callAudioStopRequested(pcId) }

    override fun webcamKeyframeRequested(pcId: String) = guard("webcamKeyframeRequested", Unit) { inner.webcamKeyframeRequested(pcId) }

    override fun cameraCaptureRequested(pcId: String, requestId: String, mode: String): Boolean = guard("cameraCaptureRequested", false) { inner.cameraCaptureRequested(pcId, requestId, mode) }

    override fun cameraCaptureCancelled(pcId: String, requestId: String) = guard("cameraCaptureCancelled", Unit) { inner.cameraCaptureCancelled(pcId, requestId) }

    override fun phoneApps(): List<PhoneApp> = guard("phoneApps", emptyList()) { inner.phoneApps() }

    override fun smsThreads(limit: UInt): List<SmsThread> = guard("smsThreads", emptyList()) { inner.smsThreads(limit) }

    override fun smsMessages(thread: String, before: Long?, limit: UInt): List<SmsMessage> = guard("smsMessages", emptyList()) { inner.smsMessages(thread, before, limit) }

    override fun smsSend(to: List<String>, body: String): Boolean = guard("smsSend", false) { inner.smsSend(to, body) }

    override fun sendSmsWithAttachments(to: List<String>, body: String, attachments: List<SmsAttachment>): Boolean = guard("sendSmsWithAttachments", false) { inner.sendSmsWithAttachments(to, body, attachments) }

    override fun smsSims(): List<CallSim> = guard("smsSims", emptyList()) { inner.smsSims() }

    override fun smsSendOn(to: List<String>, body: String, attachments: List<SmsAttachment>, sim: String): Boolean = guard("smsSendOn", false) { inner.smsSendOn(to, body, attachments, sim) }

    override fun smsMarkRead(thread: String): Boolean = guard("smsMarkRead", false) { inner.smsMarkRead(thread) }

    override fun smsMarkUnread(thread: String, message: String?): Boolean =
        guard("smsMarkUnread", false) { inner.smsMarkUnread(thread, message) }

    override fun smsDelete(thread: String, messages: List<String>): Boolean =
        guard("smsDelete", false) { inner.smsDelete(thread, messages) }

    override fun smsPart(id: String): SmsPartData? = guard("smsPart", null) { inner.smsPart(id) }

    override fun setPhoneToggle(id: String, value: PhoneToggleValue): Boolean = guard("setPhoneToggle", false) { inner.setPhoneToggle(id, value) }

    override fun unlockWrap(pc: String, pcName: String, userName: String, user: String, key: ByteArray): ByteArray = inner.unlockWrap(pc, pcName, userName, user, key)

    override fun unlockUnwrap(pc: String, pcName: String, userName: String, user: String, wrapped: ByteArray): ByteArray = inner.unlockUnwrap(pc, pcName, userName, user, wrapped)

    override fun unlockForget(pc: String, user: String?) = guard("unlockForget", Unit) { inner.unlockForget(pc, user) }

    override fun linuxUnlockEnroll(pc: String, pcName: String, userName: String, uid: UInt, timeoutMs: ULong, attempt: LinuxUnlockAttempt): ByteArray = inner.linuxUnlockEnroll(pc, pcName, userName, uid, timeoutMs, attempt)

    override fun linuxUnlockSign(pc: String, uid: UInt, transcript: ByteArray, attempt: LinuxUnlockAttempt): ByteArray = inner.linuxUnlockSign(pc, uid, transcript, attempt)

    override fun linuxUnlockForget(pc: String, uid: UInt?) = guard("linuxUnlockForget", Unit) { inner.linuxUnlockForget(pc, uid) }

    override fun storageList(path: String): List<StorageEntry> = inner.storageList(path)

    override fun storageOpenRead(path: String): StorageReadFile = inner.storageOpenRead(path)

    override fun storageWrite(path: String, stagedPath: String, modified: Long?): StorageWriteDone = inner.storageWrite(path, stagedPath, modified)

    override fun storageMkdir(path: String) = inner.storageMkdir(path)

    override fun storageRename(from: String, to: String) = inner.storageRename(from, to)

    override fun storageDelete(path: String, confirmed: Boolean) = inner.storageDelete(path, confirmed)
}
