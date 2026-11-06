// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer

/** Caches rotated quad vertices for the encoder fit and UI crop. */
internal class FrameQuad(
    private val srcWidth: Int,
    private val srcHeight: Int,
    private val dstWidth: Int,
    private val dstHeight: Int,
) {
    val buffer: FloatBuffer = ByteBuffer.allocateDirect(16 * 4)
        .order(ByteOrder.nativeOrder()).asFloatBuffer()
    private var rotation = -1
    private var swapsAxes = false
    private var mirrored = false

    fun update(degrees: Int, sourceSwapsAxes: Boolean, mirror: Boolean): Boolean {
        val normalized = ((degrees % 360) + 360) % 360
        if (rotation == normalized && swapsAxes == sourceSwapsAxes && mirrored == mirror) return false
        rotation = normalized
        swapsAxes = sourceSwapsAxes
        mirrored = mirror

        val uvWidth = if (sourceSwapsAxes) srcHeight.toFloat() else srcWidth.toFloat()
        val uvHeight = if (sourceSwapsAxes) srcWidth.toFloat() else srcHeight.toFloat()
        val width = if (normalized % 180 != 0) uvHeight else uvWidth
        val height = if (normalized % 180 != 0) uvWidth else uvHeight
        val aspect = (width / height.coerceAtLeast(1f)).coerceAtLeast(0.01f)
        val targetAspect = (dstWidth.toFloat() / dstHeight.coerceAtLeast(1)).coerceAtLeast(0.01f)
        val scaleX = if (aspect > targetAspect) 1f else aspect / targetAspect
        val sx = scaleX * (if (mirror) -1f else 1f)
        val sy = if (aspect > targetAspect) targetAspect / aspect else 1f
        val uvs = when (normalized) {
            90 -> UV_90
            180 -> UV_180
            270 -> UV_270
            else -> UV_0
        }
        buffer.position(0)
        for (i in 0 until 4) {
            buffer.put(if (i % 2 == 0) -sx else sx)
            buffer.put(if (i < 2) -sy else sy)
            buffer.put(uvs[i * 2])
            buffer.put(uvs[i * 2 + 1])
        }
        buffer.position(0)
        return true
    }

    private companion object {
        val UV_0 = floatArrayOf(0f, 0f, 1f, 0f, 0f, 1f, 1f, 1f)
        val UV_90 = floatArrayOf(0f, 1f, 0f, 0f, 1f, 1f, 1f, 0f)
        val UV_180 = floatArrayOf(1f, 1f, 0f, 1f, 1f, 0f, 0f, 0f)
        val UV_270 = floatArrayOf(1f, 0f, 1f, 1f, 0f, 0f, 0f, 1f)
    }
}
