// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.OpenableColumns
import android.system.Os
import app.nectarlink.core.FileToSend
import java.io.File

/** Files the user picked or shared, as the core takes them: name and an open descriptor. */
internal object OutgoingFiles {
    /** Sensitive non-file system authorities that must never be exfiltrated via share or clipboard URIs. */
    private val BLOCKED_AUTHORITIES = setOf(
        "sms",
        "mms",
        "mms-sms",
        "telephony",
        "call_log",
        "contacts",
        "com.android.contacts",
        "com.android.blockednumber",
        "user_dictionary",
        "settings",
    )

    internal fun isAllowedShareUri(scheme: String?, authority: String?, appPackage: String): Boolean {
        if (!scheme.equals("content", ignoreCase = true)) return false
        val cleanAuth = authority?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return false
        if (cleanAuth in BLOCKED_AUTHORITIES) return false
        if (cleanAuth == "${appPackage.lowercase()}.shizuku") return false
        return true
    }

    internal fun isAppPrivatePath(
        resolvedPath: String,
        privateDirs: List<File>,
        allowedSubdirs: List<File> = emptyList(),
    ): Boolean {
        val candPath = runCatching { File(resolvedPath).canonicalPath }.getOrDefault(File(resolvedPath).absolutePath)
        for (allowed in allowedSubdirs) {
            val ap = runCatching { allowed.canonicalPath }.getOrDefault(allowed.absolutePath)
            if (candPath == ap || candPath.startsWith("$ap${File.separator}")) {
                return false
            }
        }
        for (priv in privateDirs) {
            val pp = runCatching { priv.canonicalPath }.getOrDefault(priv.absolutePath)
            if (candPath == pp || candPath.startsWith("$pp${File.separator}")) {
                return true
            }
        }
        return false
    }

    internal fun appPrivateDirs(context: Context): List<File> = buildList {
        val pkg = context.packageName
        add(File("/data/data/$pkg"))
        add(File("/data/user/0/$pkg"))
        context.dataDir?.let { add(it) }
        context.filesDir?.let { add(it) }
        context.noBackupFilesDir?.let { add(it) }
        context.cacheDir?.let { add(it) }
        context.codeCacheDir?.let { add(it) }
        context.externalCacheDir?.let { add(it) }
        context.getExternalFilesDir(null)?.let { add(it) }
    }

    internal fun isSafeOpenedFd(
        context: Context,
        pfd: ParcelFileDescriptor,
        allowClipboardCache: Boolean = false,
    ): Boolean {
        val target = runCatching { Os.readlink("/proc/self/fd/${pfd.fd}") }.getOrNull() ?: return true
        val allowed = if (allowClipboardCache) {
            listOf(File(context.cacheDir, "clipboard"))
        } else {
            emptyList()
        }
        return !isAppPrivatePath(target, appPrivateDirs(context), allowed)
    }

    /** Closes detached file descriptors when a transfer is aborted before reaching the core. */
    fun closeAll(files: List<FileToSend>) {
        files.filterIsInstance<FileToSend.Fd>().forEach {
            runCatching { ParcelFileDescriptor.adoptFd(it.fd).close() }
        }
    }

    /** Skips items that can't be opened (the result says how many were). */
    fun open(context: Context, uris: List<Uri>): List<FileToSend> {
        if (uris.size > maxFiles) throw TooManyFiles()
        val files = mutableListOf<FileToSend>()
        try {
            for (uri in uris) {
                if (!isAllowedShareUri(uri.scheme, uri.authority, context.packageName)) continue
                val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: continue
                if (!isSafeOpenedFd(context, fd)) {
                    runCatching { fd.close() }
                    continue
                }
                val name = displayName(context, uri)
                files += FileToSend.Fd(name = name, fd = fd.detachFd(), folder = null)
            }
        } catch (e: Exception) {
            closeAll(files)
            throw e
        }
        return files
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
            closeAll(files)
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
