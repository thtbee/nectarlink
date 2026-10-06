// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.photos

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
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
import java.io.ByteArrayOutputStream

/**
 * New photos and screenshots, for PCs (docs/protocol/photos.md): watches
 * the phone's pictures while the app runs and reports each new one with a
 * small preview. A PC then asks for the photo itself by its ID.
 */
internal class RecentPhotos(context: Context, private val onPhoto: (Photo) -> Unit) {
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
        }.also { context.contentResolver.registerContentObserver(COLLECTION, true, it) }
    }

    fun stop() {
        observer?.let { context.contentResolver.unregisterContentObserver(it) }
        observer = null
        handler.removeCallbacks(check)
    }

    @Synchronized
    private fun checkNow() {
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
                COLLECTION,
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
            val thumb = preview(photo.id) ?: continue
            onPhoto(Photo(ID_PREFIX + photo.id, photo.name, photo.size.toULong(), photo.taken, photo.screenshot, thumb))
        }
    }

    private data class Found(val id: Long, val name: String, val size: Long, val taken: Long, val screenshot: Boolean)

    /** A JPEG of at most 96 KB, 512 pixels a side. */
    private fun preview(id: Long): ByteArray? = runCatching {
        val uri = ContentUris.withAppendedId(COLLECTION, id)
        val bitmap = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            context.contentResolver.loadThumbnail(uri, Size(PREVIEW_PX, PREVIEW_PX), null)
        } else {
            @Suppress("DEPRECATION")
            MediaStore.Images.Thumbnails.getThumbnail(context.contentResolver, id, MediaStore.Images.Thumbnails.MINI_KIND, null)
        } ?: return null
        try {
            // Lower quality until it fits.
            generateSequence(85) { it - 15 }.takeWhile { it >= 40 }
                .map { quality -> ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, quality, it) }.toByteArray() }
                .firstOrNull { it.size <= MAX_PREVIEW_BYTES }
        } finally {
            bitmap.recycle()
        }
    }.getOrElse {
        Log.i(TAG, "no preview for a new photo", it)
        null
    }

    companion object {
        private const val TAG = "RecentPhotos"
        private const val LAST_SEEN = "lastSeen"
        private const val ID_PREFIX = "media:"
        private const val SETTLE_MS = 1500L
        private const val MAX_PER_CHECK = 3
        private const val PREVIEW_PX = 512
        private const val MAX_PREVIEW_BYTES = 96 * 1024
        private val COLLECTION: Uri = MediaStore.Images.Media.EXTERNAL_CONTENT_URI

        /** What to ask for (Android 13 split photos from other files). */
        val permissions: Array<String> =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                arrayOf(Manifest.permission.READ_MEDIA_IMAGES)
            } else {
                arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
            }

        /**
         * Whether the app sees every new photo. Access to only some photos
         * (Android 14) isn't enough: new ones wouldn't show up.
         */
        fun hasAccess(context: Context): Boolean = permissions.all {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }

        /** Opens a photo a PC asked for; `null` when it's gone or not one of ours. */
        fun open(context: Context, id: String): FileToSend? {
            val mediaId = id.removePrefix(ID_PREFIX).takeIf { id.startsWith(ID_PREFIX) }?.toLongOrNull() ?: return null
            val uri = ContentUris.withAppendedId(COLLECTION, mediaId)
            val name = runCatching {
                context.contentResolver.query(uri, arrayOf(MediaStore.Images.Media.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) it.getString(0) else null
                }
            }.getOrNull() ?: return null
            val fd = runCatching { context.contentResolver.openFileDescriptor(uri, "r") }.getOrNull() ?: return null
            return FileToSend.Fd(name = name, fd = fd.detachFd(), folder = null)
        }
    }
}
