// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.storage

import android.Manifest
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.FileObserver
import android.os.storage.StorageManager
import android.os.storage.StorageVolume
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.Settings
import androidx.core.content.ContextCompat
import app.nectarlink.core.FileToSend
import app.nectarlink.core.StorageEntry
import app.nectarlink.core.StorageFailure
import app.nectarlink.core.StorageReadFile
import app.nectarlink.core.StorageWriteDone
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.nio.file.Files

/**
 * Exposes this phone's shared storage (or user-picked SAF folders when full
 * storage access is not granted) to paired PCs for browsing in File Explorer
 * (`docs/protocol/storage.md`). With full access, each SD card or USB drive
 * shows as a folder of its own at the root, named as Android names it.
 *
 * Security invariants:
 * - Paths are relative to the shared root; `..`, `.`, leading/trailing `/`,
 *   `\`, NUL, and control characters are rejected.
 * - Never exposes `/data`, `Android/data`, `Android/obb`, or NectarLink's own
 *   private directories.
 * - Symlinks resolving outside the canonical storage root (or into a blocked
 *   directory) are rejected.
 */
class PhoneStorage(
    context: Context,
    private val onChanged: (String) -> Unit = {},
) {
    private val context = context.applicationContext
    private val observers = LinkedHashMap<String, FileObserver>()

    /** Lists entries in `path` (`""` for the root). */
    fun list(path: String): List<StorageEntry> {
        if (!isValidDirPath(path)) {
            throw StorageFailure.Invalid("invalid directory path")
        }
        if (hasAllFilesAccess(context)) {
            val volumes = removableVolumes()
            val (root, rel) = locate(volumes, path)
            val blocked = blockedDirs(context, root)
            val dir = resolveExistingInRoot(root, blocked, rel, allowRoot = true)
            if (!dir.isDirectory) throw StorageFailure.NotFound()
            watchDirectory(path, dir)
            val children = dir.listFiles() ?: throw StorageFailure.Denied()
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            val out = ArrayList<StorageEntry>(children.size + volumes.size)
            if (path.isEmpty()) {
                for (volume in volumes) {
                    out.add(
                        StorageEntry(name = volume.name, size = 0uL, modified = volume.dir.lastModified(), isDir = true),
                    )
                }
            }
            for (child in children) {
                val name = child.name
                if (!isValidName(name) || name.startsWith(".nectarlink")) continue
                val canon = runCatching { child.canonicalFile }.getOrNull() ?: continue
                if (!isInsideRoot(canonRoot, blocked, canon)) continue
                val isDir = canon.isDirectory
                if (!isDir && !canon.isFile) continue
                out.add(
                    StorageEntry(
                        name = name,
                        size = if (isDir) 0uL else canon.length().coerceAtLeast(0L).toULong(),
                        modified = canon.lastModified(),
                        isDir = isDir,
                    ),
                )
            }
            return out
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        return listSaf(folders, path)
    }

    /** Opens `path` for ranged reading (`storage.read`). */
    fun openRead(path: String): StorageReadFile {
        if (!isValidPath(path)) {
            throw StorageFailure.Invalid("invalid file path")
        }
        if (hasAllFilesAccess(context)) {
            val (root, rel) = locate(removableVolumes(), path)
            val file = resolveExistingInRoot(root, blockedDirs(context, root), rel, allowRoot = false)
            if (!file.isFile) throw StorageFailure.NotFound()
            return StorageReadFile(
                source = FileToSend.Path(
                    path = file.absolutePath,
                    name = file.name,
                    folder = null,
                ),
                size = file.length().coerceAtLeast(0L).toULong(),
                modified = file.lastModified(),
            )
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        return openReadSaf(folders, path)
    }

    /** Commits a staged upload at `stagedPath` into `path` (`storage.write`). */
    fun write(path: String, stagedPath: String, modified: Long?): StorageWriteDone {
        if (!isValidPath(path)) {
            throw StorageFailure.Invalid("invalid destination path")
        }
        val staged = File(stagedPath)
        if (!staged.isFile) {
            throw StorageFailure.Failed("missing staged file")
        }
        if (hasAllFilesAccess(context)) {
            val (root, rel) = locate(removableVolumes(), path)
            val blocked = blockedDirs(context, root)
            val dest = resolveTargetInRoot(root, blocked, rel)
            val parent = dest.parentFile ?: throw StorageFailure.Denied()
            if (!parent.exists() && !parent.mkdirs() && !parent.isDirectory) {
                throw StorageFailure.Failed("cannot create parent folder")
            }
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            val canonParent = runCatching { parent.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (!isInsideRoot(canonRoot, blocked, canonParent)) {
                throw StorageFailure.Denied()
            }
            moveOrCopy(staged, dest)
            if (modified != null && modified >= 0L) {
                dest.setLastModified(modified)
            }
            MediaScannerConnection.scanFile(context, arrayOf(dest.absolutePath), null, null)
            return StorageWriteDone(
                size = dest.length().coerceAtLeast(0L).toULong(),
                modified = modified ?: dest.lastModified(),
            )
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        return writeSaf(folders, path, staged, modified)
    }

    /** Creates a directory (and any missing parents) at `path` (`storage.mkdir`). */
    fun mkdir(path: String) {
        if (!isValidPath(path)) {
            throw StorageFailure.Invalid("invalid folder path")
        }
        if (hasAllFilesAccess(context)) {
            val (root, rel) = locate(removableVolumes(), path)
            val blocked = blockedDirs(context, root)
            val dest = resolveTargetInRoot(root, blocked, rel)
            if (!dest.exists() && !dest.mkdirs() && !dest.isDirectory) {
                throw StorageFailure.Failed("cannot create folder")
            }
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            val canon = runCatching { dest.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (!isInsideRoot(canonRoot, blocked, canon)) {
                throw StorageFailure.Denied()
            }
            return
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        mkdirSaf(folders, path)
    }

    /** Renames or moves `from` to `to` (`storage.rename`). */
    fun rename(from: String, to: String) {
        if (!isValidPath(from) || !isValidPath(to)) {
            throw StorageFailure.Invalid("invalid rename path")
        }
        if (hasAllFilesAccess(context)) {
            val volumes = removableVolumes()
            val (root, fromRel) = locate(volumes, from)
            val (toRoot, toRel) = locate(volumes, to)
            // A rename can't cross from one storage to another.
            if (toRoot != root) throw StorageFailure.Failed("can't move between the phone's storage and an SD card")
            val blocked = blockedDirs(context, root)
            val src = resolveEntryInRoot(root, blocked, fromRel)
            val srcCanon = runCatching { src.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (containsBlockedDir(srcCanon, blocked)) {
                throw StorageFailure.Denied()
            }
            val dst = resolveTargetInRoot(root, blocked, toRel)
            val parent = dst.parentFile ?: throw StorageFailure.Denied()
            if (!parent.exists() && !parent.mkdirs() && !parent.isDirectory) {
                throw StorageFailure.Failed("cannot create destination parent")
            }
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            val canonParent = runCatching { parent.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (!isInsideRoot(canonRoot, blocked, canonParent)) {
                throw StorageFailure.Denied()
            }
            // Never replace another file (renameTo would); a change of letter case only is fine.
            if ((dst.exists() || isSymlink(dst)) && !src.absolutePath.equals(dst.absolutePath, ignoreCase = true)) {
                throw StorageFailure.Failed("a file or folder with that name already exists")
            }
            if (!src.renameTo(dst)) {
                throw StorageFailure.Failed("rename failed")
            }
            MediaScannerConnection.scanFile(
                context,
                arrayOf(src.absolutePath, dst.absolutePath),
                null,
                null,
            )
            return
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        renameSaf(folders, from, to)
    }

    /**
     * Deletes `path` (`storage.delete`).
     *
     * Media files indexed by Android's `MediaStore` on Android 11+ are moved to
     * the system trash (`IS_TRASHED = 1`) when possible; otherwise deletion
     * requires `confirmed = true` from the PC.
     */
    fun delete(path: String, confirmed: Boolean) {
        if (!isValidPath(path)) {
            throw StorageFailure.Invalid("invalid delete path")
        }
        if (hasAllFilesAccess(context)) {
            val (root, rel) = locate(removableVolumes(), path)
            val blocked = blockedDirs(context, root)
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            val target = resolveEntryInRoot(root, blocked, rel)
            val targetCanon = runCatching { target.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (containsBlockedDir(targetCanon, blocked)) {
                throw StorageFailure.Denied()
            }
            if (!isSymlink(target) && target.isFile && isMediaFile(target) && moveToMediaStoreTrash(target)) {
                return
            }
            if (!confirmed) {
                throw StorageFailure.Denied()
            }
            deleteEntry(target, canonRoot, blocked)
            MediaScannerConnection.scanFile(context, arrayOf(target.absolutePath), null, null)
            return
        }
        val folders = safFolders(context)
        if (folders.isEmpty()) throw StorageFailure.Unsupported()
        if (!confirmed) throw StorageFailure.Denied()
        deleteSaf(folders, path)
    }

    /**
     * Mounted SD cards and USB drives, each with its name at the root. A name
     * that a folder in internal storage already has gets a number after it.
     */
    private fun removableVolumes(): List<Volume> {
        val manager = context.getSystemService(StorageManager::class.java) ?: return emptyList()
        val internal = sharedRoot()
        val found = runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                manager.storageVolumes.mapNotNull { volume -> volume.directory?.let { volume to it } }
            } else {
                // Before Android 11 a volume has no directory of its own: take
                // it from this app's folder on the volume.
                context.getExternalFilesDirs(null).filterNotNull().mapNotNull { appDir ->
                    val dir = appDir.path.substringBefore("/Android/data/", missingDelimiterValue = "")
                    if (dir.isEmpty()) return@mapNotNull null
                    manager.getStorageVolume(File(dir))?.let { it to File(dir) }
                }
            }
        }.getOrDefault(emptyList())
        val removable = found.filter { (volume, _) -> volume.isRemovable && volume.isReadable() }
        return volumeNames(removable.map { (volume, dir) -> volume.getDescription(context) to dir }) { name ->
            File(internal, name).exists()
        }
    }

    private fun StorageVolume.isReadable(): Boolean =
        state == Environment.MEDIA_MOUNTED || state == Environment.MEDIA_MOUNTED_READ_ONLY

    @Synchronized
    fun stopWatching() {
        for (obs in observers.values) {
            obs.stopWatching()
        }
        observers.clear()
    }

    @Synchronized
    private fun watchDirectory(relPath: String, dir: File) {
        if (observers.containsKey(relPath)) return
        if (observers.size >= MAX_WATCHED_DIRS) {
            val eldest = observers.entries.iterator().next()
            eldest.value.stopWatching()
            observers.remove(eldest.key)
        }
        val mask = FileObserver.CREATE or
            FileObserver.DELETE or
            FileObserver.MOVED_FROM or
            FileObserver.MOVED_TO or
            FileObserver.CLOSE_WRITE
        @Suppress("DEPRECATION")
        val observer = object : FileObserver(dir.absolutePath, mask) {
            override fun onEvent(event: Int, file: String?) {
                if (file != null && file.startsWith(".nectarlink")) return
                onChanged(relPath)
            }
        }
        runCatching {
            observer.startWatching()
            observers[relPath] = observer
        }
    }

    private fun moveToMediaStoreTrash(file: File): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        return runCatching {
            val volume = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
            val projection = arrayOf(MediaStore.Files.FileColumns._ID)
            @Suppress("DEPRECATION")
            val selection = "${MediaStore.MediaColumns.DATA} = ?"
            val args = arrayOf(file.absolutePath)
            val id = context.contentResolver.query(volume, projection, selection, args, null)?.use { c ->
                if (c.moveToFirst()) c.getLong(0) else null
            } ?: return false
            val itemUri = ContentUris.withAppendedId(volume, id)
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.IS_TRASHED, 1)
            }
            context.contentResolver.update(itemUri, values, null, null) > 0
        }.getOrDefault(false)
    }

    private fun moveOrCopy(staged: File, dest: File) {
        if (dest.exists() && dest.isDirectory) {
            throw StorageFailure.Invalid("destination is a directory")
        }
        if (staged.renameTo(dest)) return
        try {
            FileInputStream(staged).use { input ->
                FileOutputStream(dest).use { output ->
                    input.copyTo(output)
                    output.fd.sync()
                }
            }
            staged.delete()
        } catch (e: Exception) {
            throw StorageFailure.Failed(e.message ?: "write failed")
        }
    }

    // ---- SAF (Storage Access Framework) fallback ----

    private fun splitSafPath(folders: List<SafFolder>, path: String): Pair<SafFolder, List<String>> {
        val parts = path.split('/')
        val rootName = parts.first()
        val folder = folders.firstOrNull { it.name == rootName } ?: throw StorageFailure.NotFound()
        return folder to parts.drop(1)
    }

    private fun resolveSafDocId(treeUri: Uri, segments: List<String>): String {
        var currentDocId = DocumentsContract.getTreeDocumentId(treeUri)
        for (seg in segments) {
            currentDocId = findChildSafDoc(treeUri, currentDocId, seg)?.docId
                ?: throw StorageFailure.NotFound()
        }
        return currentDocId
    }

    private data class SafDocMeta(
        val docId: String,
        val name: String,
        val mime: String,
        val size: Long,
        val modified: Long,
    ) {
        val isDir: Boolean get() = mime == DocumentsContract.Document.MIME_TYPE_DIR
    }

    private fun listSafChildren(treeUri: Uri, parentDocId: String): List<SafDocMeta> {
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        val result = ArrayList<SafDocMeta>()
        context.contentResolver.query(childrenUri, projection, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getString(0) ?: continue
                val name = c.getString(1) ?: continue
                val mime = c.getString(2) ?: "application/octet-stream"
                val size = if (c.isNull(3)) 0L else c.getLong(3)
                val modified = if (c.isNull(4)) 0L else c.getLong(4)
                if (isValidName(name) && !name.startsWith(".nectarlink")) {
                    result.add(SafDocMeta(id, name, mime, size, modified))
                }
            }
        } ?: throw StorageFailure.Denied()
        return result
    }

    private fun findChildSafDoc(treeUri: Uri, parentDocId: String, name: String): SafDocMeta? =
        listSafChildren(treeUri, parentDocId).firstOrNull { it.name == name }

    private fun querySafDocMeta(treeUri: Uri, docId: String): SafDocMeta {
        val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        context.contentResolver.query(docUri, projection, null, null, null)?.use { c ->
            if (c.moveToFirst()) {
                val name = c.getString(0) ?: ""
                val mime = c.getString(1) ?: "application/octet-stream"
                val size = if (c.isNull(2)) 0L else c.getLong(2)
                val modified = if (c.isNull(3)) 0L else c.getLong(3)
                return SafDocMeta(docId, name, mime, size, modified)
            }
        }
        throw StorageFailure.NotFound()
    }

    private fun listSaf(folders: List<SafFolder>, path: String): List<StorageEntry> {
        if (path.isEmpty()) {
            return folders.map {
                StorageEntry(name = it.name, size = 0uL, modified = 0L, isDir = true)
            }
        }
        val (folder, sub) = splitSafPath(folders, path)
        val docId = resolveSafDocId(folder.uri, sub)
        return listSafChildren(folder.uri, docId).map {
            StorageEntry(
                name = it.name,
                size = if (it.isDir) 0uL else it.size.coerceAtLeast(0L).toULong(),
                modified = it.modified,
                isDir = it.isDir,
            )
        }
    }

    private fun openReadSaf(folders: List<SafFolder>, path: String): StorageReadFile {
        val (folder, sub) = splitSafPath(folders, path)
        if (sub.isEmpty()) throw StorageFailure.NotFound()
        val docId = resolveSafDocId(folder.uri, sub)
        val meta = querySafDocMeta(folder.uri, docId)
        if (meta.isDir) throw StorageFailure.NotFound()
        val docUri = DocumentsContract.buildDocumentUriUsingTree(folder.uri, docId)
        val pfd = context.contentResolver.openFileDescriptor(docUri, "r")
            ?: throw StorageFailure.NotFound()
        val fd = pfd.detachFd()
        return StorageReadFile(
            source = FileToSend.Fd(fd = fd, name = meta.name, folder = null),
            size = meta.size.coerceAtLeast(0L).toULong(),
            modified = meta.modified,
        )
    }

    private fun ensureSafDir(treeUri: Uri, segments: List<String>): String {
        var currentDocId = DocumentsContract.getTreeDocumentId(treeUri)
        for (seg in segments) {
            val existing = findChildSafDoc(treeUri, currentDocId, seg)
            currentDocId = if (existing != null) {
                if (!existing.isDir) throw StorageFailure.Invalid("path component is a file")
                existing.docId
            } else {
                val parentUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, currentDocId)
                val created = DocumentsContract.createDocument(
                    context.contentResolver,
                    parentUri,
                    DocumentsContract.Document.MIME_TYPE_DIR,
                    seg,
                ) ?: throw StorageFailure.Failed("cannot create SAF directory")
                DocumentsContract.getDocumentId(created)
            }
        }
        return currentDocId
    }

    private fun writeSaf(
        folders: List<SafFolder>,
        path: String,
        staged: File,
        modified: Long?,
    ): StorageWriteDone {
        val (folder, sub) = splitSafPath(folders, path)
        if (sub.isEmpty()) throw StorageFailure.Invalid("cannot overwrite root folder")
        val parentDocId = ensureSafDir(folder.uri, sub.dropLast(1))
        val fileName = sub.last()
        val existing = findChildSafDoc(folder.uri, parentDocId, fileName)
        val targetUri = if (existing != null) {
            if (existing.isDir) throw StorageFailure.Invalid("destination is a directory")
            DocumentsContract.buildDocumentUriUsingTree(folder.uri, existing.docId)
        } else {
            val parentUri = DocumentsContract.buildDocumentUriUsingTree(folder.uri, parentDocId)
            DocumentsContract.createDocument(
                context.contentResolver,
                parentUri,
                "application/octet-stream",
                fileName,
            ) ?: throw StorageFailure.Failed("cannot create SAF file")
        }
        try {
            FileInputStream(staged).use { input ->
                context.contentResolver.openOutputStream(targetUri, "wt")?.use { output ->
                    input.copyTo(output)
                    output.flush()
                } ?: throw StorageFailure.Failed("cannot open SAF output stream")
            }
        } catch (e: StorageFailure) {
            throw e
        } catch (e: Exception) {
            throw StorageFailure.Failed(e.message ?: "SAF write failed")
        }
        val size = staged.length().coerceAtLeast(0L).toULong()
        return StorageWriteDone(
            size = size,
            modified = modified ?: System.currentTimeMillis(),
        )
    }

    private fun mkdirSaf(folders: List<SafFolder>, path: String) {
        val (folder, sub) = splitSafPath(folders, path)
        if (sub.isEmpty()) return
        ensureSafDir(folder.uri, sub)
    }

    private fun renameSaf(folders: List<SafFolder>, from: String, to: String) {
        val (srcFolder, srcSub) = splitSafPath(folders, from)
        val (dstFolder, dstSub) = splitSafPath(folders, to)
        if (srcFolder.uri != dstFolder.uri || srcSub.isEmpty() || dstSub.isEmpty()) {
            throw StorageFailure.Unsupported()
        }
        if (srcSub.dropLast(1) != dstSub.dropLast(1)) {
            throw StorageFailure.Unsupported()
        }
        val docId = resolveSafDocId(srcFolder.uri, srcSub)
        val docUri = DocumentsContract.buildDocumentUriUsingTree(srcFolder.uri, docId)
        DocumentsContract.renameDocument(context.contentResolver, docUri, dstSub.last())
            ?: throw StorageFailure.Failed("SAF rename failed")
    }

    private fun deleteSaf(folders: List<SafFolder>, path: String) {
        val (folder, sub) = splitSafPath(folders, path)
        if (sub.isEmpty()) throw StorageFailure.Denied()
        val docId = resolveSafDocId(folder.uri, sub)
        val docUri = DocumentsContract.buildDocumentUriUsingTree(folder.uri, docId)
        if (!DocumentsContract.deleteDocument(context.contentResolver, docUri)) {
            throw StorageFailure.Failed("SAF delete failed")
        }
    }

    data class SafFolder(val name: String, val uri: Uri)

    companion object {
        private const val PREFS_NAME = "nectarlink_storage_saf"
        private const val KEY_TREES = "trees"
        private const val MAX_WATCHED_DIRS = 16
        const val TOGGLE_NAME = "storage"

        private val MEDIA_EXTENSIONS = setOf(
            "jpg", "jpeg", "png", "webp", "gif", "heic", "heif",
            "mp4", "mov", "mkv", "webm", "avi",
            "mp3", "m4a", "aac", "wav", "flac", "ogg", "opus",
        )

        fun sharedRoot(): File = Environment.getExternalStorageDirectory()

        /** Whether full shared-storage access (`MANAGE_EXTERNAL_STORAGE` on Android 11+) is granted. */
        fun hasAllFilesAccess(context: Context): Boolean =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                Environment.isExternalStorageManager()
            } else {
                ContextCompat.checkSelfPermission(
                    context,
                    Manifest.permission.READ_EXTERNAL_STORAGE,
                ) == PackageManager.PERMISSION_GRANTED &&
                    ContextCompat.checkSelfPermission(
                        context,
                        Manifest.permission.WRITE_EXTERNAL_STORAGE,
                    ) == PackageManager.PERMISSION_GRANTED
            }

        /** Whether this phone can serve storage right now (All files access or at least one SAF folder). */
        fun hasAnyAccess(context: Context): Boolean =
            hasAllFilesAccess(context) || safFolders(context).isNotEmpty()

        /** Capabilities offered by this phone's storage service when access is granted. */
        fun capabilities(context: Context): List<String> =
            if (hasAnyAccess(context)) listOf("storage.read", "storage.write") else emptyList()

        /** Intent to open Android's "All files access" settings screen for NectarLink. */
        fun allFilesAccessIntent(context: Context): Intent {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val perApp = Intent(
                    Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                    Uri.fromParts("package", context.packageName, null),
                ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                if (perApp.resolveActivity(context.packageManager) != null) {
                    return perApp
                }
                return Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            return Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", context.packageName, null),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }

        /** Persists a folder picked via `ACTION_OPEN_DOCUMENT_TREE`. */
        fun addSafFolder(context: Context, treeUri: Uri) {
            val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            runCatching { context.contentResolver.takePersistableUriPermission(treeUri, flags) }
            val rawName = runCatching {
                val docId = DocumentsContract.getTreeDocumentId(treeUri)
                docId.substringAfterLast(':').substringAfterLast('/').ifEmpty { "Folder" }
            }.getOrDefault("Folder")
            val cleanName = rawName.filter { it != '/' && it != '\\' && it >= ' ' }.trim().ifEmpty { "Folder" }
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            val current = prefs.getStringSet(KEY_TREES, emptySet()).orEmpty().toMutableSet()
            current.removeAll { it.substringAfter('|') == treeUri.toString() }
            var uniqueName = cleanName
            var suffix = 2
            val existingNames = current.map { it.substringBefore('|') }.toSet()
            while (uniqueName in existingNames) {
                uniqueName = "$cleanName ($suffix)"
                suffix++
            }
            current.add("$uniqueName|$treeUri")
            prefs.edit().putStringSet(KEY_TREES, current).apply()
        }

        /** Removes a previously picked SAF folder. */
        fun removeSafFolder(context: Context, name: String) {
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            val current = prefs.getStringSet(KEY_TREES, emptySet()).orEmpty().toMutableSet()
            val removed = current.filter { it.substringBefore('|') == name }
            for (entry in removed) {
                val uri = runCatching { Uri.parse(entry.substringAfter('|')) }.getOrNull()
                if (uri != null) {
                    val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    runCatching { context.contentResolver.releasePersistableUriPermission(uri, flags) }
                }
            }
            current.removeAll(removed.toSet())
            prefs.edit().putStringSet(KEY_TREES, current).apply()
        }

        /** Lists currently persisted SAF folders. */
        fun safFolders(context: Context): List<SafFolder> {
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            val persisted = runCatching {
                context.contentResolver.persistedUriPermissions
                    .filter { it.isReadPermission }
                    .map { it.uri }
                    .toSet()
            }.getOrNull()
            return filterPersistedSafEntries(
                rawEntries = prefs.getStringSet(KEY_TREES, emptySet()).orEmpty(),
                persistedUris = persisted,
                parseUri = { raw -> runCatching { Uri.parse(raw) }.getOrNull() },
            ).map { (name, uri) -> SafFolder(name, uri) }
        }

        internal fun <U> filterPersistedSafEntries(
            rawEntries: Set<String>,
            persistedUris: Set<U>?,
            parseUri: (String) -> U?,
        ): List<Pair<String, U>> = rawEntries
            .mapNotNull { entry ->
                val idx = entry.indexOf('|')
                if (idx <= 0) return@mapNotNull null
                val name = entry.substring(0, idx)
                val uri = parseUri(entry.substring(idx + 1)) ?: return@mapNotNull null
                if (persistedUris != null && uri !in persistedUris) return@mapNotNull null
                name to uri
            }
            .sortedBy { it.first.lowercase() }

        /**
         * Names for removable volumes at the root, from their descriptions ("SD
         * card"): unique, and different from every name `taken` reports.
         */
        internal fun volumeNames(volumes: List<Pair<String, File>>, taken: (String) -> Boolean): List<Volume> {
            val used = HashSet<String>()
            return volumes.map { (description, dir) ->
                val base = description.filter { it != '/' && it != '\\' && !it.isISOControl() }
                    .trim()
                    .take(64)
                    .ifEmpty { "SD card" }
                var name = base
                var n = 2
                while (!isValidName(name) || name in used || taken(name)) {
                    name = "$base ($n)"
                    n++
                }
                used += name
                Volume(name, dir)
            }
        }

        /**
         * Where a protocol path lives: on the removable volume its first
         * segment names, at the rest of the path, or else in internal storage.
         */
        internal fun locate(volumes: List<Volume>, path: String, internal: File = sharedRoot()): Pair<File, String> {
            val first = path.substringBefore('/')
            val volume = volumes.firstOrNull { it.name == first } ?: return internal to path
            return volume.dir to path.substringAfter('/', missingDelimiterValue = "")
        }

        internal fun blockedDirs(context: Context, root: File): List<File> = buildList {
            add(File("/data"))
            add(File(root, "Android/data"))
            add(File(root, "Android/obb"))
            context.dataDir?.let { add(it) }
            context.filesDir?.let { add(it) }
            context.noBackupFilesDir?.let { add(it) }
            context.cacheDir?.let { add(it) }
            context.externalCacheDir?.let { add(it) }
            context.getExternalFilesDir(null)?.let { add(it) }
        }

        internal fun isMediaFile(file: File): Boolean =
            file.extension.lowercase() in MEDIA_EXTENSIONS

        /** Validates a single path segment (no `/`, `\`, `..`, `.`, NUL, or control chars). */
        internal fun isValidName(name: String): Boolean {
            if (name.isEmpty() || name.length > 255 || name == "." || name == "..") return false
            return name.all { c -> c != '/' && c != '\\' && c != '\u0000' && !c.isISOControl() }
        }

        /** Validates a non-empty relative path (`a/b/c`). */
        internal fun isValidPath(path: String): Boolean {
            if (path.isEmpty() || path.length > 1024 || path.startsWith('/') || path.endsWith('/')) {
                return false
            }
            return path.split('/').all(::isValidName)
        }

        /** Validates a directory path (`""` for the root or a valid relative path). */
        internal fun isValidDirPath(path: String): Boolean =
            path.isEmpty() || isValidPath(path)

        internal fun isInsideRoot(canonRoot: File, blocked: List<File>, candidate: File): Boolean {
            val rootPath = canonRoot.path
            val candPath = candidate.path
            val underRoot = candPath == rootPath ||
                candPath.startsWith(if (rootPath.endsWith(File.separator)) rootPath else "$rootPath${File.separator}")
            if (!underRoot) return false
            for (b in blocked) {
                val bp = runCatching { b.canonicalPath }.getOrDefault(b.absolutePath)
                if (candPath == bp || candPath.startsWith("$bp${File.separator}")) {
                    return false
                }
            }
            return true
        }

        internal fun isSymlink(file: File): Boolean =
            runCatching { Files.isSymbolicLink(file.toPath()) }.getOrDefault(false)

        /** True when `candidate` is or contains any directory in `blocked` (e.g. `Android` contains `Android/data`). */
        internal fun containsBlockedDir(candidate: File, blocked: List<File>): Boolean {
            val candPath = runCatching { candidate.canonicalPath }.getOrDefault(candidate.absolutePath)
            val prefix = if (candPath.endsWith(File.separator)) candPath else "$candPath${File.separator}"
            for (b in blocked) {
                val bp = runCatching { b.canonicalPath }.getOrDefault(b.absolutePath)
                if (bp == candPath || bp.startsWith(prefix)) {
                    return true
                }
            }
            return false
        }

        /**
         * Resolves an entry inside `root` for `delete` or `rename`, canonicalizing
         * parent segments while preserving the final segment as the link itself if
         * it is a symbolic link (after verifying its target is also inside `root`).
         */
        internal fun resolveEntryInRoot(
            root: File,
            blocked: List<File>,
            rel: String,
        ): File {
            if (!isValidPath(rel)) throw StorageFailure.Invalid("invalid storage path")
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            if (!isInsideRoot(canonRoot, blocked, canonRoot)) throw StorageFailure.Denied()

            val segments = rel.split('/')
            var cur = canonRoot
            for (i in 0 until segments.size - 1) {
                cur = File(cur, segments[i])
                if (!cur.exists() && !isSymlink(cur)) {
                    throw StorageFailure.NotFound()
                }
                val target = runCatching { cur.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
                if (!isInsideRoot(canonRoot, blocked, target)) {
                    throw StorageFailure.Denied()
                }
                cur = target
            }
            val leaf = File(cur, segments.last())
            if (!leaf.exists() && !isSymlink(leaf)) {
                throw StorageFailure.NotFound()
            }
            val leafTarget = runCatching { leaf.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
            if (!isInsideRoot(canonRoot, blocked, leafTarget)) {
                throw StorageFailure.Denied()
            }
            return if (isSymlink(leaf)) leaf else leafTarget
        }

        /**
         * Deletes `file` recursively without following symbolic links into target
         * directories and without deleting any ancestor of a blocked directory.
         */
        /**
         * Deletes [target] for the PC. Not there to begin with is [StorageFailure.NotFound],
         * which the PC takes as done; and once it's gone it's deleted, even if a step
         * along the way said otherwise. Only a target still there is a failure.
         */
        internal fun deleteEntry(target: File, canonRoot: File, blocked: List<File>) {
            if (!target.exists() && !isSymlink(target)) throw StorageFailure.NotFound()
            if (!deleteTreeSafely(target, canonRoot, blocked) && (target.exists() || isSymlink(target))) {
                throw StorageFailure.Failed("delete failed")
            }
        }

        internal fun deleteTreeSafely(file: File, canonRoot: File, blocked: List<File>): Boolean {
            if (isSymlink(file)) {
                return runCatching { Files.deleteIfExists(file.toPath()) }.getOrDefault(false) || file.delete()
            }
            val canon = runCatching { file.canonicalFile }.getOrNull() ?: return false
            if (!isInsideRoot(canonRoot, blocked, canon) || containsBlockedDir(canon, blocked)) {
                return false
            }
            if (file.isDirectory) {
                val children = file.listFiles() ?: return false
                for (child in children) {
                    if (!deleteTreeSafely(child, canonRoot, blocked)) {
                        return false
                    }
                }
            }
            return runCatching { Files.deleteIfExists(file.toPath()) }.getOrDefault(false) || file.delete()
        }

        /**
         * Resolves an existing path inside `root`, rejecting `..`, absolute
         * paths, blocked directories, and symlinks pointing outside `root`.
         */
        internal fun resolveExistingInRoot(
            root: File,
            blocked: List<File>,
            rel: String,
            allowRoot: Boolean,
        ): File {
            val valid = if (allowRoot) isValidDirPath(rel) else isValidPath(rel)
            if (!valid) throw StorageFailure.Invalid("invalid storage path")
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            if (!isInsideRoot(canonRoot, blocked, canonRoot)) throw StorageFailure.Denied()
            if (rel.isEmpty()) return canonRoot

            var cur = canonRoot
            for (seg in rel.split('/')) {
                cur = File(cur, seg)
                if (!cur.exists() && !isSymlink(cur)) {
                    throw StorageFailure.NotFound()
                }
                val target = runCatching { cur.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
                if (!isInsideRoot(canonRoot, blocked, target)) {
                    throw StorageFailure.Denied()
                }
                cur = target
            }
            val finalCanon = runCatching { cur.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            if (!finalCanon.exists()) throw StorageFailure.NotFound()
            if (!isInsideRoot(canonRoot, blocked, finalCanon)) throw StorageFailure.Denied()
            return finalCanon
        }

        /**
         * Resolves a target path (which may not exist yet) inside `root`,
         * verifying that every existing ancestor stays inside `root` and is
         * not a symlink escaping `root` or entering a blocked directory.
         */
        internal fun resolveTargetInRoot(
            root: File,
            blocked: List<File>,
            rel: String,
        ): File {
            if (!isValidPath(rel)) throw StorageFailure.Invalid("invalid storage path")
            val canonRoot = runCatching { root.canonicalFile }.getOrElse { throw StorageFailure.NotFound() }
            if (!isInsideRoot(canonRoot, blocked, canonRoot)) throw StorageFailure.Denied()

            var cur = canonRoot
            for (seg in rel.split('/')) {
                cur = File(cur, seg)
                if (cur.exists() || isSymlink(cur)) {
                    val target = runCatching { cur.canonicalFile }.getOrElse { throw StorageFailure.Denied() }
                    if (!isInsideRoot(canonRoot, blocked, target)) {
                        throw StorageFailure.Denied()
                    }
                    cur = target
                } else {
                    val absCheck = File(cur.absolutePath)
                    if (!isInsideRoot(canonRoot, blocked, absCheck)) {
                        throw StorageFailure.Denied()
                    }
                }
            }
            return cur
        }
    }
}

/** An SD card or USB drive, shown as the folder `name` at the root. */
internal data class Volume(val name: String, val dir: File)
