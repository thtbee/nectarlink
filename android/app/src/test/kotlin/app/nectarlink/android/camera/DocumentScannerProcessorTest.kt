// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.camera

import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DocumentScannerProcessorTest {

    @Test
    fun `QuadCorners ordered sorts shuffled corners clockwise`() {
        val tl = PointF2D(0.12f, 0.15f)
        val tr = PointF2D(0.88f, 0.18f)
        val br = PointF2D(0.84f, 0.89f)
        val bl = PointF2D(0.14f, 0.85f)

        val shuffled = QuadCorners(
            topLeft = br,
            topRight = bl,
            bottomRight = tl,
            bottomLeft = tr,
        )
        val ordered = shuffled.ordered()
        assertEquals(tl, ordered.topLeft)
        assertEquals(tr, ordered.topRight)
        assertEquals(br, ordered.bottomRight)
        assertEquals(bl, ordered.bottomLeft)
    }

    @Test
    fun `detectDocumentQuad locates tilted bright rectangle on dark background`() {
        val width = 160
        val height = 160
        val angle = 12.0 * PI / 180.0
        val cosA = cos(angle).toFloat()
        val sinA = sin(angle).toFloat()
        val halfW = 0.30f
        val halfH = 0.24f

        fun rot(dx: Float, dy: Float): PointF2D = PointF2D(
            x = 0.5f + dx * cosA - dy * sinA,
            y = 0.5f + dx * sinA + dy * cosA,
        )

        val expectedTl = rot(-halfW, -halfH)
        val expectedTr = rot(halfW, -halfH)
        val expectedBr = rot(halfW, halfH)
        val expectedBl = rot(-halfW, halfH)

        val luma = ByteArray(width * height)
        for (y in 0 until height) {
            val ny = y.toFloat() / (height - 1).toFloat() - 0.5f
            for (x in 0 until width) {
                val nx = x.toFloat() / (width - 1).toFloat() - 0.5f
                val ux = nx * cosA + ny * sinA
                val uy = -nx * sinA + ny * cosA
                val inside = abs(ux) <= halfW && abs(uy) <= halfH
                luma[y * width + x] = (if (inside) 230 else 25).toByte()
            }
        }

        val detected = DocumentScannerProcessor.detectDocumentQuad(luma, width, height)
        assertTrue(DocumentScannerProcessor.isValidDocumentQuad(detected))
        assertTrue(dist(expectedTl, detected.topLeft) < 0.06f)
        assertTrue(dist(expectedTr, detected.topRight) < 0.06f)
        assertTrue(dist(expectedBr, detected.bottomRight) < 0.06f)
        assertTrue(dist(expectedBl, detected.bottomLeft) < 0.06f)
    }

    @Test
    fun `computeHomography and warpPerspectiveArgb rectify skewed quad and enhance contrast`() {
        val srcW = 100
        val srcH = 100
        val quad = QuadCorners(
            topLeft = PointF2D(20f, 15f),
            topRight = PointF2D(80f, 20f),
            bottomRight = PointF2D(90f, 85f),
            bottomLeft = PointF2D(10f, 80f),
        )

        val h = DocumentScannerProcessor.computeHomography(quad, 59f, 59f)
        // Verify (0, 0) maps to topLeft (20, 15) and (59, 59) maps to bottomRight (90, 85)
        assertEquals(20.0, h[2] / h[8], 1e-4)
        assertEquals(15.0, h[5] / h[8], 1e-4)
        val denomBr = h[6] * 59.0 + h[7] * 59.0 + h[8]
        assertEquals(90.0, (h[0] * 59.0 + h[1] * 59.0 + h[2]) / denomBr, 1e-4)
        assertEquals(85.0, (h[3] * 59.0 + h[4] * 59.0 + h[5]) / denomBr, 1e-4)

        val srcPixels = IntArray(srcW * srcH)
        val poly = quad.toList()
        for (y in 0 until srcH) {
            for (x in 0 until srcW) {
                val inQuad = pointInConvexQuad(x.toFloat(), y.toFloat(), poly)
                val inInk = x in 42..58 && y in 42..58
                val gray = when {
                    !inQuad -> 15
                    inInk -> 60
                    else -> 205
                }
                srcPixels[y * srcW + x] = (0xFF shl 24) or (gray shl 16) or (gray shl 8) or gray
            }
        }

        val dstW = 60
        val dstH = 60
        val unenhanced = DocumentScannerProcessor.warpPerspectiveArgb(
            srcPixels = srcPixels,
            srcWidth = srcW,
            srcHeight = srcH,
            quadPixels = quad,
            dstWidth = dstW,
            dstHeight = dstH,
            enhanceDocument = false,
        )
        val enhanced = DocumentScannerProcessor.warpPerspectiveArgb(
            srcPixels = srcPixels,
            srcWidth = srcW,
            srcHeight = srcH,
            quadPixels = quad,
            dstWidth = dstW,
            dstHeight = dstH,
            enhanceDocument = true,
        )

        // Check that warped corners (near the edge of the destination rectangle) contain bright paper, not the dark 15 background
        val cornerPaperUnenhanced = unenhanced[2 * dstW + 2] and 0xFF
        assertTrue("Expected bright paper at warped corner, got $cornerPaperUnenhanced", cornerPaperUnenhanced >= 190)

        val paperUnenhanced = unenhanced[10 * dstW + 10] and 0xFF
        val inkUnenhanced = unenhanced[30 * dstW + 30] and 0xFF
        val paperEnhanced = enhanced[10 * dstW + 10] and 0xFF
        val inkEnhanced = enhanced[30 * dstW + 30] and 0xFF

        assertTrue(paperEnhanced > paperUnenhanced)
        assertTrue((paperEnhanced - inkEnhanced) > (paperUnenhanced - inkUnenhanced))
    }

    private fun dist(a: PointF2D, b: PointF2D): Float = hypot(a.x - b.x, a.y - b.y)

    private fun pointInConvexQuad(px: Float, py: Float, pts: List<PointF2D>): Boolean {
        for (i in 0 until 4) {
            val a = pts[i]
            val b = pts[(i + 1) % 4]
            val cross = (b.x - a.x) * (py - a.y) - (b.y - a.y) * (px - a.x)
            if (cross < 0f) return false
        }
        return true
    }
}
