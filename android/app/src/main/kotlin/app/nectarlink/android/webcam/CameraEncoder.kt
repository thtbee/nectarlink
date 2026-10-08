// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.content.Context
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.hardware.display.DisplayManager
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.util.Size
import android.view.Display
import android.view.OrientationEventListener
import android.view.Surface
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.webcamConfig
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

data class CameraLiveState(
    val width: Int = 1280,
    val height: Int = 720,
    val fps: Int = 30,
    val camera: String = "back",
    val zoomRatio: Float = 1f,
    val minZoomRatio: Float = 1f,
    val maxZoomRatio: Float = 4f,
    val hasFlash: Boolean = false,
    val torchOn: Boolean = false,
    val supports4k: Boolean = false,
)

/**
 * Captures frames from the phone's front or back camera with CameraX and
 * encodes them into low-latency Annex B H.264 access units with hardware
 * [MediaCodec] (`COLOR_FormatSurface`) for `docs/protocol/webcam.md`.
 *
 * Switching cameras (`front` / `back`) or resolution (`720p` / `1080p` /
 * `4K`) rebinds CameraX and starts a fresh [MediaCodec] on the same
 * [MirrorStream], emitting a new `WebcamConfig` packet followed by an IDR
 * keyframe without dropping the QUIC stream.
 */
