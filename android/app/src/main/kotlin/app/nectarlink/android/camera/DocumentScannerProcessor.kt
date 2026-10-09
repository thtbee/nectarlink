// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.camera

import android.graphics.Bitmap
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.acos
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/** A 2D point in either normalized (`0f..1f`) or pixel coordinates. */
data class PointF2D(val x: Float, val y: Float)

/** Four corners of a document quadrilateral in clockwise order. */
data class QuadCorners(
    val topLeft: PointF2D,
    val topRight: PointF2D,
    val bottomRight: PointF2D,
    val bottomLeft: PointF2D,
) {
    fun toList(): List<PointF2D> = listOf(topLeft, topRight, bottomRight, bottomLeft)

    /**
     * Orders any 4 points clockwise (`topLeft`, `topRight`, `bottomRight`, `bottomLeft`)
     * using coordinate sum `(x + y)` and difference `(y - x)`.
     */
    fun ordered(): QuadCorners {
        val pts = toList()
        val tl = pts.minBy { it.x + it.y }
        val br = pts.maxBy { it.x + it.y }
        val remaining = pts.filter { it !== tl && it !== br }.let { rem ->
            if (rem.size == 2) rem else pts.sortedBy { it.x + it.y }.subList(1, 3)
        }
        val tr = remaining.minBy { it.y - it.x }
        val bl = remaining.maxBy { it.y - it.x }
        return QuadCorners(
            topLeft = tl,
            topRight = tr,
            bottomRight = br,
            bottomLeft = bl,
        )
    }

    /** Scales normalized (`0f..1f`) corners into pixel coordinates. */
    fun toPixelSpace(width: Int, height: Int): QuadCorners {
        val w = (width - 1).coerceAtLeast(1).toFloat()
        val h = (height - 1).coerceAtLeast(1).toFloat()
        return QuadCorners(
            topLeft = PointF2D(topLeft.x * w, topLeft.y * h),
            topRight = PointF2D(topRight.x * w, topRight.y * h),
            bottomRight = PointF2D(bottomRight.x * w, bottomRight.y * h),
            bottomLeft = PointF2D(bottomLeft.x * w, bottomLeft.y * h),
        )
    }
}

/**
 * Pure-Kotlin document quad detector and 4-corner perspective rectifier for
 * Continuity Camera's document scanner.
 */
object DocumentScannerProcessor {
    private const val MAX_WORK_SIDE = 180
    private const val MIN_QUAD_AREA_FRACTION = 0.15f
    private const val MIN_INTERIOR_ANGLE_DEG = 45f
    private const val MAX_INTERIOR_ANGLE_DEG = 135f

    val DEFAULT_QUAD = QuadCorners(
        topLeft = PointF2D(0.08f, 0.08f),
        topRight = PointF2D(0.92f, 0.08f),
        bottomRight = PointF2D(0.92f, 0.92f),
        bottomLeft = PointF2D(0.08f, 0.92f),
    )

