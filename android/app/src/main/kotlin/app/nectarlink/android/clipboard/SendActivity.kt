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
            Intent.ACTION_SEND -> {
                val stream = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
                when {
                    stream != null -> sendFiles(listOf(stream))
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

    /** Files go to one PC: the connected one, or the one the user picks. */
    private fun sendFiles(uris: List<Uri>) {
        sent = true
        val core = (application as NectarlinkApplication).core
        val pcs = core.state.value.devices.filter { it.online }
        when (pcs.size) {
            0 -> finishWith(getString(R.string.clip_no_pc))
            1 -> {
                core.sendFiles(pcs[0].id, uris)
                finishWith(getString(R.string.transfer_sending, pcs[0].name))
            }
            else -> AlertDialog.Builder(this)
                .setTitle(R.string.transfer_pick_pc)
                .setItems(pcs.map { it.name }.toTypedArray()) { _, which ->
                    core.sendFiles(pcs[which].id, uris)
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

    private fun send(text: String) {
        sent = true
        val core = (application as NectarlinkApplication).core
        scope.launch { finishWith(core.sendClipboard(text)) }
    }

    private fun sendImage(uri: Uri) {
        sent = true
        val core = (application as NectarlinkApplication).core
        scope.launch { finishWith(core.sendClipboardImage(uri)) }
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
        const val ACTION_SEND_CLIPBOARD = "app.nectarlink.action.SEND_CLIPBOARD"

        fun sendClipboardIntent(context: Context): Intent =
            Intent(context, SendActivity::class.java)
                .setAction(ACTION_SEND_CLIPBOARD)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
    }
}
