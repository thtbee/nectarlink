// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.core.content.IntentCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Device
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
            // The clipboard is read in onWindowFocusChanged.
            return
        }
        if (intent.action == ACTION_SEND_FILES) {
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
            val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
            val html = intent.getStringExtra(Intent.EXTRA_HTML_TEXT)?.takeIf(String::isNotBlank)
            val link = text?.let(::handoffUrl)
            when {
                stream != null -> SharePayload.Files(uris = listOf(stream), handoff = true)
                !text.isNullOrBlank() && link != null -> SharePayload.Link(url = link, text = text)
                !text.isNullOrBlank() -> SharePayload.Text(text = text, html = html)
                else -> null
            }
        }
        Intent.ACTION_SEND_MULTIPLE -> {
            val streams = IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            if (streams.isNullOrEmpty()) {
                null
            } else {
                SharePayload.Files(uris = streams, handoff = false)
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
        if (!hasFocus || sent || intent?.action != ACTION_SEND_CLIPBOARD) return
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

        fun sendClipboardIntent(context: Context, pcId: String? = null): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_CLIPBOARD)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                .apply {
                    if (!pcId.isNullOrEmpty()) putExtra(EXTRA_PC_ID, pcId)
                }

        fun sendFilesIntent(context: Context, pcId: String? = null): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_FILES)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                .apply {
                    if (!pcId.isNullOrEmpty()) putExtra(EXTRA_PC_ID, pcId)
                }
    }
}