    /**
     * Detects the dominant document quadrilateral in an 8-bit luminance (`Y`) buffer,
     * returning normalized (`0f..1f`) clockwise corners. Falls back to [DEFAULT_QUAD]
     * if no valid convex document quad is found.
     */
    fun detectDocumentQuad(luma: ByteArray, width: Int, height: Int): QuadCorners {
        if (width < 16 || height < 16 || luma.size < width * height) return DEFAULT_QUAD

        val maxSide = max(width, height)
        val scale = if (maxSide > MAX_WORK_SIDE) MAX_WORK_SIDE.toFloat() / maxSide.toFloat() else 1f
        val gw = (width * scale).roundToInt().coerceAtLeast(16)
        val gh = (height * scale).roundToInt().coerceAtLeast(16)

        val raw = IntArray(gw * gh)
        for (y in 0 until gh) {
            val sy = (y * height / gh).coerceIn(0, height - 1)
            val rowOffset = sy * width
            for (x in 0 until gw) {
                val sx = (x * width / gw).coerceIn(0, width - 1)
                raw[y * gw + x] = luma[rowOffset + sx].toInt() and 0xFF
            }
        }

        // 3x3 Gaussian blur: [1, 2, 1; 2, 4, 2; 1, 2, 1] / 16
        val blurred = IntArray(gw * gh)
        for (y in 0 until gh) {
            val ym = max(0, y - 1)
            val yp = min(gh - 1, y + 1)
            for (x in 0 until gw) {
                val xm = max(0, x - 1)
                val xp = min(gw - 1, x + 1)
                val sum = raw[ym * gw + xm] + 2 * raw[ym * gw + x] + raw[ym * gw + xp] +
                    2 * raw[y * gw + xm] + 4 * raw[y * gw + x] + 2 * raw[y * gw + xp] +
                    raw[yp * gw + xm] + 2 * raw[yp * gw + x] + raw[yp * gw + xp]
                blurred[y * gw + x] = sum ushr 4
            }
        }

        // Sobel gradient magnitudes
        val mag = FloatArray(gw * gh)
        var maxMag = 0f
        for (y in 1 until gh - 1) {
            for (x in 1 until gw - 1) {
                val gx = -blurred[(y - 1) * gw + (x - 1)] + blurred[(y - 1) * gw + (x + 1)] -
                    2 * blurred[y * gw + (x - 1)] + 2 * blurred[y * gw + (x + 1)] -
                    blurred[(y + 1) * gw + (x - 1)] + blurred[(y + 1) * gw + (x + 1)]
                val gy = -blurred[(y - 1) * gw + (x - 1)] - 2 * blurred[(y - 1) * gw + x] -
                    blurred[(y - 1) * gw + (x + 1)] + blurred[(y + 1) * gw + (x - 1)] +
                    2 * blurred[(y + 1) * gw + x] + blurred[(y + 1) * gw + (x + 1)]
                val m = hypot(gx.toFloat(), gy.toFloat())
                mag[y * gw + x] = m
                if (m > maxMag) maxMag = m
            }
        }

        val otsu = otsuThreshold(blurred)
        val rawFg = BooleanArray(gw * gh) { blurred[it] > otsu }

        // 3x3 majority smoothing to remove isolated specks
        val fg = BooleanArray(gw * gh)
        var fgCount = 0
        for (y in 1 until gh - 1) {
            for (x in 1 until gw - 1) {
                var count = 0
                for (dy in -1..1) {
                    for (dx in -1..1) {
                        if (rawFg[(y + dy) * gw + (x + dx)]) count++
                    }
                }
                if (count >= 5) {
                    fg[y * gw + x] = true
                    fgCount++
                }
            }
        }

        val totalInterior = (gw - 2) * (gh - 2)
        val fgFraction = fgCount.toFloat() / totalInterior.coerceAtLeast(1).toFloat()

        // Extract candidate boundary points and foreground moments
        val boundaryX = IntArray(gw * gh)
        val boundaryY = IntArray(gw * gh)
        var boundaryCount = 0

        var sumX = 0.0
        var sumY = 0.0
        var momentCount = 0

        if (fgFraction in 0.12f..0.92f) {
            for (y in 1 until gh - 1) {
                for (x in 1 until gw - 1) {
                    val idx = y * gw + x
                    if (!fg[idx]) continue
                    sumX += x
                    sumY += y
                    momentCount++
                    val isBoundary = !fg[(y - 1) * gw + x] ||
                        !fg[(y + 1) * gw + x] ||
                        !fg[y * gw + (x - 1)] ||
                        !fg[y * gw + (x + 1)]
                    if (isBoundary) {
                        boundaryX[boundaryCount] = x
                        boundaryY[boundaryCount] = y
                        boundaryCount++
                    }
                }
            }
        }

        // Fallback to strong Sobel edge pixels if bright-region segmentation wasn't decisive
        if (boundaryCount < 16 && maxMag > 1f) {
            val edgeThresh = maxMag * 0.30f
            boundaryCount = 0
            sumX = 0.0
            sumY = 0.0
            momentCount = 0
            for (y in 1 until gh - 1) {
                for (x in 1 until gw - 1) {
                    if (mag[y * gw + x] >= edgeThresh) {
                        boundaryX[boundaryCount] = x
                        boundaryY[boundaryCount] = y
                        boundaryCount++
                        sumX += x
                        sumY += y
                        momentCount++
                    }
                }
            }
        }

        if (boundaryCount < 4 || momentCount == 0) return DEFAULT_QUAD

        val cx = sumX / momentCount
        val cy = sumY / momentCount

        // Compute second central moments to align with the document's principal axes
        var mu20 = 0.0
        var mu02 = 0.0
        var mu11 = 0.0
        if (fgFraction in 0.12f..0.92f) {
            for (y in 1 until gh - 1) {
                for (x in 1 until gw - 1) {
                    if (!fg[y * gw + x]) continue
                    val dx = x - cx
                    val dy = y - cy
                    mu20 += dx * dx
                    mu02 += dy * dy
                    mu11 += dx * dy
                }
            }
        } else {
            for (i in 0 until boundaryCount) {
                val dx = boundaryX[i] - cx
                val dy = boundaryY[i] - cy
                mu20 += dx * dx
                mu02 += dy * dy
                mu11 += dx * dy
            }
        }

        val theta = 0.5 * atan2(2.0 * mu11, mu20 - mu02)
        val cosT = cos(theta)
        val sinT = sin(theta)

        var varXi = 0.0
        var varEta = 0.0
        for (i in 0 until boundaryCount) {
            val dx = boundaryX[i] - cx
            val dy = boundaryY[i] - cy
            val xi = dx * cosT + dy * sinT
            val eta = -dx * sinT + dy * cosT
            varXi += xi * xi
            varEta += eta * eta
        }
        val sigmaXi = sqrt(varXi / boundaryCount).coerceAtLeast(1.0)
        val sigmaEta = sqrt(varEta / boundaryCount).coerceAtLeast(1.0)

        var minSum = Double.POSITIVE_INFINITY
        var maxSum = Double.NEGATIVE_INFINITY
        var minDiff = Double.POSITIVE_INFINITY
        var maxDiff = Double.NEGATIVE_INFINITY
        var idx1 = 0
        var idx2 = 0
        var idx3 = 0
        var idx4 = 0

        for (i in 0 until boundaryCount) {
            val dx = boundaryX[i] - cx
            val dy = boundaryY[i] - cy
            val xi = (dx * cosT + dy * sinT) / sigmaXi
            val eta = (-dx * sinT + dy * cosT) / sigmaEta
            val s = xi + eta
            val d = xi - eta
            if (s < minSum) {
                minSum = s
                idx1 = i
            }
            if (d > maxDiff) {
                maxDiff = d
                idx2 = i
            }
            if (s > maxSum) {
                maxSum = s
                idx3 = i
            }
            if (d < minDiff) {
                minDiff = d
                idx4 = i
            }
        }

        val invW = 1f / (gw - 1).coerceAtLeast(1).toFloat()
        val invH = 1f / (gh - 1).coerceAtLeast(1).toFloat()
        val candidate = QuadCorners(
            topLeft = PointF2D(boundaryX[idx1] * invW, boundaryY[idx1] * invH),
            topRight = PointF2D(boundaryX[idx2] * invW, boundaryY[idx2] * invH),
            bottomRight = PointF2D(boundaryX[idx3] * invW, boundaryY[idx3] * invH),
            bottomLeft = PointF2D(boundaryX[idx4] * invW, boundaryY[idx4] * invH),
        ).ordered()

        return if (isValidDocumentQuad(candidate)) candidate else DEFAULT_QUAD
    }

