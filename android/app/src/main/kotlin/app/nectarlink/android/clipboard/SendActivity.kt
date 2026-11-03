// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.core.content.IntentCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Device
import app.nectarlink.android.files.OutgoingFiles
import java.security.MessageDigest
import java.security.SecureRandom
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Sends to the connected PCs: files or text shared from another app, or the
 * clipboard (from the Quick Settings tile or the connection notification).
 * Everything goes the moment it's shared; with several PCs online, the user
 * picks one first. File progress and the confirmation are notifications.
 */
class SendActivity : ComponentActivity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var sent = false
    private val pickFilesLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments(),
    ) { uris ->
        if (uris.isNullOrEmpty()) {
            finish()
        } else {
            sendFiles(uris, handoff = false)
        }
    }

    private sealed interface SharePayload {
        data class Files(val uris: List<Uri>, val handoff: Boolean) : SharePayload

        data class Link(val url: String, val text: String) : SharePayload

        data class Text(val text: String, val html: String?) : SharePayload
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val intent = intent ?: return finish()
        if (intent.action == ACTION_SEND_CLIPBOARD) {
            if (!isTrustedInternalCall(intent)) {
                finish()
                return
            }
            // The clipboard is read in onWindowFocusChanged.
            return
        }
        if (intent.action == ACTION_SEND_FILES) {
            if (!isTrustedInternalCall(intent)) {
                finish()
                return
            }
            if (savedInstanceState == null) {
                pickFilesLauncher.launch(arrayOf("*/*"))
            }
            return
        }
        val payload = parseSharePayload(intent)
        if (payload == null) {
            val msg = if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
                getString(R.string.transfer_nothing)
            } else {
                getString(R.string.clip_nothing_shared)
            }
            finishWith(msg)
            return
        }

        dispatchImmediate(payload)
    }

    private fun isTrustedInternalCall(intent: Intent): Boolean {
        if (!isValidInternalToken(internalToken(this), intent.getStringExtra(EXTRA_INTERNAL_TOKEN))) {
            return false
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val uid = launchedFromUid
            if (uid != Process.myUid() && uid != Process.SYSTEM_UID && !isSystemUid(uid)) {
                return false
            }
        }
        return true
    }

    private fun isSystemUid(uid: Int): Boolean {
        if (uid <= 0 || uid == Process.SYSTEM_UID) return true
        val pkgs = runCatching { packageManager.getPackagesForUid(uid) }.getOrNull().orEmpty()
        return pkgs.any { pkg ->
            val info = runCatching { packageManager.getApplicationInfo(pkg, 0) }.getOrNull()
            info != null && (info.flags and (ApplicationInfo.FLAG_SYSTEM or ApplicationInfo.FLAG_UPDATED_SYSTEM_APP)) != 0
        }
    }

    private fun grantedUrisInIntent(intent: Intent): Set<Uri> = buildSet {
        intent.data?.let { add(it) }
        val clip = intent.clipData
        if (clip != null) {
            for (i in 0 until clip.itemCount) {
                clip.getItemAt(i)?.uri?.let { add(it) }
            }
        }
    }

    /**
     * Only streams the sharing app granted us (Android adds the grant and the
     * ClipData for every shared stream), and never from providers holding
     * messages, contacts or settings, or our own: so another app can't make
     * us send what it couldn't read itself. No check of the sharer's own
     * access: MediaProvider enforces that inside, so Android can't answer it
     * for a gallery sharing its photos, and refusing those would break the
     * share sheet.
     */
    private fun allowedIncomingStreams(intent: Intent, raw: List<Uri>): List<Uri> {
        val hasGrantFlag = (intent.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION) != 0
        return filterGrantedStreamUris(
            hasReadGrantFlag = hasGrantFlag,
            streams = raw,
            clipUris = grantedUrisInIntent(intent),
            schemeOf = { it.scheme },
            authorityOf = { it.authority },
            appPackage = packageName,
        )
    }

    private fun parseSharePayload(intent: Intent): SharePayload? = when (intent.action) {
        Intent.ACTION_VIEW -> {
            val uri = intent.data
            if (uri != null && uri.scheme?.lowercase() == "geo") {
                val geo = uri.toString()
                SharePayload.Link(url = geo, text = geo)
            } else {
                null
            }
        }
        Intent.ACTION_SEND -> {
            val stream = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            val validStreams = if (stream != null) allowedIncomingStreams(intent, listOf(stream)) else emptyList()
            val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
            val html = intent.getStringExtra(Intent.EXTRA_HTML_TEXT)?.takeIf(String::isNotBlank)
            val link = text?.let(::handoffUrl)
            when {
                validStreams.isNotEmpty() -> SharePayload.Files(uris = validStreams, handoff = true)
                stream != null -> null
                !text.isNullOrBlank() && link != null -> SharePayload.Link(url = link, text = text)
                !text.isNullOrBlank() -> SharePayload.Text(text = text, html = html)
                else -> null
            }
        }
        Intent.ACTION_SEND_MULTIPLE -> {
            val streams = IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            val validStreams = if (streams.isNullOrEmpty()) emptyList() else allowedIncomingStreams(intent, streams)
            if (validStreams.isEmpty()) {
                null
            } else {
                SharePayload.Files(uris = validStreams, handoff = false)
            }
        }
        else -> null
    }

    private fun dispatchImmediate(payload: SharePayload) {
        when (payload) {
            is SharePayload.Files -> sendFiles(payload.uris, payload.handoff)
            is SharePayload.Link -> sendLink(payload.url, payload.text)
            is SharePayload.Text -> send(payload.text, payload.html)
        }
    }

    private fun targetPcId(): String? {
        val explicit = intent?.getStringExtra(EXTRA_PC_ID)?.takeIf { it.isNotBlank() }
        if (explicit != null) return explicit
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val shortcutId = intent?.getStringExtra(Intent.EXTRA_SHORTCUT_ID)
            DirectShareTargets.pcIdFromShortcutId(shortcutId)?.let { return it }
        }
        return null
    }

    /** Files go to one PC: the connected one, or the one the user picks (queueing if offline). */
    private fun sendFiles(uris: List<Uri>, handoff: Boolean = false) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val chosen = targetPcId()?.let { core.state.value.device(it) }
        if (chosen != null) {
            sendFilesTo(chosen, uris, handoff)
            return
        }
        val allPcs = core.state.value.devices
        val pcs = allPcs.filter { it.online }.ifEmpty { allPcs }
        when (pcs.size) {
            0 -> finishWith(getString(R.string.clip_no_pc))
            1 -> sendFilesTo(pcs[0], uris, handoff)
            else -> AlertDialog.Builder(this)
                .setTitle(R.string.transfer_pick_pc)
                .setItems(pcs.map { it.name }.toTypedArray()) { _, which -> sendFilesTo(pcs[which], uris, handoff) }
                .setOnCancelListener { finish() }
                .show()
        }
    }

    /**
     * Starts the transfer and closes. A connected PC's progress and the
     * confirmation show as notifications; an offline one queues, and says so.
     */
    private fun sendFilesTo(pc: Device, uris: List<Uri>, handoff: Boolean) {
        (application as NectarlinkApplication).core.sendFiles(pc.id, uris, handoff)
        if (pc.online) finish() else finishWith(getString(R.string.send_queue_offline_toast, pc.name))
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        val currentIntent = intent ?: return
        if (!hasFocus || sent || currentIntent.action != ACTION_SEND_CLIPBOARD) return
        if (!isTrustedInternalCall(currentIntent)) {
            finish()
            return
        }
        when (val clip = PhoneClipboard.read(this)) {
            is PhoneClipboard.Read.Text -> send(clip.text, clip.html)
            is PhoneClipboard.Read.Image -> sendImage(clip.uri)
            is PhoneClipboard.Read.Sensitive -> {
                sent = true
                val core = (application as NectarlinkApplication).core
                val pcId = targetPcId()
                scope.launch { finishWith(core.sendClipboardSensitive(clip.text, pcId)) }
            }
            PhoneClipboard.Read.Private -> finishWith(getString(R.string.clip_private))
            PhoneClipboard.Read.Empty -> finishWith(getString(R.string.clip_empty))
        }
    }

    /**
     * A shared link, video with timestamp, or map location opens on the PC
     * (the one that takes links; the user picks when there are several).
     * With no such PC, it goes to the clipboard like other text.
     */
    private fun sendLink(url: String, text: String) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val chosenPcId = targetPcId()
        if (chosenPcId != null) {
            val pc = core.state.value.device(chosenPcId)
            if (pc != null) {
                if (!pc.online || pc.has("device.links_to_pc")) {
                    scope.launch { finishWith(core.openLinkOnPc(pc.id, url)) }
                } else {
                    send(text)
                }
                return
            }
        }
        val onlineLinkPcs = core.state.value.devices.filter { it.online && it.has("device.links_to_pc") }
        val pcs = onlineLinkPcs.ifEmpty {
            if (core.state.value.devices.none { it.online }) core.state.value.devices else emptyList()
        }
        when (pcs.size) {
            0 -> send(text)
            1 -> scope.launch { finishWith(core.openLinkOnPc(pcs[0].id, url)) }
            else -> AlertDialog.Builder(this)
                .setTitle(R.string.link_pick_pc)
                .setItems(pcs.map { it.name }.toTypedArray()) { _, which ->
                    scope.launch { finishWith(core.openLinkOnPc(pcs[which].id, url)) }
                }
                .setOnCancelListener { finish() }
                .show()
        }
    }

    private fun send(text: String, html: String? = null) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val pcId = targetPcId()
        scope.launch { finishWith(core.sendClipboard(text, pcId, html)) }
    }

    private fun sendImage(uri: Uri) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val pcId = targetPcId()
        scope.launch { finishWith(core.sendClipboardImage(uri, pcId)) }
    }

    private fun finishWith(message: String) {
        sent = true
        Toast.makeText(applicationContext, message, Toast.LENGTH_SHORT).show()
        finish()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        /** The text as a web link, if it's one (apps often share "Title https://…"). */
        fun webLink(text: String): String? {
            val words = text.trim().split(Regex("\\s+"))
            val link = words.lastOrNull() ?: return null
            val lower = link.lowercase()
            val web = (lower.startsWith("https://") || lower.startsWith("http://")) && link.length in 9..4096
            // Only when the link is the point: alone, or after a short title.
            return link.takeIf { web && words.size <= 12 }
        }

        /**
         * Extracts a Handoff URL (web link, YouTube/video URL with timestamp,
         * `geo:` map URI, or street address converted to `geo:`) from shared text.
         */
        fun handoffUrl(text: String): String? {
            app.nectarlink.core.extractHandoffLink(text)?.let { return it.url }
            val suggestion = app.nectarlink.core.classifyClip(text)
            if (suggestion?.kind == app.nectarlink.core.ClipKind.STREET_ADDRESS) {
                return "geo:0,0?q=${Uri.encode(text.trim())}"
            }
            return webLink(text)
        }

        const val ACTION_SEND_CLIPBOARD = "app.nectarlink.action.SEND_CLIPBOARD"
        const val ACTION_SEND_FILES = "app.nectarlink.action.SEND_FILES"
        const val EXTRA_PC_ID = "app.nectarlink.extra.PC_ID"
        private const val EXTRA_INTERNAL_TOKEN = "app.nectarlink.extra.INTERNAL_TOKEN"
        private const val PREFS_INTERNAL = "send_activity_internal"
        private const val KEY_TOKEN = "token"

        internal fun isValidInternalToken(expected: String, given: String?): Boolean {
            if (expected.isEmpty() || given.isNullOrEmpty()) return false
            return MessageDigest.isEqual(
                expected.toByteArray(Charsets.UTF_8),
                given.toByteArray(Charsets.UTF_8),
            )
        }

        internal fun <U> filterGrantedStreamUris(
            hasReadGrantFlag: Boolean,
            streams: List<U>,
            clipUris: Set<U>,
            schemeOf: (U) -> String?,
            authorityOf: (U) -> String?,
            appPackage: String,
        ): List<U> {
            if (!hasReadGrantFlag) return emptyList()
            return streams.filter { u ->
                u in clipUris && OutgoingFiles.isAllowedShareUri(schemeOf(u), authorityOf(u), appPackage)
            }
        }

        @Synchronized
        private fun internalToken(context: Context): String {
            val prefs = context.applicationContext.getSharedPreferences(PREFS_INTERNAL, Context.MODE_PRIVATE)
            val existing = prefs.getString(KEY_TOKEN, null)
            if (!existing.isNullOrEmpty()) return existing
            val bytes = ByteArray(24)
            SecureRandom().nextBytes(bytes)
            val generated = bytes.joinToString("") { "%02x".format(it) }
            prefs.edit().putString(KEY_TOKEN, generated).commit()
            return generated
        }

        fun sendClipboardIntent(context: Context, pcId: String? = null): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_CLIPBOARD)
                .putExtra(EXTRA_INTERNAL_TOKEN, internalToken(context))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                .apply {
                    if (!pcId.isNullOrEmpty()) putExtra(EXTRA_PC_ID, pcId)
                }

        fun sendFilesIntent(context: Context, pcId: String? = null): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_FILES)
                .putExtra(EXTRA_INTERNAL_TOKEN, internalToken(context))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                .apply {
                    if (!pcId.isNullOrEmpty()) putExtra(EXTRA_PC_ID, pcId)
                }
    }
}
