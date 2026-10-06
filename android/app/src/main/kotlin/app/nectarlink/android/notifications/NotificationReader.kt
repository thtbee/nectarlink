// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.service.notification.NotificationListenerService.Ranking
import android.service.notification.NotificationListenerService.RankingMap
import android.service.notification.StatusBarNotification
import android.util.LruCache
import androidx.core.app.NotificationCompat
import androidx.core.graphics.drawable.toBitmap
import androidx.core.graphics.scale
import java.io.ByteArrayOutputStream
import app.nectarlink.core.Notification as Mirrored
import app.nectarlink.core.NotificationAction as MirroredAction

/**
 * Turns Android notifications into what Nectarlink mirrors, and decides
 * which ones are worth mirroring at all.
 */
internal class NotificationReader(private val context: Context) {
    private val packages: PackageManager = context.packageManager
    private val icons = LruCache<String, ByteArray>(48)
    private val names = LruCache<String, String>(128)
    /** Pictures already made, by notification and picture (updates repeat them). */
    private val images = LruCache<String, ByteArray>(16)

    /** `null` when this notification isn't mirrored. */
    fun read(sbn: StatusBarNotification, ranking: RankingMap?, update: Boolean): Mirrored? {
        if (!shouldMirror(sbn)) return null
        val n = sbn.notification
        val content = content(n) ?: return null
        val silent = isSilent(sbn, ranking) || (update && n.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0)
        return Mirrored(
            key = sbn.key,
            app = sbn.packageName,
            appName = appName(sbn.packageName),
            title = content.title,
            text = content.text,
            sub = content.sub,
            `when` = sbn.postTime,
            actions = actions(n),
            silent = silent,
            icon = icon(sbn.packageName),
            image = image(sbn.key, n),
        )
    }

    private fun shouldMirror(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        val skipped = Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE or
            Notification.FLAG_GROUP_SUMMARY or Notification.FLAG_LOCAL_ONLY
        return sbn.packageName != context.packageName &&
            sbn.isClearable &&
            n.flags and skipped == 0 &&
            // Media players and progress bars get their own features.
            n.category != Notification.CATEGORY_TRANSPORT &&
            n.category != Notification.CATEGORY_PROGRESS &&
            !n.extras.containsKey(Notification.EXTRA_MEDIA_SESSION)
    }

    private fun isSilent(sbn: StatusBarNotification, ranking: RankingMap?): Boolean {
        val r = Ranking()
        if (ranking == null || !ranking.getRanking(sbn.key, r)) return false
        return r.importance < NotificationManager.IMPORTANCE_DEFAULT
    }

    private data class Content(val title: String?, val text: String?, val sub: String?)

