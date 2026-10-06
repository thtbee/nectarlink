// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.content.Context
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Display
import android.view.WindowManager
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorConfig
import kotlin.math.max
import kotlin.math.min

/**
 * The screen as H.264 for a PC (docs/protocol/mirror.md): the projection
 * draws into the hardware encoder's input surface, and each encoded
 * picture goes straight to the stream. When the network is behind, the
 * stream drops pictures and this asks the encoder for a keyframe. Turning
 * the phone restarts the encoder at the new size.
 */
internal class ScreenEncoder(
    context: Context,
    private val projection: MediaProjection,
    private val stream: MirrorStream,
    private val maxSize: Int,
    private val fps: Int,
    private val bitrate: Int,
    private val onEnded: () -> Unit,
) {
    private val context = context.applicationContext
    private val displays = context.getSystemService(DisplayManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    @Volatile private var running = true
    @Volatile private var keyframeWanted = false
    @Volatile private var resizeWanted = false
    @Volatile private var size: Size = screenSize()

    private data class Size(val width: Int, val height: Int, val dpi: Int)

    private val rotation = object : DisplayManager.DisplayListener {
        override fun onDisplayChanged(displayId: Int) {
            if (displayId != Display.DEFAULT_DISPLAY) return
            val now = screenSize()
            if (now.width != size.width || now.height != size.height) resizeWanted = true
        }
        override fun onDisplayAdded(displayId: Int) {}
        override fun onDisplayRemoved(displayId: Int) {}
    }

    fun start() {
        displays.registerDisplayListener(rotation, main)
        Thread(::run, "screen-encoder").start()
    }

    /** Stops (the encoder thread ends and calls `onEnded`). */
    fun stop() {
        running = false
    }

    /** The PC needs a fresh start. */
    fun requestKeyframe() {
        keyframeWanted = true
    }

    private fun run() {
        var display: VirtualDisplay? = null
        try {
            while (running) {
                size = screenSize()
                resizeWanted = false
                val codec = encoder(size)
                val surface = codec.createInputSurface()
                codec.start()
                val current = display
                if (current == null) {
                    display = projection.createVirtualDisplay(
                        "Nectarlink", size.width, size.height, size.dpi,
                        DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null,
                    )
                } else {
                    current.resize(size.width, size.height, size.dpi)
                    current.surface = surface
                }
                val closed = stream.send(VideoPacketKind.CONFIG, 0u, mirrorConfig(size.width.toUInt(), size.height.toUInt(), 0u)) ==
                    MirrorSendResult.CLOSED || drain(codec)
                runCatching { codec.stop() }
                codec.release()
                surface.release()
                if (closed) break
            }
        } catch (e: Exception) {
            Log.w(TAG, "screen sharing failed", e)
        } finally {
            running = false
            main.post { displays.unregisterDisplayListener(rotation) }
            display?.release()
            stream.end()
            onEnded()
        }
    }

    /** Sends what the encoder makes until the size changes (false) or it's over (true). */
    private fun drain(codec: MediaCodec): Boolean {
        val info = MediaCodec.BufferInfo()
        // The parameter sets, sent again with each keyframe so the PC can
        // start from any of them.
        var parameters = ByteArray(0)
        while (running && !resizeWanted) {
            if (keyframeWanted) {
                keyframeWanted = false
                codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
            }
            val index = codec.dequeueOutputBuffer(info, WAIT_US)
            if (index < 0) continue
            val bytes = ByteArray(info.size)
            codec.getOutputBuffer(index)?.let { buffer ->
                buffer.position(info.offset)
                buffer.get(bytes)
            }
            codec.releaseOutputBuffer(index, false)
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return true
            if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                parameters = bytes
                continue
            }
            if (bytes.isEmpty()) continue
            val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
            val result = stream.send(
                if (key) VideoPacketKind.KEYFRAME else VideoPacketKind.FRAME,
                info.presentationTimeUs.toULong(),
                if (key) parameters + bytes else bytes,
            )
            when (result) {
                // A video stream never drops without asking for a keyframe.
                MirrorSendResult.NEED_KEYFRAME, MirrorSendResult.DROPPED -> keyframeWanted = true
                MirrorSendResult.CLOSED -> return true
                MirrorSendResult.QUEUED -> {}
            }
        }
        return !running
    }

    private fun encoder(size: Size): MediaCodec {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, size.width, size.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            // Keyframes when the PC asks; a periodic one heals the rest.
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
            // A still screen still sends now and then, so the PC isn't stale.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            // Real time, no reordering.
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) setInteger(MediaFormat.KEY_LATENCY, 1)
            // What the PC assumes when it turns the picture into RGB.
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
        }
        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        return codec
    }

    /** The screen's size scaled to fit `maxSize`, in multiples of 16 (what encoders like). */
    private fun screenSize(): Size {
        val windows = context.getSystemService(WindowManager::class.java)
        val (w, h) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windows.maximumWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windows.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels to metrics.heightPixels
        }
        val scale = min(1.0, maxSize.toDouble() / max(w, h))
        fun fit(v: Int) = max(16, ((v * scale) / 16).toInt() * 16)
        return Size(fit(w), fit(h), context.resources.displayMetrics.densityDpi)
    }

    companion object {
        private const val TAG = "ScreenEncoder"
        private const val WAIT_US = 50_000L
    }
}