internal class CameraEncoder(
    context: Context,
    private val lifecycleOwner: LifecycleOwner,
    private val stream: MirrorStream,
    initialHeight: Int,
    private val fps: Int,
    initialCamera: String,
    initialZoomRatio: Float = 1f,
    private val onStateChanged: (CameraLiveState) -> Unit,
    private val onEnded: () -> Unit,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val mainExecutor = ContextCompat.getMainExecutor(this.context)
    private val surfaceExecutor = Executors.newSingleThreadExecutor()
    private val displayManager = this.context.getSystemService(DisplayManager::class.java)

    val supports4k: Boolean = supports4kEncoder()

    @Volatile private var running = true
    @Volatile private var keyframeWanted = false
    @Volatile private var targetHeight: Int = normalizeHeight(initialHeight, supports4k)
    @Volatile private var cameraFacing: String = if (initialCamera == "front") "front" else "back"
    @Volatile private var activeCodec: ActiveCodec? = null
    @Volatile private var currentSurfaceRotation: Int = readDisplayRotation()

    private var cameraProvider: ProcessCameraProvider? = null
    private var boundCamera: Camera? = null
    private var encoderPreview: Preview? = null
    private var uiPreview: Preview? = null
    private var pendingUiSurfaceProvider: Preview.SurfaceProvider? = null
    private var liveState = CameraLiveState(
        width = widthForHeight(targetHeight),
        height = targetHeight,
        fps = fps,
        camera = cameraFacing,
        zoomRatio = initialZoomRatio.coerceAtLeast(1f),
        maxZoomRatio = cameraMaxZoom(this.context, cameraFacing),
        hasFlash = cameraHasFlash(this.context, cameraFacing),
        supports4k = supports4k,
    )

    private val displayListener = object : DisplayManager.DisplayListener {
        override fun onDisplayAdded(displayId: Int) {}
        override fun onDisplayRemoved(displayId: Int) {}
        override fun onDisplayChanged(displayId: Int) {
            if (displayId != Display.DEFAULT_DISPLAY || !running) return
            applySurfaceRotation(readDisplayRotation())
        }
    }

    private val orientationListener = object : OrientationEventListener(this.context) {
        override fun onOrientationChanged(orientation: Int) {
            if (orientation == ORIENTATION_UNKNOWN || !running) return
            val snapped = snapOrientationToSurfaceRotation(orientation, currentSurfaceRotation)
            applySurfaceRotation(snapped)
        }
    }

    private fun readDisplayRotation(): Int =
        displayManager?.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: Surface.ROTATION_0

    private fun applySurfaceRotation(rotation: Int) {
        if (rotation == currentSurfaceRotation) return
        currentSurfaceRotation = rotation
        encoderPreview?.targetRotation = rotation
        uiPreview?.targetRotation = rotation
        requestKeyframe()
    }

    fun start() {
        main.post {
            if (!running) return@post
            currentSurfaceRotation = readDisplayRotation()
            runCatching { displayManager?.registerDisplayListener(displayListener, main) }
            runCatching { if (orientationListener.canDetectOrientation()) orientationListener.enable() }
            val future = ProcessCameraProvider.getInstance(context)
            future.addListener({
                if (!running) return@addListener
                val provider = runCatching { future.get() }.getOrElse { e ->
                    Log.w(TAG, "can't get ProcessCameraProvider", e)
                    stop()
                    return@addListener
                }
                cameraProvider = provider
                bindCamera()
            }, mainExecutor)
        }
    }

    fun stop() {
        if (!running) return
        running = false
        main.post {
            runCatching { displayManager?.unregisterDisplayListener(displayListener) }
            runCatching { orientationListener.disable() }
            runCatching { cameraProvider?.unbindAll() }
            boundCamera = null
            encoderPreview = null
            uiPreview = null
        }
        surfaceExecutor.execute {
            activeCodec?.stop()
            activeCodec = null
            runCatching { stream.end() }
            main.post { onEnded() }
        }
        surfaceExecutor.shutdown()
    }

    fun requestKeyframe() {
        keyframeWanted = true
    }

    fun setCamera(camera: String) {
        val normalized = if (camera == "front") "front" else "back"
        main.post {
            if (!running || cameraFacing == normalized) return@post
            cameraFacing = normalized
            bindCamera()
        }
    }

    fun setResolutionHeight(height: Int) {
        val normalized = normalizeHeight(height, supports4k)
        main.post {
            if (!running || targetHeight == normalized) return@post
            targetHeight = normalized
            bindCamera()
        }
    }

    fun setZoomRatio(ratio: Float) {
        main.post {
            val cam = boundCamera ?: return@post
            val minZ = liveState.minZoomRatio
            val maxZ = liveState.maxZoomRatio
            val clamped = ratio.coerceIn(minZ, maxZ)
            cam.cameraControl.setZoomRatio(clamped)
            updateState { it.copy(zoomRatio = clamped) }
        }
    }

    fun setTorch(on: Boolean) {
        main.post {
            val cam = boundCamera ?: return@post
            if (!cam.cameraInfo.hasFlashUnit()) return@post
            cam.cameraControl.enableTorch(on)
            updateState { it.copy(torchOn = on) }
        }
    }

    fun setUiSurfaceProvider(provider: Preview.SurfaceProvider?) {
        main.post {
            pendingUiSurfaceProvider = provider
            uiPreview?.setSurfaceProvider(provider)
        }
    }

    private fun updateState(transform: (CameraLiveState) -> CameraLiveState) {
        liveState = transform(liveState)
        onStateChanged(liveState)
    }

    private fun bindCamera() {
        val provider = cameraProvider ?: return
        if (!running) return

        val desiredHeight = targetHeight
        val desiredWidth = widthForHeight(desiredHeight)
        val desiredSize = Size(desiredWidth, desiredHeight)
        val resSelector = ResolutionSelector.Builder()
            .setAspectRatioStrategy(AspectRatioStrategy.RATIO_16_9_FALLBACK_AUTO_STRATEGY)
            .setResolutionStrategy(
                ResolutionStrategy(
                    desiredSize,
                    ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
                ),
            )
            .build()

        val rotation = currentSurfaceRotation
        val encPreview = Preview.Builder()
            .setResolutionSelector(resSelector)
            .setTargetRotation(rotation)
            .build()
        encoderPreview = encPreview

        val facingForRequest = cameraFacing
        encPreview.setSurfaceProvider(surfaceExecutor) { request ->
            if (!running) {
                request.willNotProvideSurface()
                return@setSurfaceProvider
            }
            activeCodec?.stop()
            activeCodec = null

            val srcW = request.resolution.width
            val srcH = request.resolution.height
            val outH = desiredHeight
            val outW = widthForHeight(outH)
            val started = startCodecForResolution(srcW, srcH, outW, outH, facingForRequest)
            val inputSurface = started?.pipe?.inputSurface
            if (started == null || inputSurface == null) {
                started?.stop()
                request.willNotProvideSurface()
                if (targetHeight > 1080) {
                    // Fall back from 4K to 1080p if the codec rejected 4K.
                    main.post { setResolutionHeight(1080) }
                } else {
                    stop()
                }
                return@setSurfaceProvider
            }
            activeCodec = started
            request.setTransformationInfoListener(surfaceExecutor) { info ->
                started.pipe.updateTransform(info.rotationDegrees, info.isMirroring)
            }
            main.post {
                updateState { it.copy(width = started.width, height = started.height, camera = facingForRequest) }
            }
            request.provideSurface(inputSurface, surfaceExecutor) {
                request.clearTransformationInfoListener()
                if (activeCodec === started) {
                    activeCodec = null
                }
                started.stop()
            }
        }

        val previewForUi = Preview.Builder()
            .setResolutionSelector(
                ResolutionSelector.Builder()
                    .setAspectRatioStrategy(AspectRatioStrategy.RATIO_16_9_FALLBACK_AUTO_STRATEGY)
                    .build(),
            )
            .setTargetRotation(rotation)
            .build()
            .also { it.setSurfaceProvider(pendingUiSurfaceProvider) }
        uiPreview = previewForUi

        val requestedSelector = if (cameraFacing == "front") {
            CameraSelector.DEFAULT_FRONT_CAMERA
        } else {
            CameraSelector.DEFAULT_BACK_CAMERA
        }
        val selector = if (runCatching { provider.hasCamera(requestedSelector) }.getOrDefault(false)) {
            requestedSelector
        } else if (runCatching { provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA) }.getOrDefault(false)) {
            cameraFacing = "back"
            CameraSelector.DEFAULT_BACK_CAMERA
        } else {
            cameraFacing = "front"
            CameraSelector.DEFAULT_FRONT_CAMERA
        }

        provider.unbindAll()
        val camera = runCatching {
            provider.bindToLifecycle(lifecycleOwner, selector, encPreview, previewForUi)
        }.getOrElse {
            // Fallback for single-stream legacy cameras: bind encPreview alone.
            runCatching {
                provider.bindToLifecycle(lifecycleOwner, selector, encPreview)
            }.getOrNull()
        }

        if (camera == null) {
            Log.w(TAG, "failed to bind camera")
            stop()
            return
        }
        boundCamera = camera
        val zoomState = camera.cameraInfo.zoomState.value
        val minZ = zoomState?.minZoomRatio ?: 1f
        val maxZ = (zoomState?.maxZoomRatio ?: cameraMaxZoom(context, cameraFacing)).coerceAtLeast(2f)
        val initialZ = liveState.zoomRatio.coerceIn(minZ, maxZ)
        if (initialZ > minZ) {
            camera.cameraControl.setZoomRatio(initialZ)
        }
        val hasFlash = camera.cameraInfo.hasFlashUnit() && cameraHasFlash(context, cameraFacing)
        updateState {
            it.copy(
                camera = cameraFacing,
                zoomRatio = initialZ,
                minZoomRatio = minZ,
                maxZoomRatio = maxZ,
                hasFlash = hasFlash,
                torchOn = false,
            )
        }
    }

    private fun startCodecForResolution(
        srcWidth: Int,
        srcHeight: Int,
        outWidth: Int,
        outHeight: Int,
        camera: String,
    ): ActiveCodec? {
        val bitrate = bitrateForHeight(outHeight)
        return runCatching {
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, outWidth, outHeight).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                setInteger(MediaFormat.KEY_FRAME_RATE, fps)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, I_FRAME_INTERVAL_SEC)
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000L)
                setInteger(MediaFormat.KEY_PRIORITY, 0)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
                    setInteger(MediaFormat.KEY_LATENCY, 1)
                }
                setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
                setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
                setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
            }
            val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            val codecSurface = codec.createInputSurface()
            codec.start()
            val pipe = GlSurfacePipe(
                srcWidth = srcWidth,
                srcHeight = srcHeight,
                dstWidth = outWidth,
                dstHeight = outHeight,
                encoderInputSurface = codecSurface,
                initialRotationDegrees = 0,
                initialMirror = false,
            )
            ActiveCodec(codec, codecSurface, pipe, outWidth, outHeight, camera).also { it.startDrain() }
        }.onFailure { e ->
            Log.w(TAG, "can't start H.264 encoder for ${outWidth}x${outHeight}", e)
        }.getOrNull()
    }

    private inner class ActiveCodec(
        val codec: MediaCodec,
        val codecSurface: Surface,
        val pipe: GlSurfacePipe,
        val width: Int,
        val height: Int,
        val camera: String,
    ) {
        private val active = AtomicBoolean(true)
        private var drainThread: Thread? = null

        fun startDrain() {
            drainThread = Thread(::drainLoop, "webcam-encoder-${width}x${height}").also { it.start() }
        }

        fun stop() {
            if (!active.compareAndSet(true, false)) return
            runCatching { pipe.release() }
            drainThread?.let { runCatching { it.join(500) } }
            runCatching { codec.stop() }
            runCatching { codec.release() }
            runCatching { codecSurface.release() }
        }

        private fun drainLoop() {
            val cfgBytes = webcamConfig(width.toUInt(), height.toUInt(), camera, fps.toUInt())
            if (stream.send(VideoPacketKind.CONFIG, 0u, cfgBytes) == MirrorSendResult.CLOSED) {
                this@CameraEncoder.stop()
                return
            }
            val info = MediaCodec.BufferInfo()
            var parameters = ByteArray(0)
            try {
                while (active.get() && running) {
                    if (keyframeWanted) {
                        keyframeWanted = false
                        runCatching {
                            codec.setParameters(
                                Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) },
                            )
                        }
                    }
                    val index = codec.dequeueOutputBuffer(info, WAIT_US)
                    if (index < 0) continue
                    val bytes = ByteArray(info.size)
                    codec.getOutputBuffer(index)?.let { buf ->
                        buf.position(info.offset)
                        buf.get(bytes)
                    }
                    codec.releaseOutputBuffer(index, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        parameters = bytes
                        continue
                    }
                    if (bytes.isEmpty()) continue
                    val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    val payload = ensureAudAndParameters(bytes, parameters, key)
                    val result = stream.send(
                        if (key) VideoPacketKind.KEYFRAME else VideoPacketKind.FRAME,
                        info.presentationTimeUs.toULong(),
                        payload,
                    )
                    when (result) {
                        MirrorSendResult.NEED_KEYFRAME, MirrorSendResult.DROPPED -> keyframeWanted = true
                        MirrorSendResult.CLOSED -> {
                            this@CameraEncoder.stop()
                            return
                        }
                        MirrorSendResult.QUEUED -> {}
                    }
                }
            } catch (e: Exception) {
                if (active.get() && running) {
                    Log.w(TAG, "webcam encoder drain interrupted", e)
                }
            }
        }
    }

    companion object {
        private const val TAG = "CameraEncoder"
        private const val WAIT_US = 40_000L
        private const val I_FRAME_INTERVAL_SEC = 2

        /** Annex B Access Unit Delimiter NAL (`00 00 00 01 09 F0`). */
        internal val AUD_NAL = byteArrayOf(0x00, 0x00, 0x00, 0x01, 0x09, 0xF0.toByte())

        fun widthForHeight(height: Int): Int = when (height) {
            2160 -> 3840
            1080 -> 1920
            else -> 1280
        }

        fun bitrateForHeight(height: Int): Int = when {
            height >= 2160 -> 20_000_000
            height >= 1080 -> 8_000_000
            else -> 4_000_000
        }

        fun normalizeHeight(height: Int, supports4k: Boolean): Int = when {
            height >= 2160 && supports4k -> 2160
            height >= 1080 -> 1080
            else -> 720
        }

        /** Maps physical tilt degrees (0..359) to [Surface] rotation with hysteresis. */
        internal fun snapOrientationToSurfaceRotation(orientation: Int, currentRotation: Int): Int {
            val currentDeg = when (currentRotation) {
                Surface.ROTATION_0 -> 0
                Surface.ROTATION_270 -> 90
                Surface.ROTATION_180 -> 180
                Surface.ROTATION_90 -> 270
                else -> 0
            }
            val diff = kotlin.math.min(
                kotlin.math.abs(orientation - currentDeg),
                360 - kotlin.math.abs(orientation - currentDeg),
            )
            // 55-degree hysteresis band avoids flipping near diagonal angles.
            if (diff < 55) return currentRotation
            return when {
                orientation >= 315 || orientation < 45 -> Surface.ROTATION_0
                orientation in 45..134 -> Surface.ROTATION_270
                orientation in 135..224 -> Surface.ROTATION_180
                else -> Surface.ROTATION_90
            }
        }

        /** Checks whether the requested camera (`front` or `back`) has a flash unit. */
        fun cameraHasFlash(context: Context, camera: String): Boolean = runCatching {
            val cm = context.getSystemService(CameraManager::class.java) ?: return false
            val wantedFacing = if (camera == "front") {
                CameraCharacteristics.LENS_FACING_FRONT
            } else {
                CameraCharacteristics.LENS_FACING_BACK
            }
            cm.cameraIdList.any { id ->
                val chars = cm.getCameraCharacteristics(id)
                chars.get(CameraCharacteristics.LENS_FACING) == wantedFacing &&
                    chars.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
            }
        }.getOrDefault(false)

        /** Queries the maximum digital zoom ratio supported by the requested camera (`front` or `back`). */
        fun cameraMaxZoom(context: Context, camera: String): Float = runCatching {
            val cm = context.getSystemService(CameraManager::class.java) ?: return 4f
            val wantedFacing = if (camera == "front") {
                CameraCharacteristics.LENS_FACING_FRONT
            } else {
                CameraCharacteristics.LENS_FACING_BACK
            }
            for (id in cm.cameraIdList) {
                val chars = cm.getCameraCharacteristics(id)
                if (chars.get(CameraCharacteristics.LENS_FACING) == wantedFacing) {
                    val rangeMax = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                        chars.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)?.upper
                    } else {
                        null
                    }
                    val maxZ = rangeMax
                        ?: chars.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM)
                        ?: 4f
                    return maxZ.coerceIn(2f, 8f)
                }
            }
            4f
        }.getOrDefault(4f)

        /** Checks whether any hardware AVC encoder on this device supports 3840×2160. */
        fun supports4kEncoder(): Boolean = runCatching {
            val list = MediaCodecList(MediaCodecList.REGULAR_CODECS)
            list.codecInfos.any { info ->
                info.isEncoder &&
                    info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_AVC, ignoreCase = true) } &&
                    runCatching {
                        info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                            .videoCapabilities?.isSizeSupported(3840, 2160) == true
                    }.getOrDefault(false)
            }
        }.getOrDefault(false)

        /**
         * Guarantees each H.264 access unit starts with an Annex B Access
         * Unit Delimiter (`00 00 00 01 09 F0`) and that keyframes include
         * the cached SPS/PPS (`parameters`) right after the AUD so the PC's
         * Media Foundation decoder flushes every frame immediately.
         */
        internal fun ensureAudAndParameters(
            bytes: ByteArray,
            parameters: ByteArray,
            isKeyframe: Boolean,
        ): ByteArray {
            val stripped = stripLeadingAud(bytes)
            return if (isKeyframe && parameters.isNotEmpty()) {
                val cleanParams = stripLeadingAud(parameters)
                ByteArray(AUD_NAL.size + cleanParams.size + stripped.size).also { out ->
                    System.arraycopy(AUD_NAL, 0, out, 0, AUD_NAL.size)
                    System.arraycopy(cleanParams, 0, out, AUD_NAL.size, cleanParams.size)
                    System.arraycopy(stripped, 0, out, AUD_NAL.size + cleanParams.size, stripped.size)
                }
            } else {
                ByteArray(AUD_NAL.size + stripped.size).also { out ->
                    System.arraycopy(AUD_NAL, 0, out, 0, AUD_NAL.size)
                    System.arraycopy(stripped, 0, out, AUD_NAL.size, stripped.size)
                }
            }
        }

        private fun stripLeadingAud(data: ByteArray): ByteArray {
            if (data.size >= 6 &&
                data[0] == 0.toByte() && data[1] == 0.toByte() &&
                data[2] == 0.toByte() && data[3] == 1.toByte() &&
                (data[4].toInt() and 0x1F) == 9
            ) {
                return data.copyOfRange(6, data.size)
            }
            if (data.size >= 5 &&
                data[0] == 0.toByte() && data[1] == 0.toByte() &&
                data[2] == 1.toByte() &&
                (data[3].toInt() and 0x1F) == 9
            ) {
                return data.copyOfRange(5, data.size)
            }
            return data
        }
    }
}
