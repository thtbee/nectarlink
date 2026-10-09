// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.core.content.IntentCompat
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Sends to the connected PCs, without a screen of its own: files or text
 * shared from another app, or the clipboard (from the Quick Settings tile or
 * the connection notification). Android lets only the focused app read the
 * clipboard, which is why this is an activity: it reads once it has focus.
 */
class SendActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var sent = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val intent = intent ?: return finish()
        when (intent.action) {
            Intent.ACTION_VIEW -> {
                val uri = intent.data
                if (uri != null && uri.scheme?.lowercase() == "geo") {
                    val geo = uri.toString()
                    sendLink(geo, geo)
                } else {
                    finishWith(getString(R.string.clip_nothing_shared))
                }
            }
            Intent.ACTION_SEND -> {
                val stream = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
                val link = text?.let(::handoffUrl)
                when {
                    stream != null -> sendFiles(listOf(stream), handoff = true)
                    !text.isNullOrBlank() && link != null -> sendLink(link, text)
                    !text.isNullOrBlank() -> send(text)
                    else -> finishWith(getString(R.string.clip_nothing_shared))
                }
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                val streams = IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                if (streams.isNullOrEmpty()) finishWith(getString(R.string.transfer_nothing)) else sendFiles(streams)
            }
        }
        // The clipboard is read in onWindowFocusChanged.
    }

    private fun targetPcId(): String? {
        val explicit = intent?.getStringExtra(EXTRA_PC_ID)?.takeIf { it.isNotBlank() }
        if (explicit != null) return explicit
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            val shortcutId = intent?.getStringExtra(Intent.EXTRA_SHORTCUT_ID)
            DirectShareTargets.pcIdFromShortcutId(shortcutId)?.let { return it }
        }
        return null
    }

    /** Files go to one PC: the connected one, or the one the user picks. */
    private fun sendFiles(uris: List<Uri>, handoff: Boolean = false) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val chosenPcId = targetPcId()
        if (chosenPcId != null) {
            val pc = core.state.value.device(chosenPcId)
            if (pc != null && pc.online) {
                core.sendFiles(pc.id, uris, handoff)
                finishWith(getString(R.string.transfer_sending, pc.name))
                return
            }
        }
        val pcs = core.state.value.devices.filter { it.online }
        when (pcs.size) {
            0 -> finishWith(getString(R.string.clip_no_pc))
            1 -> {
                core.sendFiles(pcs[0].id, uris, handoff)
                finishWith(getString(R.string.transfer_sending, pcs[0].name))
            }
            else -> AlertDialog.Builder(this)
                .setTitle(R.string.transfer_pick_pc)
                .setItems(pcs.map { it.name }.toTypedArray()) { _, which ->
                    core.sendFiles(pcs[which].id, uris, handoff)
                    finishWith(getString(R.string.transfer_sending, pcs[which].name))
                }
                .setOnCancelListener { finish() }
                .show()
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || sent || intent?.action != ACTION_SEND_CLIPBOARD) return
        when (val clip = PhoneClipboard.read(this)) {
            is PhoneClipboard.Read.Text -> send(clip.text)
            is PhoneClipboard.Read.Image -> sendImage(clip.uri)
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
            if (pc != null && pc.online) {
                if (pc.has("device.links_to_pc")) {
                    scope.launch { finishWith(core.openLinkOnPc(pc.id, url)) }
                } else {
                    send(text)
                }
                return
            }
        }
        val pcs = core.state.value.devices.filter { it.online && it.has("device.links_to_pc") }
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

    private fun send(text: String) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val pcId = targetPcId()
        scope.launch { finishWith(core.sendClipboard(text, pcId)) }
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
        const val EXTRA_PC_ID = "app.nectarlink.extra.PC_ID"

        fun sendClipboardIntent(context: Context, pcId: String? = null): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_CLIPBOARD)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
                .apply {
                    if (!pcId.isNullOrEmpty()) putExtra(EXTRA_PC_ID, pcId)
                }
    }
}
