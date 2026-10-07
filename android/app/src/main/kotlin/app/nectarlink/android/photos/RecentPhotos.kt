// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.photos

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.database.Cursor
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import android.util.Size
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import app.nectarlink.core.FileToSend
import app.nectarlink.core.Photo
import app.nectarlink.core.PhotoAlbum
import app.nectarlink.core.PhotoItem
import app.nectarlink.core.PhotoThumb
import java.io.ByteArrayOutputStream

/**
 * Photos, screenshots and videos for PCs (docs/protocol/photos.md): watches
 * the phone's gallery while the app runs, reports each new photo with a small
 * preview, notifies PCs when the library changes, and answers gallery album,
 * item, thumbnail, and full-file queries.
 */
internal class RecentPhotos(
    context: Context,
    private val onPhoto: (Photo) -> Unit,
    private val onChanged: () -> Unit = {},
) {
    private val context = context.applicationContext
    private val prefs = context.getSharedPreferences("photos", Context.MODE_PRIVATE)
    private val handler = Handler(Looper.getMainLooper())
    private var observer: ContentObserver? = null
    private val check = Runnable { Thread { checkNow() }.start() }

    /** Starts watching (when allowed); only photos from now on are new. */
    fun start() {
        if (observer != null || !hasAccess(context)) return
        if (!prefs.contains(LAST_SEEN)) prefs.edit { putLong(LAST_SEEN, System.currentTimeMillis() / 1000) }
        observer = object : ContentObserver(handler) {
            // A new picture changes several rows (pending, then done): wait
            // for it to settle.
            override fun onChange(selfChange: Boolean) {
                handler.removeCallbacks(check)
                handler.postDelayed(check, SETTLE_MS)
            }
        }.also {
            context.contentResolver.registerContentObserver(IMAGES_URI, true, it)
            context.contentResolver.registerContentObserver(VIDEOS_URI, true, it)
        }
    }

    fun stop() {
        observer?.let { context.contentResolver.unregisterContentObserver(it) }
        observer = null
        handler.removeCallbacks(check)
    }

    @Synchronized
    private fun checkNow() {
        onChanged()
        val lastSeen = prefs.getLong(LAST_SEEN, System.currentTimeMillis() / 1000)
        var newest = lastSeen
        val columns = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.SIZE,
            MediaStore.Images.Media.DATE_ADDED,
            MediaStore.Images.ImageColumns.DATE_TAKEN,
            MediaStore.Images.ImageColumns.BUCKET_DISPLAY_NAME,
        )
        val pending = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) " AND ${MediaStore.Images.Media.IS_PENDING} = 0" else ""
        val found = runCatching {
            context.contentResolver.query(
                IMAGES_URI,
                columns,
                "${MediaStore.Images.Media.DATE_ADDED} > ?$pending",
                arrayOf(lastSeen.toString()),
                "${MediaStore.Images.Media.DATE_ADDED} ASC",
            )?.use { cursor ->
                buildList {
                    while (cursor.moveToNext()) {
                        val added = cursor.getLong(3)
                        newest = maxOf(newest, added)
                        val album = cursor.getString(5).orEmpty()
                        add(
                            Found(
                                id = cursor.getLong(0),
                                name = cursor.getString(1) ?: "photo.jpg",
                                size = cursor.getLong(2),
                                taken = cursor.getLong(4).takeIf { it > 0 }?.div(1000) ?: added,
                                screenshot = album.contains("Screenshot", ignoreCase = true),
                            ),
                        )
                    }
                }
            }.orEmpty()
        }.getOrElse {
            Log.w(TAG, "can't look for new photos", it)
            return
        }
        prefs.edit { putLong(LAST_SEEN, newest) }
        // A burst (a camera's burst mode, a downloaded album) isn't news
        // photo by photo: only the latest few.
        for (photo in found.takeLast(MAX_PER_CHECK)) {
            val thumb = encodeThumbnail(context, IMAGES_URI, photo.id, isVideo = false, PREVIEW_PX, MAX_PREVIEW_BYTES) ?: continue
            onPhoto(Photo(IMAGE_PREFIX + photo.id, photo.name, photo.size.coerceAtLeast(0L).toULong(), photo.taken, photo.screenshot, thumb))
        }
    }

    private data class Found(val id: Long, val name: String, val size: Long, val taken: Long, val screenshot: Boolean)

    companion object {
        private const val TAG = "RecentPhotos"
        private const val LAST_SEEN = "lastSeen"
        private const val IMAGE_PREFIX = "media:"
        private const val VIDEO_PREFIX = "video:"
        private const val SETTLE_MS = 1500L
        private const val MAX_PER_CHECK = 3
        private const val PREVIEW_PX = 512
        private const val MAX_PREVIEW_BYTES = 96 * 1024
        private const val THUMB_PX = 256
        private const val MAX_THUMB_BYTES = 32 * 1024
        private const val MAX_BATCH = 24
        private val IMAGES_URI: Uri = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        private val VIDEOS_URI: Uri = MediaStore.Video.Media.EXTERNAL_CONTENT_URI

        /** The gallery's order: when taken, or else when last written (as [dateOf]). */
        private val DATE_KEY =
            "COALESCE(${MediaStore.MediaColumns.DATE_TAKEN}, ${MediaStore.MediaColumns.DATE_MODIFIED} * 1000)"

        /** Permissions to request from the user for photos and videos. */
        val permissions: Array<String> = when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE -> arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
                Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
            )
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU -> arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
            )
            else -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }

        /** Whether the user granted access to all photos and videos. */
        fun hasFullAccess(context: Context): Boolean =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_IMAGES) == PackageManager.PERMISSION_GRANTED &&
                    ContextCompat.checkSelfPermission(context, Manifest.permission.READ_MEDIA_VIDEO) == PackageManager.PERMISSION_GRANTED
            } else {
                ContextCompat.checkSelfPermission(context, Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED
            }

        /**
         * Whether the app sees some photos or videos: all photos (what
         * versions before the gallery asked for), all videos, or the ones
         * the user picked (Android 14's "Select photos and videos").
         */
        fun hasAccess(context: Context): Boolean = hasFullAccess(context) || listOfNotNull(
            Manifest.permission.READ_MEDIA_IMAGES.takeIf { Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU },
            Manifest.permission.READ_MEDIA_VIDEO.takeIf { Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU },
            Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED.takeIf {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE
            },
        ).any { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

        /** Whether it sees only some of them: the user can allow the rest. */
        fun hasPartialAccess(context: Context): Boolean = hasAccess(context) && !hasFullAccess(context)

        /** Lists photo and video albums (MediaStore buckets), Camera and Screenshots first. */
        fun albums(context: Context): List<PhotoAlbum> {
            if (!hasAccess(context)) return emptyList()
            class Bucket(val id: String, val name: String) {
                var count = 0
                var newest = Long.MIN_VALUE to Long.MIN_VALUE
                var cover: String? = null
            }
            val buckets = LinkedHashMap<String, Bucket>()
            for ((uri, prefix) in listOf(IMAGES_URI to IMAGE_PREFIX, VIDEOS_URI to VIDEO_PREFIX)) {
                runCatching {
                    context.contentResolver.query(
                        uri,
                        arrayOf(
                            MediaStore.MediaColumns._ID,
                            MediaStore.MediaColumns.BUCKET_ID,
                            MediaStore.MediaColumns.BUCKET_DISPLAY_NAME,
                            MediaStore.MediaColumns.DATE_TAKEN,
                            MediaStore.MediaColumns.DATE_MODIFIED,
                        ),
                        notPending(),
                        null,
                        null,
                    )?.use { cursor ->
                        while (cursor.moveToNext()) {
                            val id = cursor.getLong(0)
                            val bucketId = cursor.getString(1)?.takeIf { it.isNotBlank() } ?: continue
                            val bucket = buckets.getOrPut(bucketId) {
                                Bucket(bucketId, cursor.getString(2)?.takeIf { it.isNotBlank() } ?: "Photos")
                            }
                            bucket.count += 1
                            val key = dateOf(cursor, 3, 4) to id
                            if (compareValuesBy(key, bucket.newest, { it.first }, { it.second }) > 0) {
                                bucket.newest = key
                                bucket.cover = prefix + id
                            }
                        }
                    }
                }.onFailure { Log.w(TAG, "can't list albums", it) }
            }
            return buckets.values
                .sortedWith(
                    compareBy<Bucket> {
                        when {
                            it.name.equals("Camera", ignoreCase = true) -> 0
                            it.name.equals("Screenshots", ignoreCase = true) -> 1
                            else -> 2
                        }
                    }.thenByDescending { it.count }.thenBy { it.name.lowercase() },
                )
                .map { PhotoAlbum(id = it.id, name = it.name, count = it.count.toUInt(), cover = it.cover) }
        }

        /**
         * Lists photos and videos (in `album` when given), newest first by
         * when they were taken, then by media ID. With `before`, only those
         * after the previous page's last item (`before`, `beforeId`).
         */
        fun list(context: Context, album: String?, before: Long?, beforeId: String, limit: Int): List<PhotoItem> {
            if (!hasAccess(context) || limit <= 0) return emptyList()
            val max = limit.coerceIn(1, 200)
            // Images and videos share one ID space (MediaStore's files table).
            val lastId = parseId(beforeId)?.second
            val out = ArrayList<Pair<PhotoItem, Long>>(max * 2)
            for ((uri, prefix) in listOf(IMAGES_URI to IMAGE_PREFIX, VIDEOS_URI to VIDEO_PREFIX)) {
                val isVideo = prefix == VIDEO_PREFIX
                val columns = arrayOf(
                    MediaStore.MediaColumns._ID,
                    MediaStore.MediaColumns.DISPLAY_NAME,
                    MediaStore.MediaColumns.SIZE,
                    MediaStore.MediaColumns.DATE_TAKEN,
                    MediaStore.MediaColumns.DATE_MODIFIED,
                    MediaStore.MediaColumns.WIDTH,
                    MediaStore.MediaColumns.HEIGHT,
                    MediaStore.MediaColumns.BUCKET_ID,
                ) + if (isVideo) arrayOf(MediaStore.Video.VideoColumns.DURATION) else emptyArray()
                val clauses = listOfNotNull(notPending()).toMutableList()
                val args = mutableListOf<String>()
                if (!album.isNullOrBlank()) {
                    clauses += "${MediaStore.MediaColumns.BUCKET_ID} = ?"
                    args += album
                }
                // Arguments arrive as text, and SQLite orders any number before
                // any text unless told otherwise (the date has no column type).
                if (before != null) {
                    if (lastId != null) {
                        clauses += "($DATE_KEY < CAST(? AS INTEGER) OR ($DATE_KEY = CAST(? AS INTEGER) AND " +
                            "${MediaStore.MediaColumns._ID} < CAST(? AS INTEGER)))"
                        args += listOf(before.toString(), before.toString(), lastId.toString())
                    } else {
                        clauses += "$DATE_KEY < CAST(? AS INTEGER)"
                        args += before.toString()
                    }
                }
                runCatching {
                    context.contentResolver.query(
                        uri,
                        columns,
                        clauses.joinToString(" AND ").ifEmpty { null },
                        args.toTypedArray().ifEmpty { null },
                        "$DATE_KEY DESC, ${MediaStore.MediaColumns._ID} DESC",
                    )?.use { cursor ->
                        var count = 0
                        while (count < max && cursor.moveToNext()) {
                            val id = cursor.getLong(0)
                            val fallback = if (isVideo) "VID_$id.mp4" else "IMG_$id.jpg"
                            out += PhotoItem(
                                id = prefix + id,
                                name = cursor.getString(1)?.takeIf { it.isNotBlank() } ?: fallback,
                                date = dateOf(cursor, 3, 4),
                                size = cursor.getLong(2).coerceAtLeast(0L).toULong(),
                                width = cursor.getInt(5).coerceAtLeast(0).toUInt(),
                                height = cursor.getInt(6).coerceAtLeast(0).toUInt(),
                                duration = if (isVideo) cursor.getLong(8).coerceIn(0L, UInt.MAX_VALUE.toLong()).toUInt() else null,
                                album = cursor.getString(7)?.takeIf { it.isNotBlank() },
                            ) to id
                            count += 1
                        }
                    }
                }.onFailure { Log.w(TAG, "can't list gallery items", it) }
            }
            return out.sortedWith(compareByDescending<Pair<PhotoItem, Long>> { it.first.date }.thenByDescending { it.second })
                .take(max)
                .map { it.first }
        }

        /** Only finished files (Android 10+ marks ones still being written as pending). */
        private fun notPending(): String? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) "${MediaStore.MediaColumns.IS_PENDING} = 0" else null

        /** When it was taken (ms), or else when the file was last written; as [DATE_KEY]. */
        private fun dateOf(cursor: Cursor, taken: Int, modified: Int): Long =
            cursor.getLong(taken).takeIf { !cursor.isNull(taken) } ?: (cursor.getLong(modified) * 1000L)

        /** Loads small JPEG thumbnails (<= 32 KiB, ~256 px) for a batch of item IDs. */
        fun thumbs(context: Context, ids: List<String>): List<PhotoThumb> {
            if (!hasAccess(context) || ids.isEmpty()) return emptyList()
            return ids.take(MAX_BATCH).mapNotNull { rawId ->
                val (uri, mediaId, isVideo) = parseId(rawId) ?: return@mapNotNull null
                val bytes = encodeThumbnail(context, uri, mediaId, isVideo, THUMB_PX, MAX_THUMB_BYTES)
                    ?: return@mapNotNull null
                PhotoThumb(id = rawId, data = bytes)
            }
        }

        /** Opens a photo or video a PC asked for; `null` when it's gone or not one of ours. */
        fun open(context: Context, id: String): FileToSend? {
            if (!hasAccess(context)) return null
            val (collection, mediaId, isVideo) = parseId(id) ?: return null
            val uri = ContentUris.withAppendedId(collection, mediaId)
            val fallback = if (isVideo) "VID_$mediaId.mp4" else "IMG_$mediaId.jpg"
            val name = runCatching {
                context.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) it.getString(0) else null
                }
            }.getOrNull()?.takeIf { it.isNotBlank() } ?: fallback
            val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: return null
            return FileToSend.Fd(name = name, fd = fd.detachFd(), folder = null)
        }

        private fun parseId(id: String): Triple<Uri, Long, Boolean>? = when {
            id.startsWith(IMAGE_PREFIX) -> id.removePrefix(IMAGE_PREFIX).toLongOrNull()?.let { Triple(IMAGES_URI, it, false) }
            id.startsWith(VIDEO_PREFIX) -> id.removePrefix(VIDEO_PREFIX).toLongOrNull()?.let { Triple(VIDEOS_URI, it, true) }
            else -> null
        }

        private fun encodeThumbnail(
            context: Context,
            collection: Uri,
            mediaId: Long,
            isVideo: Boolean,
            targetPx: Int,
            maxBytes: Int,
        ): ByteArray? = runCatching {
            val uri = ContentUris.withAppendedId(collection, mediaId)
            val bitmap = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                context.contentResolver.loadThumbnail(uri, Size(targetPx, targetPx), null)
            } else {
                @Suppress("DEPRECATION")
                if (isVideo) {
                    MediaStore.Video.Thumbnails.getThumbnail(
                        context.contentResolver,
                        mediaId,
                        MediaStore.Video.Thumbnails.MINI_KIND,
                        null,
                    )
                } else {
                    MediaStore.Images.Thumbnails.getThumbnail(
                        context.contentResolver,
                        mediaId,
                        MediaStore.Images.Thumbnails.MINI_KIND,
                        null,
                    )
                }
            } ?: return null
            try {
                generateSequence(82) { it - 15 }.takeWhile { it >= 35 }
                    .map { quality ->
                        ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, quality, it) }.toByteArray()
                    }
                    .firstOrNull { it.size <= maxBytes }
            } finally {
                bitmap.recycle()
            }
        }.getOrElse {
            Log.i(TAG, "no thumbnail for media item", it)
            null
        }
    }
}

