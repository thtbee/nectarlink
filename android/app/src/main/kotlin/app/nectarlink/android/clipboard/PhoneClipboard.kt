// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.core.content.FileProvider
import java.io.File

/**
 * The phone's clipboard. Android lets an app read it only while that app
 * is in front (and focused); writing works any time.
 */
object PhoneClipboard {
    private const val TAG = "PhoneClipboard"

    /** The largest image sent (docs/protocol/clipboard.md). */
    private const val MAX_IMAGE_BYTES = 32L * 1024 * 1024

    /** Where images from a PC are kept while they're on the clipboard. */
    private const val IMAGE_DIR = "clipboard"

    sealed interface Read {
        data class Text(val text: String) : Read
        /** An image, still to be read with [readImage] (the app holds access to it for now). */
        data class Image(val uri: Uri) : Read
        /** Marked sensitive by the app it came from (a password manager): never sent. */
        data object Private : Read
        data object Empty : Read
    }

    /** An image ready to send: PNG or JPEG, as the protocol allows. */
    class ImageData(val mime: String, val bytes: ByteArray)

    sealed interface ImageResult {
        class Ready(val image: ImageData) : ImageResult
        data object TooLarge : ImageResult
        data object Unreadable : ImageResult
    }

    fun read(context: Context): Read {
        val clip = manager(context)?.primaryClip ?: return Read.Empty
        if (clip.description.isSensitive()) return Read.Private
        val resolver = context.contentResolver
        for (i in 0 until clip.itemCount) {
            val item = clip.getItemAt(i)
            item.text?.toString()?.takeIf(String::isNotBlank)?.let { return Read.Text(it) }
            // Checked before coercing to text, which would turn it into its address.
            val uri = item.uri
            if (uri != null && resolver.getType(uri)?.startsWith("image/") == true) return Read.Image(uri)
        }
        val text = (0 until clip.itemCount)
            .firstNotNullOfOrNull { clip.getItemAt(it).coerceToText(context)?.toString()?.takeIf(String::isNotBlank) }
        return text?.let { Read.Text(it) } ?: Read.Empty
    }

    /**
     * Reads a copied image for sending (blocking; call off the main thread).
     * PNG and JPEG go as they are; other formats are converted to PNG.
     */
    fun readImage(context: Context, uri: Uri): ImageResult = runCatching {
        val resolver = context.contentResolver
        val size = resolver.openAssetFileDescriptor(uri, "r")?.use { it.length } ?: -1
        if (size > MAX_IMAGE_BYTES) return ImageResult.TooLarge
        val bytes = resolver.openInputStream(uri)?.use { input ->
            // Read one byte past the limit to notice a source that didn't say its size.
            val buffer = input.readNBytesCompat(MAX_IMAGE_BYTES.toInt() + 1)
            if (buffer.size > MAX_IMAGE_BYTES) return ImageResult.TooLarge
            buffer
        } ?: return ImageResult.Unreadable
        when (val mime = resolver.getType(uri)) {
            "image/png", "image/jpeg" -> ImageResult.Ready(ImageData(mime, bytes))
            else -> {
                val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return ImageResult.Unreadable
                val png = java.io.ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
                if (png.size() > MAX_IMAGE_BYTES) ImageResult.TooLarge else ImageResult.Ready(ImageData("image/png", png.toByteArray()))
            }
        }
    }.getOrElse {
        Log.w(TAG, "can't read the copied image", it)
        ImageResult.Unreadable
    }

    /** Puts text a PC sent on the clipboard. */
    fun write(context: Context, text: String): Boolean =
        runCatching { manager(context)?.setPrimaryClip(ClipData.newPlainText("Nectarlink", text)) != null }
            .getOrDefault(false)

    /**
     * Puts an image a PC sent on the clipboard: kept in the app's cache and
     * shared through its FileProvider, so the app that pastes can read it.
     * The previous one is deleted; only the latest is on the clipboard.
     */
    fun writeImage(context: Context, mime: String, bytes: ByteArray): Boolean = runCatching {
        val dir = File(context.cacheDir, IMAGE_DIR).apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val extension = if (mime == "image/jpeg") "jpg" else "png"
        // A new name each time: apps cache what they read by address.
        val file = File(dir, "Image ${System.currentTimeMillis()}.$extension")
        file.writeBytes(bytes)
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        val clip = ClipData.newUri(context.contentResolver, "Image", uri)
        manager(context)?.setPrimaryClip(clip) != null
    }.getOrElse {
        Log.w(TAG, "can't put an image on the clipboard", it)
        false
    }

    private fun manager(context: Context) = context.getSystemService(ClipboardManager::class.java)

    private fun ClipDescription.isSensitive(): Boolean {
        val key = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ClipDescription.EXTRA_IS_SENSITIVE
        } else {
            "android.content.extra.IS_SENSITIVE"
        }
        return extras?.getBoolean(key) == true
    }

    /** `readNBytes`, which needs Android 13. */
    private fun java.io.InputStream.readNBytesCompat(limit: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (out.size() < limit) {
            val read = read(buffer, 0, minOf(buffer.size, limit - out.size()))
            if (read < 0) break
            out.write(buffer, 0, read)
        }
        return out.toByteArray()
    }
}
