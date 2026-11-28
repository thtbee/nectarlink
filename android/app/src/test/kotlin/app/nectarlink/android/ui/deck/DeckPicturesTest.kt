// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.deck

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class DeckPicturesTest {
    @Test
    fun `a picture the PC can send is decoded at full size`() {
        assertEquals(1, deckPictureSampleSize(256, 256))
        assertEquals(1, deckPictureSampleSize(512, 300))
        assertEquals(1, deckPictureSampleSize(1, 1))
    }

    @Test
    fun `a larger picture is divided until its longest side fits`() {
        assertEquals(2, deckPictureSampleSize(513, 100))
        assertEquals(2, deckPictureSampleSize(100, 1024))
        assertEquals(4, deckPictureSampleSize(1025, 1025))
        assertEquals(8, deckPictureSampleSize(4000, 3000))
        assertEquals(4, deckPictureSampleSize(100, 100, maxSide = 32))
    }

    @Test
    fun `a size that isn't a picture's isn't decoded`() {
        assertEquals(0, deckPictureSampleSize(0, 0))
        assertEquals(0, deckPictureSampleSize(-1, 256))
        assertEquals(0, deckPictureSampleSize(256, 0))
    }

    @Test
    fun `the cache drops the least recently used first`() {
        val cache = SizedLruCache<String, String>(maxSize = 6) { it.length }
        cache.put("a", "aa")
        cache.put("b", "bb")
        cache.put("c", "cc")
        cache.get("a")
        cache.put("d", "dd")
        assertEquals("aa", cache.get("a"))
        assertNull(cache.get("b"))
        assertEquals("cc", cache.get("c"))
        assertEquals("dd", cache.get("d"))
        assertEquals(6, cache.size())
    }

    @Test
    fun `the cache counts a replaced value once`() {
        val cache = SizedLruCache<String, String>(maxSize = 10) { it.length }
        cache.put("a", "aaaa")
        cache.put("a", "aa")
        assertEquals(2, cache.size())
        assertEquals(1, cache.count())
    }

    @Test
    fun `the cache doesn't keep a value larger than all of it`() {
        val cache = SizedLruCache<String, String>(maxSize = 3) { it.length }
        cache.put("a", "aa")
        cache.put("b", "bbbb")
        assertNull(cache.get("b"))
        assertEquals("aa", cache.get("a"))
        assertEquals(2, cache.size())
    }

    private class Decoder {
        var calls = 0
        fun decode(png: ByteArray): String? {
            calls++
            return if (png.isEmpty()) null else String(png)
        }
    }

    private fun store(decoder: Decoder) = DeckPictureStore(1024, String::length, decoder::decode)

    @Test
    fun `a tile shows its icon until the phone has the picture`() {
        val decoder = Decoder()
        val store = store(decoder)
        assertNull(store.load("p1") { null })
        assertNull(store.cached("p1"))
        assertEquals("png", store.load("p1") { "png".toByteArray() })
        assertEquals("png", store.cached("p1"))
    }

    @Test
    fun `a picture is decoded once`() {
        val decoder = Decoder()
        val store = store(decoder)
        val first = store.load("p1") { "png".toByteArray() }
        var fetched = false
        val second = store.load("p1") { fetched = true; "png".toByteArray() }
        assertSame(first, second)
        assertEquals(1, decoder.calls)
        assertEquals(false, fetched)
    }

    @Test
    fun `a picture that doesn't decode keeps the icon and isn't tried again`() {
        val decoder = Decoder()
        val store = store(decoder)
        assertNull(store.load("bad") { ByteArray(0) })
        assertNull(store.load("bad") { ByteArray(0) })
        assertEquals(1, decoder.calls)
        assertNull(store.cached("bad"))
    }

    @Test
    fun `a decoder that throws counts as one that failed`() {
        val store = DeckPictureStore<String>(1024, String::length) { error("not a PNG") }
        assertNull(store.load("p1") { "png".toByteArray() })
    }
}
