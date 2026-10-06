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

    /**
     * Moves a received file, or a folder with everything in it, into
     * Downloads. Returns what's there now; a file that failed stays where
     * it is and is left out.
     */
    fun publish(context: Context, item: File): List<Published> =
        if (item.isDirectory) {
            item.walkTopDown().filter { it.isFile }.mapNotNull { file ->
                val folder = file.parentFile!!.relativeTo(item.parentFile!!).invariantSeparatorsPath
                publishFile(context, file, folder)
            }.toList().also {
                item.walkBottomUp().filter { it.isDirectory }.forEach { dir ->
                    if (dir.list()?.isEmpty() == true) dir.delete()
                }
            }
        } else {
            listOfNotNull(publishFile(context, item, folder = null))
        }

    /** `folder`: where it goes in Download/Nectarlink (`Trip/Day 1`). */
    private fun publishFile(context: Context, file: File, folder: String?): Published? = runCatching {
        val mime = mimeOf(file.name)
        val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            toMediaStore(context, file, mime, folder)
        } else {
            toAppDownloads(context, file, folder)
        }
        file.delete()
        Published(uri, file.name, mime)
    }.getOrNull()

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun toMediaStore(context: Context, file: File, mime: String, folder: String?): Uri {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, file.name)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, listOfNotNull(Environment.DIRECTORY_DOWNLOADS, "Nectarlink", folder).joinToString("/"))
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
    private fun toAppDownloads(context: Context, file: File, folder: String?): Uri {
        val base = File(context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), "Nectarlink")
        val dir = (if (folder == null) base else File(base, folder)).apply { mkdirs() }
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
