// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.format.Formatter
import android.view.View
import android.webkit.MimeTypeMap
import android.widget.RemoteViews
import androidx.core.content.FileProvider
import androidx.core.net.toUri
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.core.ClipboardHistoryEntry
import app.nectarlink.core.ClipboardItemKind
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import java.io.File

/** From your PC: the last clip a PC sent, to copy again, and the last file, to open. */
class FromPcWidgetProvider : BloomWidget() {

    override fun render(context: Context, id: Int, options: Bundle, state: CoreState?): RemoteViews {
        val views = RemoteViews(context.packageName, R.layout.widget_from_pc)
        views.setOnClickPendingIntent(R.id.fp_root, Widgets.openApp(context))
        val clip = lastClip(state)
        val file = lastFile(state)

        views.setViewVisibility(R.id.fp_empty, if (clip == null && file == null) View.VISIBLE else View.GONE)
        if (clip == null) {
            views.setViewVisibility(R.id.fp_clip, View.GONE)
        } else {
            views.setViewVisibility(R.id.fp_clip, View.VISIBLE)
            val text = if (clip.kind == ClipboardItemKind.IMAGE) {
                context.getString(R.string.clipboard_history_image)
            } else {
                // One paragraph: line breaks waste the two lines the well has.
                clip.text.trim().replace(Regex("\\s+"), " ")
            }
            views.setTextViewText(R.id.fp_clip_text, text)
            views.setTextViewText(R.id.fp_clip_meta, meta(clip.deviceName, Widgets.whenText(context, clip.timestamp)))
            views.setOnClickPendingIntent(
                R.id.fp_clip_copy,
                Widgets.action(context, PcWidgetProvider.ACTION_COPY_CLIP, null, Bundle().apply { putString(PcWidgetProvider.EXTRA_CLIP_ID, clip.id) }),
            )
        }

        val open = file?.let { openFile(context, it) }
        if (file == null) {
            views.setViewVisibility(R.id.fp_file, View.GONE)
        } else {
            views.setViewVisibility(R.id.fp_file, View.VISIBLE)
            views.setTextViewText(R.id.fp_file_name, file.title)
            val size = if (file.sizeBytes > 0u) Formatter.formatShortFileSize(context, file.sizeBytes.toLong()) else null
            views.setTextViewText(
                R.id.fp_file_meta,
                meta(file.deviceName, listOfNotNull(size, Widgets.whenText(context, file.timestamp)).joinToString(" · ")),
            )
            // A file that's gone (moved or deleted) can't open: the chip hides and the well opens the app.
            views.setViewVisibility(R.id.fp_file_open, if (open != null) View.VISIBLE else View.GONE)
            views.setOnClickPendingIntent(R.id.fp_file_open, open)
            views.setOnClickPendingIntent(R.id.fp_file, open ?: Widgets.openApp(context))
        }
        return views
    }

    private fun meta(device: String, detail: String) = if (device.isBlank()) detail else "$device · $detail"

    private fun lastClip(state: CoreState?): ClipboardHistoryEntry? =
        state?.clipboardHistory.orEmpty().filter { it.incoming }.maxByOrNull { it.timestamp }

    private fun lastFile(state: CoreState?): TimelineEntry? = state?.timeline.orEmpty()
        .filter { it.incoming && it.kind in FILE_KINDS && it.target.isNotEmpty() }
        .maxByOrNull { it.timestamp }

    /** Opens [entry] in the app the user picks, as the timeline does, or null if it's gone. */
    private fun openFile(context: Context, entry: TimelineEntry): PendingIntent? = runCatching {
        val target = entry.target
        val uri = when {
            target.startsWith("content://") -> target.toUri()
            File(target).exists() -> FileProvider.getUriForFile(context, "${context.packageName}.files", File(target))
            else -> return null
        }
        val ext = target.substringAfterLast('.', "").lowercase()
        val mime = context.contentResolver.getType(uri)?.takeIf { it != "application/octet-stream" }
            ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
            ?: when (entry.kind) {
                TimelineKind.PHOTO -> "image/*"
                TimelineKind.RECORDING -> "audio/*"
                else -> null
            }
        val view = Intent(Intent.ACTION_VIEW)
            .apply { if (mime != null) setDataAndType(uri, mime) else setData(uri) }
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        val chooser = Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        Widgets.activity(context, REQ_OPEN_FILE, chooser)
    }.getOrNull()

    private companion object {
        const val REQ_OPEN_FILE = 108
        val FILE_KINDS = setOf(TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING)
    }
}
