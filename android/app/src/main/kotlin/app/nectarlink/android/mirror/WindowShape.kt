// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import kotlin.math.abs
import kotlin.math.ln

/**
 * The size of an app window's display: an exact, common screen shape, close
 * to the window's. Apps check their display's shape against the ones they
 * know (Samsung's Camera throws on 880x1920, the phone's 1080x2340 rounded
 * to encoder-friendly sizes), so a window never gets an odd one; the PC
 * shows the picture letterboxed in what is left over.
 */
object WindowShape {
    /** Sides are multiples of this, for encoders. */
    private const val ALIGN = 16
    private const val MIN_SIDE = 240

    /** Common shapes, as long side : short side. */
    private val COMMON = listOf(1 to 1, 4 to 3, 3 to 2, 16 to 10, 16 to 9, 2 to 1, 20 to 9, 21 to 9)

    /**
     * The largest display of a common shape (or the phone's own, [phoneW] x
     * [phoneH]) that fits [width] x [height] scaled down to [maxSize], in the
     * same orientation, with sides that are multiples of 16.
     */
    fun fit(width: Int, height: Int, maxSize: Int, phoneW: Int, phoneH: Int): Pair<Int, Int> {
        val w = width.coerceAtLeast(MIN_SIDE)
        val h = height.coerceAtLeast(MIN_SIDE)
        val scale = minOf(1.0, maxSize.toDouble() / maxOf(w, h))
        val (boxW, boxH) = w * scale to h * scale
        val portrait = h > w
        val wanted = maxOf(w, h).toDouble() / minOf(w, h)
        val shape = shapes(phoneW, phoneH).minBy { (long, short) -> abs(ln(long.toDouble() / short / wanted)) }
        // One step of the shape: the smallest exact size with aligned sides.
        val step = ALIGN / gcd(ALIGN, gcd(shape.first, shape.second))
        val (unitLong, unitShort) = shape.first * step to shape.second * step
        val (unitW, unitH) = if (portrait) unitShort to unitLong else unitLong to unitShort
        val n = minOf(boxW / unitW, boxH / unitH).toInt()
        val minN = (MIN_SIDE + minOf(unitW, unitH) - 1) / minOf(unitW, unitH)
        val count = n.coerceAtLeast(minN.coerceAtLeast(1))
        return unitW * count to unitH * count
    }

    /**
     * [width] x [height] scaled down to [maxSize], each side to the nearest
     * multiple of 16: the PC window's own shape, so a resized window has no
     * bars (the PC stretches the last few pixels). Unlike [fit], an app may
     * see an uncommon shape (Samsung's Camera refuses some).
     */
    fun exact(width: Int, height: Int, maxSize: Int): Pair<Int, Int> {
        val scale = minOf(1.0, maxSize.toDouble() / maxOf(width, height))
        fun side(v: Int) = (Math.round(v * scale / ALIGN) * ALIGN).toInt().coerceAtLeast(MIN_SIDE)
        return side(width) to side(height)
    }

    /** [COMMON] and the phone's own shape, each reduced (long : short). */
    private fun shapes(phoneW: Int, phoneH: Int): List<Pair<Int, Int>> {
        val own = if (phoneW > 0 && phoneH > 0) {
            val (long, short) = maxOf(phoneW, phoneH) to minOf(phoneW, phoneH)
            val g = gcd(long, short)
            listOf(long / g to short / g)
        } else {
            emptyList()
        }
        // A phone's odd shape (e.g. 2400:1081) would need a huge step; keep only reasonable ones.
        return (COMMON + own.filter { (long, short) -> long <= 64 && short <= 64 }).distinct()
    }

    private tailrec fun gcd(a: Int, b: Int): Int = if (b == 0) a else gcd(b, a % b)
}