    /**
     * Checks that a normalized quad is convex, covers at least 15% of the frame,
     * and has interior angles between 45° and 135°.
     */
    fun isValidDocumentQuad(quad: QuadCorners): Boolean {
        val pts = quad.ordered().toList()
        if (pts.any { it.x !in 0f..1f || it.y !in 0f..1f }) return false

        // Convexity and interior angle check in clockwise order (screen Y-down)
        for (i in 0 until 4) {
            val prev = pts[(i + 3) % 4]
            val curr = pts[i]
            val next = pts[(i + 1) % 4]

            val ux = prev.x - curr.x
            val uy = prev.y - curr.y
            val vx = next.x - curr.x
            val vy = next.y - curr.y

            val lenU = hypot(ux, uy)
            val lenV = hypot(vx, vy)
            if (lenU < 0.05f || lenV < 0.05f) return false

            // For clockwise points in Y-down coordinates, edge1 x edge2 > 0
            val edge1x = curr.x - prev.x
            val edge1y = curr.y - prev.y
            val edge2x = next.x - curr.x
            val edge2y = next.y - curr.y
            val cross = edge1x * edge2y - edge1y * edge2x
            if (cross <= 1e-4f) return false

            val cosAngle = ((ux * vx + uy * vy) / (lenU * lenV)).coerceIn(-1f, 1f)
            val angleDeg = (acos(cosAngle) * (180.0 / PI)).toFloat()
            if (angleDeg !in MIN_INTERIOR_ANGLE_DEG..MAX_INTERIOR_ANGLE_DEG) return false
        }

        // Shoelace area
        var doubleArea = 0f
        for (i in 0 until 4) {
            val p1 = pts[i]
            val p2 = pts[(i + 1) % 4]
            doubleArea += p1.x * p2.y - p2.x * p1.y
        }
        val area = abs(doubleArea) * 0.5f
        return area >= MIN_QUAD_AREA_FRACTION
    }

