// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.deck

import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap

/*
 * Pictures on Deck tiles (#490). The PC names a tile's picture by an ID taken
 * from the hash of its PNG, so the same ID always means the same picture: a
 * decoded picture can be kept by ID, and one that doesn't decode never will.
 */

/** The longest side a tile's picture is decoded at. The PC sends at most 256 x 256. */
internal const val DECK_PICTURE_MAX_SIDE = 512

/** Memory kept for decoded pictures, shared by every PC's Deck. */
private const val DECK_PICTURE_CACHE_BYTES = 8 * 1024 * 1024

/**
 * The power of two to divide a `width` x `height` picture by so that its
 * longest side is at most [maxSide], or 0 when the size isn't a picture's.
 */
internal fun deckPictureSampleSize(width: Int, height: Int, maxSide: Int = DECK_PICTURE_MAX_SIDE): Int {
    if (width <= 0 || height <= 0 || maxSide <= 0) return 0
    val longest = maxOf(width, height)
    var sample = 1
    // Rounded up, as the decoder may round a side's division up.
    while ((longest + sample - 1) / sample > maxSide) sample *= 2
    return sample
}

/** A least recently used cache that holds at most [maxSize], as [sizeOf] counts it. */
internal class SizedLruCache<K : Any, V : Any>(
    private val maxSize: Int,
    private val sizeOf: (V) -> Int,
) {
    private val entries = LinkedHashMap<K, V>(16, 0.75f, true)
    private var size = 0

    @Synchronized
    fun get(key: K): V? = entries[key]

    @Synchronized
    fun put(key: K, value: V) {
        entries.remove(key)?.let { size -= sizeOf(it) }
        val valueSize = sizeOf(value)
        if (valueSize > maxSize) return
        entries[key] = value
        size += valueSize
        val oldest = entries.entries.iterator()
        while (size > maxSize && oldest.hasNext()) {
            size -= sizeOf(oldest.next().value)
            oldest.remove()
        }
    }

    @Synchronized
    fun size(): Int = size

    @Synchronized
    fun count(): Int = entries.size
}

/**
 * Tiles' pictures by ID, decoded once. Generic over the decoded type so it
 * can be tested without Android's bitmaps.
 */
internal class DeckPictureStore<P : Any>(
    maxBytes: Int,
    sizeOf: (P) -> Int,
    private val decode: (ByteArray) -> P?,
) {
    private val cache = SizedLruCache<String, P>(maxBytes, sizeOf)
    private val undecodable = mutableSetOf<String>()

    /** The picture `id` if it's already decoded. */
    fun cached(id: String): P? = cache.get(id)

    /**
     * The picture `id`, decoding the PNG [fetch] returns when it isn't kept
     * yet. Null while the phone doesn't have it (fetch returns null) and for
     * one that doesn't decode, which isn't tried again. Blocking: call it off
     * the main thread.
     */
    fun load(id: String, fetch: () -> ByteArray?): P? {
        cache.get(id)?.let { return it }
        if (synchronized(undecodable) { id in undecodable }) return null
        val png = fetch() ?: return null
        val picture = runCatching { decode(png) }.getOrNull()
        if (picture == null) {
            synchronized(undecodable) { undecodable += id }
            return null
        }
        cache.put(id, picture)
        return picture
    }
}

/** Decodes a tile's PNG at most [DECK_PICTURE_MAX_SIDE] on its longest side. */
private fun decodeDeckPicture(png: ByteArray): ImageBitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(png, 0, png.size, bounds)
    val sample = deckPictureSampleSize(bounds.outWidth, bounds.outHeight)
    if (sample == 0) return null
    val options = BitmapFactory.Options().apply { inSampleSize = sample }
    return BitmapFactory.decodeByteArray(png, 0, png.size, options)?.asImageBitmap()
}

/** The pictures of every PC's Deck tiles. */
internal val deckPictures = DeckPictureStore(
    maxBytes = DECK_PICTURE_CACHE_BYTES,
    sizeOf = { it.asAndroidBitmap().allocationByteCount },
    decode = ::decodeDeckPicture,
)
