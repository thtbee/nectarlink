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
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.core.content.FileProvider
import app.nectarlink.android.files.OutgoingFiles
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

    @Volatile private var appContext: Context? = null
    @Volatile private var lastWrittenText: String? = null
    @Volatile private var lastWrittenHtml: String? = null
    @Volatile private var lastWrittenUri: Uri? = null
    @Volatile private var lastWrittenAtMs: Long = 0L
    private const val ECHO_WINDOW_MS = 6_000L

    private const val PREFS = "clipboard"
    private const val SYNC_PASSWORDS = "sync_passwords"

    private fun normalizeClipText(s: String): String = s.replace("\r\n", "\n").replace('\r', '\n')

    fun init(context: Context) {
        appContext = context.applicationContext
    }

    sealed interface Read {
        data class Text(val text: String, val html: String? = null) : Read
        /** An image, still to be read with [readImage] (the app holds access to it for now). */
        data class Image(val uri: Uri) : Read
        /** Marked sensitive by the app it came from (a password manager): not sent. */
        data object Private : Read
        /** The same, when the user chose to sync those. */
        data class Sensitive(val text: String) : Read
        data object Empty : Read
    }

    /** An image ready to send: PNG or JPEG, as the protocol allows. */
    class ImageData(val mime: String, val bytes: ByteArray)

    sealed interface ImageResult {
        class Ready(val image: ImageData) : ImageResult
        data object TooLarge : ImageResult
        data object Unreadable : ImageResult
    }

    /** True when `(text, html)` was written to the phone clipboard by NectarLink itself within the last few seconds. */
    fun isRecentEcho(text: String, html: String? = null): Boolean {
        val last = lastWrittenText ?: return false
        if (normalizeClipText(last) != normalizeClipText(text)) return false
        if ((android.os.SystemClock.elapsedRealtime() - lastWrittenAtMs) !in 0..ECHO_WINDOW_MS) return false
        if (!html.isNullOrBlank() && lastWrittenHtml == null) return false
        return true
    }

    /** True when `uri` was placed on the phone clipboard by NectarLink itself within the last few seconds. */
    fun isRecentUriEcho(uri: Uri): Boolean {
        val last = lastWrittenUri ?: return false
        if (last != uri) return false
        return (android.os.SystemClock.elapsedRealtime() - lastWrittenAtMs) in 0..ECHO_WINDOW_MS
    }

    /** Records a local phone copy so a plain-text bounce-back (e.g. emulator host clipboard pipe) won't overwrite it. */
    fun recordLocalCopy(text: String, html: String?) {
        lastWrittenText = text
        lastWrittenHtml = html?.takeIf(String::isNotBlank)
        lastWrittenAtMs = android.os.SystemClock.elapsedRealtime()
    }

    internal sealed interface ItemChoice<out U> {
        data class Text(val index: Int, val text: String, val coerced: Boolean) : ItemChoice<Nothing>
        data class Image<out U>(val uri: U) : ItemChoice<U>
        data object Empty : ItemChoice<Nothing>
    }

    internal class ClipCandidate<out U>(
        val text: CharSequence? = null,
        val uri: U? = null,
        val isUriEcho: Boolean = false,
        val uriMime: () -> String? = { null },
        val coercedText: () -> CharSequence? = { null },
    )

    internal fun <U : Any> selectClipItem(
        items: List<ClipCandidate<U>>,
        descHasImage: Boolean = false,
        isTextEcho: (String) -> Boolean = { false },
    ): ItemChoice<U> {
        for ((index, item) in items.withIndex()) {
            val direct = item.text?.toString()?.takeIf(String::isNotBlank)
            if (direct != null) {
                return ItemChoice.Text(index, direct, coerced = false)
            }
            // Checked before coercing to text, which would turn it into its address.
            val uri = item.uri
            if (uri != null) {
                if (item.isUriEcho) return ItemChoice.Empty
                val mime = item.uriMime()
                if (mime?.startsWith("image/") == true || (mime == null && descHasImage)) {
                    return ItemChoice.Image(uri)
                }
            }
        }
        val coerced = items.asSequence()
            .withIndex()
            // Never call coerceToText on an item with a content URI because Android opens a FileInputStream
            // on the URI and decodes raw binary file bytes (e.g. JPEG/JFIF) as UTF-8 text.
            .filter { (_, item) -> item.uri == null }
            .firstNotNullOfOrNull { (index, item) ->
                item.coercedText()?.toString()?.takeIf(String::isNotBlank)?.let { index to it }
            }
        return if (coerced != null && !isTextEcho(coerced.second)) {
            ItemChoice.Text(coerced.first, coerced.second, coerced = true)
        } else {
            ItemChoice.Empty
        }
    }

    fun read(context: Context): Read {
        appContext = context.applicationContext
        val clip = manager(context)?.primaryClip ?: return Read.Empty
        if (clip.description.isSensitive()) {
            if (!syncPasswords(context)) return Read.Private
            val text = (0 until clip.itemCount).firstNotNullOfOrNull { clip.getItemAt(it).text?.toString() }
            return text?.takeIf(String::isNotBlank)?.let { Read.Sensitive(it) } ?: Read.Empty
        }
        val resolver = context.contentResolver
        val descHasImage = (0 until clip.description.mimeTypeCount)
            .any { clip.description.getMimeType(it)?.startsWith("image/") == true }
        val candidates = (0 until clip.itemCount).map { i ->
            val item = clip.getItemAt(i)
            val uri = item.uri?.takeIf { OutgoingFiles.isAllowedShareUri(it.scheme, it.authority, context.packageName) }
            ClipCandidate(
                text = item.text,
                uri = uri,
                isUriEcho = uri != null && isRecentUriEcho(uri),
                uriMime = { uri?.let { runCatching { resolver.getType(it) }.getOrNull() } },
                coercedText = { item.coerceToText(context) },
            )
        }
        return when (val choice = selectClipItem(candidates, descHasImage, ::isRecentEcho)) {
            is ItemChoice.Text -> {
                if (choice.coerced) {
                    Read.Text(choice.text)
                } else {
                    val item = clip.getItemAt(choice.index)
                    val cs = item.text
                    val html = item.htmlText?.takeIf(String::isNotBlank) ?: run {
                        if (cs is android.text.Spanned &&
                            cs.getSpans(0, cs.length, android.text.style.CharacterStyle::class.java).isNotEmpty()
                        ) {
                            android.text.Html.toHtml(cs, android.text.Html.TO_HTML_PARAGRAPH_LINES_CONSECUTIVE)
                                .takeIf(String::isNotBlank)
                        } else if (clip.description?.label == "host clipboard" && isRecentEcho(choice.text)) {
                            lastWrittenHtml
                        } else {
                            null
                        }
                    }
                    Read.Text(choice.text, html)
                }
            }
            is ItemChoice.Image -> Read.Image(choice.uri)
            ItemChoice.Empty -> Read.Empty
        }
    }

    /**
     * Reads a copied image for sending (blocking; call off the main thread).
     * PNG and JPEG go as they are; other formats are converted to PNG.
     */
    fun readImage(context: Context, uri: Uri): ImageResult = runCatching {
        if (!OutgoingFiles.isAllowedShareUri(uri.scheme, uri.authority, context.packageName)) {
            return ImageResult.Unreadable
        }
        val resolver = context.contentResolver
        val bytes = resolver.openFileDescriptor(uri, "r")?.use { pfd ->
            if (!OutgoingFiles.isSafeOpenedFd(context, pfd, allowClipboardCache = true)) {
                return ImageResult.Unreadable
            }
            val size = pfd.statSize
            if (size > MAX_IMAGE_BYTES) return ImageResult.TooLarge
            ParcelFileDescriptor.AutoCloseInputStream(pfd).use { input ->
                // Read one byte past the limit to notice a source that didn't say its size.
                val buffer = input.readNBytesCompat(MAX_IMAGE_BYTES.toInt() + 1)
                if (buffer.size > MAX_IMAGE_BYTES) return ImageResult.TooLarge
                buffer
            }
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

    /** Puts text (and optional HTML) a PC sent on the clipboard. */
    /** Whether what password managers mark sensitive also goes to PCs. */
    fun syncPasswords(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(SYNC_PASSWORDS, false)

    fun setSyncPasswords(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(SYNC_PASSWORDS, on).apply()
    }

    /** A password from a PC: marked sensitive, so Android hides it and keyboards don't keep it. */
    fun writeSensitive(context: Context, text: String): Boolean = runCatching {
        appContext = context.applicationContext
        val mgr = manager(context) ?: return false
        lastWrittenText = text
        lastWrittenHtml = null
        lastWrittenUri = null
        lastWrittenAtMs = android.os.SystemClock.elapsedRealtime()
        if (app.nectarlink.android.elevated.Elevated.setClipboard(text, null, sensitive = true)) {
            return@runCatching true
        }
        val clip = ClipData.newPlainText("Nectarlink", text)
        clip.description.extras = android.os.PersistableBundle().apply {
            putBoolean("com.android.systemui.SUPPRESS_CLIPBOARD_OVERLAY", true)
            putBoolean("android.content.extra.IS_REMOTE_DEVICE", true)
            putBoolean("android.content.extra.IS_SENSITIVE", true)
        }
        mgr.setPrimaryClip(clip)
        true
    }.getOrDefault(false)

    /**
     * Takes a password from a PC off the clipboard, if it's still what's there. Android
     * hides the clipboard from apps in the background: without Elevated, a phone that
     * can't tell leaves it, since it could be something the user copied since.
     */
    fun clearSensitive(context: Context, text: String) {
        if (app.nectarlink.android.elevated.Elevated.clearClipboard(text)) return
        runCatching {
            val mgr = manager(context) ?: return
            val now = mgr.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString() ?: return
            if (normalizeClipText(now) != normalizeClipText(text)) return
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                mgr.clearPrimaryClip()
            } else {
                mgr.setPrimaryClip(ClipData.newPlainText("Nectarlink", ""))
            }
        }
    }

    fun write(context: Context, text: String, html: String? = null): Boolean = runCatching {
        appContext = context.applicationContext
        val cleanHtml = html?.takeIf(String::isNotBlank)
        if (isRecentEcho(text) && (cleanHtml == null || cleanHtml == lastWrittenHtml)) {
            return@runCatching true
        }
        val mgr = manager(context) ?: return false
        val existingText = runCatching {
            mgr.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()
        }.getOrNull()
        lastWrittenText = text
        lastWrittenHtml = cleanHtml
        lastWrittenUri = null
        lastWrittenAtMs = android.os.SystemClock.elapsedRealtime()
        if (existingText != null && normalizeClipText(existingText) == normalizeClipText(text) && cleanHtml == null) {
            return@runCatching true
        }
        // The shell's copy can ask SystemUI not to show it; the app's can't.
        if (app.nectarlink.android.elevated.Elevated.setClipboard(text, cleanHtml)) return@runCatching true
        val clip = if (cleanHtml != null) {
            ClipData.newHtmlText("Nectarlink", text, cleanHtml)
        } else {
            ClipData.newPlainText("Nectarlink", text)
        }
        runCatching {
            val extras = (clip.description.extras ?: android.os.PersistableBundle()).apply {
                putBoolean("com.android.systemui.SUPPRESS_CLIPBOARD_OVERLAY", true)
                putBoolean("android.content.extra.IS_REMOTE_DEVICE", true)
            }
            clip.description.extras = extras
        }
        mgr.setPrimaryClip(clip)
        true
    }.getOrDefault(false)

    /**
     * Puts an image a PC sent on the clipboard: kept in the app's cache and
     * shared through its FileProvider, so the app that pastes can read it.
     * The previous one is deleted; only the latest is on the clipboard.
     */
    fun writeImage(context: Context, mime: String, bytes: ByteArray): Boolean = runCatching {
        appContext = context.applicationContext
        val dir = File(context.cacheDir, IMAGE_DIR).apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val extension = if (mime == "image/jpeg") "jpg" else "png"
        // A new name each time: apps cache what they read by address.
        val file = File(dir, "Image ${System.currentTimeMillis()}.$extension")
        file.writeBytes(bytes)
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        lastWrittenUri = uri
        lastWrittenText = uri.toString()
        lastWrittenHtml = null
        lastWrittenAtMs = android.os.SystemClock.elapsedRealtime()
        val clip = ClipData.newUri(context.contentResolver, "Nectarlink", uri)
        manager(context)?.setPrimaryClip(clip) != null
    }.getOrElse {
        Log.w(TAG, "can't put an image on the clipboard", it)
        false
    }

    /**
     * Places a received file's content [Uri] onto the phone clipboard so the user can
     * immediately paste it into chat or email apps, while recording the URI to suppress
     * clipboard echo back to the PC.
     */
    fun setFileUri(context: Context, uri: Uri, label: String = "NectarLink file"): Boolean = runCatching {
        appContext = context.applicationContext
        lastWrittenUri = uri
        lastWrittenText = uri.toString()
        lastWrittenHtml = null
        lastWrittenAtMs = android.os.SystemClock.elapsedRealtime()
        val mgr = manager(context) ?: return false
        val clip = ClipData.newUri(context.contentResolver, label.ifBlank { "NectarLink file" }, uri)
        mgr.setPrimaryClip(clip)
        true
    }.getOrElse {
        Log.w(TAG, "can't put a file URI on the clipboard", it)
        false
    }

    /** Overload using the cached application [Context] when available. */
    fun setFileUri(uri: Uri, label: String = "NectarLink file"): Boolean {
        val ctx = appContext ?: return false
        return setFileUri(ctx, uri, label)
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