    /**
     * Solves the exact 8x8 linear system (Gaussian elimination with partial pivoting)
     * for the 3x3 projective homography matrix mapping target rectangle
     * `(0, 0), (dstWidth, 0), (dstWidth, dstHeight), (0, dstHeight)` to `src`.
     */
    fun computeHomography(src: QuadCorners, dstWidth: Float, dstHeight: Float): DoubleArray {
        val ordered = src.ordered()
        val u = doubleArrayOf(0.0, dstWidth.toDouble(), dstWidth.toDouble(), 0.0)
        val v = doubleArrayOf(0.0, 0.0, dstHeight.toDouble(), dstHeight.toDouble())
        val x = doubleArrayOf(
            ordered.topLeft.x.toDouble(),
            ordered.topRight.x.toDouble(),
            ordered.bottomRight.x.toDouble(),
            ordered.bottomLeft.x.toDouble(),
        )
        val y = doubleArrayOf(
            ordered.topLeft.y.toDouble(),
            ordered.topRight.y.toDouble(),
            ordered.bottomRight.y.toDouble(),
            ordered.bottomLeft.y.toDouble(),
        )

        val a = Array(8) { DoubleArray(9) }
        for (i in 0 until 4) {
            val r0 = i * 2
            val r1 = r0 + 1
            a[r0][0] = u[i]
            a[r0][1] = v[i]
            a[r0][2] = 1.0
            a[r0][6] = -u[i] * x[i]
            a[r0][7] = -v[i] * x[i]
            a[r0][8] = x[i]

            a[r1][3] = u[i]
            a[r1][4] = v[i]
            a[r1][5] = 1.0
            a[r1][6] = -u[i] * y[i]
            a[r1][7] = -v[i] * y[i]
            a[r1][8] = y[i]
        }

        // Gaussian elimination with partial pivoting
        for (col in 0 until 8) {
            var pivot = col
            var maxAbs = abs(a[col][col])
            for (row in col + 1 until 8) {
                val candidate = abs(a[row][col])
                if (candidate > maxAbs) {
                    maxAbs = candidate
                    pivot = row
                }
            }
            if (pivot != col) {
                val tmp = a[col]
                a[col] = a[pivot]
                a[pivot] = tmp
            }
            val diag = a[col][col]
            if (abs(diag) < 1e-12) continue
            for (j in col..8) {
                a[col][j] /= diag
            }
            for (row in 0 until 8) {
                if (row == col) continue
                val factor = a[row][col]
                if (abs(factor) < 1e-15) continue
                for (j in col..8) {
                    a[row][j] -= factor * a[col][j]
                }
            }
        }

        return doubleArrayOf(
            a[0][8], a[1][8], a[2][8],
            a[3][8], a[4][8], a[5][8],
            a[6][8], a[7][8], 1.0,
        )
    }

