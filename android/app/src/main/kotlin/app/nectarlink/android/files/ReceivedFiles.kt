// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.annotation.RequiresApi
import androidx.core.content.FileProvider
import java.io.File

/**
 * Files from a PC land in the app's cache first (the core writes them
 * there); this puts them where the user finds them: Download/Nectarlink.
 */
internal object ReceivedFiles {
    /** A received file where the user can open it. */
    data class Published(val uri: Uri, val name: String, val mime: String)

    fun mimeOf(name: String): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
            ?: "application/octet-stream"

    /** Moves a received file into Downloads; `null` if that failed (the file stays where it is). */
    fun publish(context: Context, file: File): Published? = runCatching {
        val mime = mimeOf(file.name)
        val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            toMediaStore(context, file, mime)
        } else {
            toAppDownloads(context, file)
        }
        file.delete()
        Published(uri, file.name, mime)
    }.getOrNull()

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun toMediaStore(context: Context, file: File, mime: String): Uri {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, file.name)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Nectarlink")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = checkNotNull(resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values))
        try {
            checkNotNull(resolver.openOutputStream(uri)).use { out -> file.inputStream().use { it.copyTo(out) } }
            resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        return uri
    }

    /** Android 8–9: the app's own Downloads folder, shared through a FileProvider. */
    private fun toAppDownloads(context: Context, file: File): Uri {
        val dir = File(context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), "Nectarlink").apply { mkdirs() }
        var target = File(dir, file.name)
        var n = 2
        while (target.exists()) {
            target = File(dir, "${file.nameWithoutExtension} ($n)${file.extension.let { if (it.isEmpty()) "" else ".$it" }}")
            n++
        }
        file.copyTo(target)
        return FileProvider.getUriForFile(context, "${context.packageName}.files", target)
    }
}