    /** Title and text, from a conversation's last message when there is one. */
    private fun content(n: Notification): Content? {
        val extras = n.extras
        val conversation = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n)
        val last = conversation?.messages?.lastOrNull()
        if (conversation != null && last != null) {
            val sender = last.person?.name?.toString()
            val group = conversation.isGroupConversation
            val title = conversation.conversationTitle?.toString()?.takeIf { group } ?: sender
            val text = last.text?.toString()?.let { if (group && sender != null) "$sender: $it" else it }
            return Content(title, text, extras.text(Notification.EXTRA_SUB_TEXT)).takeIf { it.hasContent() }
        }
        val lines = extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.joinToString("\n")
        val text = extras.text(Notification.EXTRA_BIG_TEXT) ?: lines?.ifBlank { null } ?: extras.text(Notification.EXTRA_TEXT)
        val title = extras.text(Notification.EXTRA_TITLE_BIG) ?: extras.text(Notification.EXTRA_TITLE)
        return Content(title, text, extras.text(Notification.EXTRA_SUB_TEXT)).takeIf { it.hasContent() }
    }

    private fun Content.hasContent() = !title.isNullOrBlank() || !text.isNullOrBlank()

    private fun android.os.Bundle.text(key: String): String? = getCharSequence(key)?.toString()?.trim()?.ifEmpty { null }

    /** Buttons, with their index as ID; reply actions take free text. */
    private fun actions(n: Notification): List<MirroredAction> =
        n.actions.orEmpty().mapIndexedNotNull { index, action ->
            val title = action.title?.toString()?.trim()
            if (title.isNullOrEmpty() || action.actionIntent == null) return@mapIndexedNotNull null
            MirroredAction(
                id = index.toString(),
                title = title,
                reply = action.remoteInputs.orEmpty().any { it.allowFreeFormInput },
            )
        }

    /**
     * The picture a notification shows, as a small JPEG: a big picture, or
     * the last image in a conversation. `null` when there's none or it
     * can't be read (images in messages often aren't shared with listeners).
     */
    private fun image(key: String, n: Notification): ByteArray? = runCatching {
        val source = pictureSource(n) ?: return null
        images.get("$key\u0001${source.id}")?.let { return it }
        val bitmap = source.load() ?: return null
        jpeg(bitmap).also { bytes ->
            if (bitmap !== source.owned) bitmap.recycle()
            if (bytes != null) images.put("$key\u0001${source.id}", bytes)
        }
    }.getOrNull()

    /** Where a picture comes from: an ID for caching, and how to load it. */
    private class Picture(val id: String, val owned: Bitmap?, val load: () -> Bitmap?)

    private fun pictureSource(n: Notification): Picture? {
        val extras = n.extras
        val bitmap = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            extras.getParcelable(Notification.EXTRA_PICTURE, Bitmap::class.java)
        } else {
            @Suppress("DEPRECATION")
            extras.getParcelable(Notification.EXTRA_PICTURE) as? Bitmap
        }
        if (bitmap != null) {
            return Picture("${bitmap.width}x${bitmap.height}:${bitmap.generationId}", bitmap) { bitmap }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val icon = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                extras.getParcelable(Notification.EXTRA_PICTURE_ICON, Icon::class.java)
            } else {
                @Suppress("DEPRECATION")
                extras.getParcelable(Notification.EXTRA_PICTURE_ICON) as? Icon
            }
            if (icon != null) {
                return Picture("icon:${icon.hashCode()}", null) { icon.loadDrawable(context)?.toBitmap() }
            }
        }
        val message = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n)
            ?.messages
            ?.lastOrNull { it.dataMimeType?.startsWith("image/") == true && it.dataUri != null }
        val uri = message?.dataUri ?: return null
        return Picture(uri.toString(), null) { decode(uri) }
    }

    /** Decodes an image URI at about the size it's sent at (it may be huge). */
    private fun decode(uri: Uri): Bitmap? {
        val resolver = context.contentResolver
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) } ?: return null
        val longest = maxOf(bounds.outWidth, bounds.outHeight)
        if (longest <= 0) return null
        var sample = 1
        while (longest / (sample * 2) >= IMAGE_PX) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, options) }
    }

    /** A JPEG that fits the protocol's limit, smaller and softer if needed. */
    private fun jpeg(bitmap: Bitmap): ByteArray? {
        for ((side, quality) in listOf(IMAGE_PX to 82, IMAGE_PX to 70, IMAGE_PX to 55, SMALL_IMAGE_PX to 70)) {
            val scale = minOf(1f, side.toFloat() / maxOf(bitmap.width, bitmap.height))
            val sized = if (scale < 1f) {
                bitmap.scale((bitmap.width * scale).toInt().coerceAtLeast(1), (bitmap.height * scale).toInt().coerceAtLeast(1))
            } else {
                bitmap
            }
            val bytes = ByteArrayOutputStream().use { out ->
                sized.compress(Bitmap.CompressFormat.JPEG, quality, out)
                out.toByteArray()
            }
            if (sized !== bitmap) sized.recycle()
            if (bytes.size <= MAX_IMAGE_BYTES) return bytes
        }
        return null
    }

    private fun appName(pkg: String): String = names.get(pkg) ?: runCatching {
        packages.getApplicationLabel(packages.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg).also { names.put(pkg, it) }

    /** The app's icon as a small PNG (the core sends it once per PC). */
    private fun icon(pkg: String): ByteArray? = icons.get(pkg) ?: runCatching {
        val bitmap = packages.getApplicationIcon(pkg).toBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888)
        ByteArrayOutputStream().use { out ->
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
            out.toByteArray()
        }
    }.getOrNull()?.takeIf { it.size <= MAX_ICON_BYTES }?.also { icons.put(pkg, it) }

    private companion object {
        const val ICON_PX = 96
        const val MAX_ICON_BYTES = 64 * 1024
        /** Pictures' longest side, and the fallback for large ones. */
        const val IMAGE_PX = 512
        const val SMALL_IMAGE_PX = 360
        const val MAX_IMAGE_BYTES = 160 * 1024
    }
}