    /**
     * Rectifies the quadrilateral `quadPixels` in `srcPixels` (`ARGB_8888`) into a
     * flat `dstWidth x dstHeight` image using bilinear interpolation and optional
     * whiteboard/document contrast enhancement.
     */
    fun warpPerspectiveArgb(
        srcPixels: IntArray,
        srcWidth: Int,
        srcHeight: Int,
        quadPixels: QuadCorners,
        dstWidth: Int,
        dstHeight: Int,
        enhanceDocument: Boolean = true,
    ): IntArray {
        val out = IntArray(dstWidth * dstHeight)
        if (srcWidth <= 0 || srcHeight <= 0 || dstWidth <= 0 || dstHeight <= 0) return out

        val h = computeHomography(
            src = quadPixels.ordered(),
            dstWidth = (dstWidth - 1).coerceAtLeast(1).toFloat(),
            dstHeight = (dstHeight - 1).coerceAtLeast(1).toFloat(),
        )

        val maxX = (srcWidth - 1).coerceAtLeast(0)
        val maxY = (srcHeight - 1).coerceAtLeast(0)

        for (dy in 0 until dstHeight) {
            val rowOff = dy * dstWidth
            val h1y = h[1] * dy + h[2]
            val h4y = h[4] * dy + h[5]
            val h7y = h[7] * dy + h[8]
            for (dx in 0 until dstWidth) {
                val denom = h[6] * dx + h7y
                val invDenom = if (abs(denom) > 1e-9) 1.0 / denom else 1.0
                val sx = ((h[0] * dx + h1y) * invDenom).toFloat().coerceIn(0f, maxX.toFloat())
                val sy = ((h[3] * dx + h4y) * invDenom).toFloat().coerceIn(0f, maxY.toFloat())

                val x0 = sx.toInt().coerceIn(0, maxX)
                val y0 = sy.toInt().coerceIn(0, maxY)
                val x1 = min(x0 + 1, maxX)
                val y1 = min(y0 + 1, maxY)
                val fx = sx - x0
                val fy = sy - y0
                val w00 = (1f - fx) * (1f - fy)
                val w10 = fx * (1f - fy)
                val w01 = (1f - fx) * fy
                val w11 = fx * fy

                val c00 = srcPixels[y0 * srcWidth + x0]
                val c10 = srcPixels[y0 * srcWidth + x1]
                val c01 = srcPixels[y1 * srcWidth + x0]
                val c11 = srcPixels[y1 * srcWidth + x1]

                val r = ((c00 ushr 16 and 0xFF) * w00 +
                    (c10 ushr 16 and 0xFF) * w10 +
                    (c01 ushr 16 and 0xFF) * w01 +
                    (c11 ushr 16 and 0xFF) * w11).roundToInt().coerceIn(0, 255)
                val g = ((c00 ushr 8 and 0xFF) * w00 +
                    (c10 ushr 8 and 0xFF) * w10 +
                    (c01 ushr 8 and 0xFF) * w01 +
                    (c11 ushr 8 and 0xFF) * w11).roundToInt().coerceIn(0, 255)
                val b = ((c00 and 0xFF) * w00 +
                    (c10 and 0xFF) * w10 +
                    (c01 and 0xFF) * w01 +
                    (c11 and 0xFF) * w11).roundToInt().coerceIn(0, 255)

                out[rowOff + dx] = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
            }
        }

        if (enhanceDocument) {
            enhanceDocumentInPlace(out)
        }
        return out
    }

    private fun enhanceDocumentInPlace(pixels: IntArray) {
        if (pixels.isEmpty()) return
        val hist = IntArray(256)
        for (c in pixels) {
            val r = c ushr 16 and 0xFF
            val g = c ushr 8 and 0xFF
            val b = c and 0xFF
            val y = (77 * r + 150 * g + 29 * b) ushr 8
            hist[y]++
        }

        val lowTarget = (pixels.size * 0.05f).toInt()
        val highTarget = (pixels.size * 0.85f).toInt()
        var acc = 0
        var low = 0
        var high = 255
        for (i in 0..255) {
            acc += hist[i]
            if (acc <= lowTarget) low = i
            if (acc <= highTarget) high = i
        }
        if (high <= low + 12) return

        val span = (high - low).toFloat()
        val lut = IntArray(256) { v ->
            val norm = ((v - low).toFloat() / span).coerceIn(0f, 1f)
            // S-curve that whitens paper highlights and deepens dark ink
            val curved = if (norm > 0.72f) {
                val t = (norm - 0.72f) / 0.28f
                0.78f + 0.22f * (1f - (1f - t) * (1f - t))
            } else {
                norm * (0.78f / 0.72f)
            }
            (curved * 255f).roundToInt().coerceIn(0, 255)
        }

        for (i in pixels.indices) {
            val c = pixels[i]
            val r = lut[c ushr 16 and 0xFF]
            val g = lut[c ushr 8 and 0xFF]
            val b = lut[c and 0xFF]
            pixels[i] = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
        }
    }

