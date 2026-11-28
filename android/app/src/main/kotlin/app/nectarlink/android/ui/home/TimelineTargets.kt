// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import app.nectarlink.core.TimelineEntry
import app.nectarlink.core.TimelineKind
import java.io.File

/** Opens what a timeline entry points at: its link, or its file on this phone. */
internal fun openTimelineTarget(context: Context, entry: TimelineEntry) {
    when (entry.kind) {
        TimelineKind.LINK -> runCatching {
            val url = entry.target.ifEmpty { entry.title }
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        TimelineKind.FILE, TimelineKind.PHOTO, TimelineKind.RECORDING -> openTimelineFile(context, entry.kind, entry.target)
        else -> Unit
    }
}

/** Opens a file, photo or recording on this phone ([target]: a path or a `content://` URI) in an app for it. */
internal fun openTimelineFile(context: Context, kind: TimelineKind, target: String) {
    runCatching {
        val uri = timelineFileUri(context, target) ?: return
        val mime = timelineFileMime(context, kind, target, uri)
        val view = Intent(Intent.ACTION_VIEW)
            .apply {
                if (mime != null) setDataAndType(uri, mime) else setData(uri)
            }
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

/** Shares a file, photo or recording on this phone ([target]: a path or a `content://` URI) with another app. */
internal fun shareTimelineFile(context: Context, kind: TimelineKind, target: String) {
    runCatching {
        val uri = timelineFileUri(context, target) ?: return
        val mime = timelineFileMime(context, kind, target, uri) ?: "*/*"
        val send = Intent(Intent.ACTION_SEND)
            .setType(mime)
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

/** A URI other apps can read [target] by, or null when it isn't on this phone. */
internal fun timelineFileUri(context: Context, target: String): Uri? = when {
    target.startsWith("content://") -> Uri.parse(target)
    target.isNotEmpty() && File(target).exists() ->
        FileProvider.getUriForFile(context, "${context.packageName}.files", File(target))
    else -> null
}

private fun timelineFileMime(context: Context, kind: TimelineKind, target: String, uri: Uri): String? {
    val ext = target.substringAfterLast('.', "").lowercase()
    return runCatching { context.contentResolver.getType(uri) }.getOrNull()?.takeIf { it != "application/octet-stream" }
        ?: android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
        ?: when (kind) {
            TimelineKind.PHOTO -> "image/*"
            TimelineKind.RECORDING -> "audio/*"
            else -> null
        }
}
