// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.OpenableColumns
import app.nectarlink.core.FileToSend

/** Files the user picked or shared, as the core takes them: name and an open descriptor. */
internal object OutgoingFiles {
    /** Skips items that can't be opened (the result says how many were). */
    fun open(context: Context, uris: List<Uri>): List<FileToSend> = uris.mapNotNull { uri ->
        val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: return@mapNotNull null
        FileToSend.Fd(name = displayName(context, uri), fd = fd.detachFd(), folder = null)
    }

    /** Why a folder can't be sent. */
    class TooManyFiles : Exception("too many files")

    /**
     * Everything in a folder the user picked (`OpenDocumentTree`), with
     * where each file is in it. Empty folders are left out. Every file is
     * opened now, so there's a limit on how many.
     */
    fun openFolder(context: Context, tree: Uri): List<FileToSend> {
        val rootId = DocumentsContract.getTreeDocumentId(tree)
        val rootName = query(context, DocumentsContract.buildDocumentUriUsingTree(tree, rootId)).firstOrNull()?.name
        val files = mutableListOf<FileToSend>()
        try {
            walk(context, tree, rootId, safeName(rootName ?: "Folder"), depth = 1, files)
        } catch (e: Exception) {
            // Close what was opened before giving up.
            files.filterIsInstance<FileToSend.Fd>().forEach { runCatching { ParcelFileDescriptor.adoptFd(it.fd).close() } }
            throw e
        }
        return files
    }

    private data class Child(val id: String, val name: String, val folder: Boolean)

    private fun walk(context: Context, tree: Uri, parentId: String, folder: String, depth: Int, out: MutableList<FileToSend>) {
        if (depth > MAX_DEPTH) throw TooManyFiles()
        val children = query(context, DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId)).sortedBy { it.name }
        for (child in children) {
            val name = safeName(child.name)
            if (child.folder) {
                walk(context, tree, child.id, "$folder/$name", depth + 1, out)
            } else {
                if (out.size >= maxFiles) throw TooManyFiles()
                val uri = DocumentsContract.buildDocumentUriUsingTree(tree, child.id)
                val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: continue
                out += FileToSend.Fd(name = name, fd = fd.detachFd(), folder = folder)
            }
        }
    }

    private fun query(context: Context, uri: Uri): List<Child> {
        val columns = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE)
        return context.contentResolver.query(uri, columns, null, null, null)?.use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    val id = cursor.getString(0) ?: continue
                    add(Child(id, cursor.getString(1) ?: "file", cursor.getString(2) == Document.MIME_TYPE_DIR))
                }
            }
        }.orEmpty()
    }

    /** A name without the characters folder paths use; the core checks the rest. */
    private fun safeName(name: String): String =
        name.replace('/', '_').replace('\\', '_').trim().ifEmpty { "file" }.take(200)

    private fun displayName(context: Context, uri: Uri): String =
        runCatching {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull() ?: uri.lastPathSegment?.substringAfterLast('/') ?: "file"

    private const val MAX_DEPTH = 32

    /**
     * Each file stays open until it's sent. Android 9 and later allow an
     * app tens of thousands of open files; Android 8 only about a thousand.
     */
    private val maxFiles = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) 5000 else 500
}