    /** Detects a document quad directly from an Android [Bitmap]. */
    fun detectDocumentQuad(bitmap: Bitmap): QuadCorners {
        val w = bitmap.width
        val h = bitmap.height
        if (w < 16 || h < 16) return DEFAULT_QUAD
        val pixels = IntArray(w * h)
        bitmap.getPixels(pixels, 0, w, 0, 0, w, h)
        val luma = ByteArray(w * h) { i ->
            val c = pixels[i]
            val r = c ushr 16 and 0xFF
            val g = c ushr 8 and 0xFF
            val b = c and 0xFF
            ((77 * r + 150 * g + 29 * b) ushr 8).toByte()
        }
        return detectDocumentQuad(luma, w, h)
    }

    /**
     * Rectifies `normalizedQuad` (`0f..1f`) from `bitmap` into a perspective-corrected
     * rectangular [Bitmap].
     */
    fun rectifyDocument(
        bitmap: Bitmap,
        normalizedQuad: QuadCorners,
        enhance: Boolean = true,
    ): Bitmap {
        val w = bitmap.width
        val h = bitmap.height
        val quadPx = normalizedQuad.ordered().toPixelSpace(w, h)

        val topW = hypot(quadPx.topRight.x - quadPx.topLeft.x, quadPx.topRight.y - quadPx.topLeft.y)
        val bottomW = hypot(quadPx.bottomRight.x - quadPx.bottomLeft.x, quadPx.bottomRight.y - quadPx.bottomLeft.y)
        val leftH = hypot(quadPx.bottomLeft.x - quadPx.topLeft.x, quadPx.bottomLeft.y - quadPx.topLeft.y)
        val rightH = hypot(quadPx.bottomRight.x - quadPx.topRight.x, quadPx.bottomRight.y - quadPx.topRight.y)

        var dstW = max(topW, bottomW).roundToInt().coerceAtLeast(64)
        var dstH = max(leftH, rightH).roundToInt().coerceAtLeast(64)
        val longest = max(dstW, dstH)
        if (longest > 2200) {
            val scale = 2200f / longest.toFloat()
            dstW = (dstW * scale).roundToInt().coerceAtLeast(64)
            dstH = (dstH * scale).roundToInt().coerceAtLeast(64)
        }

        val srcPixels = IntArray(w * h)
        bitmap.getPixels(srcPixels, 0, w, 0, 0, w, h)
        val dstPixels = warpPerspectiveArgb(
            srcPixels = srcPixels,
            srcWidth = w,
            srcHeight = h,
            quadPixels = quadPx,
            dstWidth = dstW,
            dstHeight = dstH,
            enhanceDocument = enhance,
        )
        return Bitmap.createBitmap(dstPixels, dstW, dstH, Bitmap.Config.ARGB_8888)
    }

    private fun otsuThreshold(values: IntArray): Int {
        val hist = IntArray(256)
        for (v in values) hist[v.coerceIn(0, 255)]++
        val total = values.size
        var sumAll = 0.0
        for (i in 0..255) sumAll += i.toDouble() * hist[i]

        var sumB = 0.0
        var wB = 0
        var maxVar = -1.0
        var threshold = 128

        for (t in 0..255) {
            wB += hist[t]
            if (wB == 0) continue
            val wF = total - wB
            if (wF == 0) break
            sumB += t.toDouble() * hist[t]
            val mB = sumB / wB
            val mF = (sumAll - sumB) / wF
            val diff = mB - mF
            val between = wB.toDouble() * wF.toDouble() * diff * diff
            if (between > maxVar) {
                maxVar = between
                threshold = t
            }
        }
        return threshold
    }
}
