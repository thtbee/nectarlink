// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import app.nectarlink.core.FileToSend

/** Files the user picked or shared, as the core takes them: name and an open descriptor. */
internal object OutgoingFiles {
    /** Skips items that can't be opened (the result says how many were). */
    fun open(context: Context, uris: List<Uri>): List<FileToSend> = uris.mapNotNull { uri ->
        val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: return@mapNotNull null
        FileToSend.Fd(name = displayName(context, uri), fd = fd.detachFd())
    }

    private fun displayName(context: Context, uri: Uri): String =
        runCatching {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull() ?: uri.lastPathSegment?.substringAfterLast('/') ?: "file"
}
