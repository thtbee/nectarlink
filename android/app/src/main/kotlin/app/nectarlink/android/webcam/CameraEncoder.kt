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
import app.nectarlink.core.webcamConfigCodec
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
 * encodes them into low-latency Annex B H.264 or negotiated HEVC access units with hardware
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
    private val preferredCodec: String = "h264",
    initialZoomRatio: Float = 1f,
    private val onStateChanged: (CameraLiveState) -> Unit,
    private val onEnded: () -> Unit,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val mainExecutor = ContextCompat.getMainExecutor(this.context)
    private val surfaceThread = Executors.newSingleThreadExecutor()

    /**
     * Where the encoder's surface work runs. CameraX can still hand it a
     * task (a surface it's done with) after [stop] shut the thread down, as
     * it unbinds on the main thread; that one is dropped instead of thrown.
     */
    private val surfaceExecutor = java.util.concurrent.Executor { task ->
        try {
            surfaceThread.execute(task)
        } catch (_: java.util.concurrent.RejectedExecutionException) {
        }
    }
    private val displayManager = this.context.getSystemService(DisplayManager::class.java)

    @Volatile private var running = true
    @Volatile private var keyframeWanted = false
    @Volatile private var hevcFailed = false
    @Volatile private var cameraFacing: String = if (initialCamera == "front") "front" else "back"

    /** Whether the camera in use and the encoder can both do 4K. */
    @Volatile var supports4k: Boolean = supports4k(this.context, cameraFacing)
        private set
    @Volatile private var targetHeight: Int = normalizeHeight(initialHeight, supports4k)
    @Volatile private var activeCodec: ActiveCodec? = null
    @Volatile private var currentSurfaceRotation: Int = readDisplayRotation()
    @Volatile private var currentSensorRotation: Int = 0
    @Volatile private var currentDisplayRotation: Int = surfaceRotationToDegrees(currentSurfaceRotation)

    private var cameraProvider: ProcessCameraProvider? = null
    private var boundCamera: Camera? = null
    private var encoderPreview: Preview? = null
    @Volatile private var pendingUiPreview: WebcamPreviewTarget? = null
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
            if (!orientationListener.canDetectOrientation()) {
                applySurfaceRotation(readDisplayRotation())
            }
        }
    }

    private val orientationListener = object : OrientationEventListener(this.context) {
        override fun onOrientationChanged(orientation: Int) {
            if (orientation == ORIENTATION_UNKNOWN || !running) return
            val snapped = snapOrientationToSurfaceRotation(orientation, currentSurfaceRotation)
            currentDisplayRotation = surfaceRotationToDegrees(snapped)
            activeCodec?.pipe?.setTransform(currentSensorRotation, currentDisplayRotation)
            applySurfaceRotation(snapped)
        }
    }

    private fun readDisplayRotation(): Int =
        displayManager?.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: Surface.ROTATION_0

    private fun surfaceRotationToDegrees(rotation: Int): Int = when (rotation) {
        Surface.ROTATION_90 -> 90
        Surface.ROTATION_180 -> 180
        Surface.ROTATION_270 -> 270
        else -> 0
    }

    private fun applySurfaceRotation(rotation: Int) {
        if (rotation == currentSurfaceRotation) return
        currentSurfaceRotation = rotation
        currentDisplayRotation = surfaceRotationToDegrees(rotation)
        activeCodec?.pipe?.setTransform(currentSensorRotation, currentDisplayRotation)
        encoderPreview?.targetRotation = rotation
        requestKeyframe()
    }

    fun start() {
        main.post {
            if (!running) return@post
            currentSurfaceRotation = readDisplayRotation()
            currentDisplayRotation = surfaceRotationToDegrees(currentSurfaceRotation)
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
        }
        surfaceExecutor.execute {
            activeCodec?.stop()
            activeCodec = null
            runCatching { stream.end() }
            main.post { onEnded() }
        }
        surfaceThread.shutdown()
    }

    fun requestKeyframe() {
        keyframeWanted = true
    }

    fun setCamera(camera: String) {
        val normalized = if (camera == "front") "front" else "back"
        main.post {
            if (!running || cameraFacing == normalized) return@post
            cameraFacing = normalized
            // The other camera may not do 4K (front cameras often don't).
            supports4k = supports4k(context, normalized)
            targetHeight = normalizeHeight(targetHeight, supports4k)
            bindCamera()
        }
    }

    fun setResolutionHeight(height: Int) {
        main.post {
            // After any camera switch queued before it, which can change what 4K means.
            val normalized = normalizeHeight(height, supports4k)
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

    fun setUiPreview(target: WebcamPreviewTarget?) {
        main.post {
            pendingUiPreview = target
            activeCodec?.pipe?.setPreviewTarget(target)
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
                currentSensorRotation = info.rotationDegrees
                currentDisplayRotation = surfaceRotationToDegrees(encPreview.targetRotation)
                started.pipe.setTransform(currentSensorRotation, currentDisplayRotation)
                started.pipe.updateTransform(info.rotationDegrees, info.isMirroring)
            }
            main.post {
                if (activeCodec === started) started.pipe.setPreviewTarget(pendingUiPreview)
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
            provider.bindToLifecycle(lifecycleOwner, selector, encPreview)
        }.onFailure { Log.w(TAG, "can't bind webcam camera output", it) }.getOrNull()

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
                supports4k = supports4k,
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
        if (preferredCodec == "h265" && !hevcFailed) {
            val name = hevcEncoder(outWidth, outHeight, fps)
            if (name != null) {
                startCodec(srcWidth, srcHeight, outWidth, outHeight, camera, "h265", name)?.let { return it }
            }
            Log.i(TAG, "HEVC unavailable for ${outWidth}x${outHeight}; using H.264")
        }
        return startCodec(srcWidth, srcHeight, outWidth, outHeight, camera, "h264", null)
    }

    private fun startCodec(
        srcWidth: Int,
        srcHeight: Int,
        outWidth: Int,
        outHeight: Int,
        camera: String,
        codecName: String,
        encoderName: String?,
    ): ActiveCodec? {
        val mime = if (codecName == "h265") MediaFormat.MIMETYPE_VIDEO_HEVC else MediaFormat.MIMETYPE_VIDEO_AVC
        val bitrate = bitrateForHeight(outHeight)
        var codec: MediaCodec? = null
        var codecSurface: Surface? = null
        var pipe: GlSurfacePipe? = null
        return runCatching {
            val format = MediaFormat.createVideoFormat(mime, outWidth, outHeight).apply {
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
                if (codecName == "h265") {
                    setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.HEVCProfileMain)
                }
            }
            val startedCodec = if (encoderName != null) MediaCodec.createByCodecName(encoderName) else MediaCodec.createEncoderByType(mime)
            codec = startedCodec
            configureEncoder(startedCodec, format)
            val surface = startedCodec.createInputSurface()
            codecSurface = surface
            startedCodec.start()
            val startedPipe = GlSurfacePipe(
                srcWidth = srcWidth,
                srcHeight = srcHeight,
                dstWidth = outWidth,
                dstHeight = outHeight,
                encoderInputSurface = surface,
                initialRotationDegrees = currentSensorRotation,
                initialMirror = false,
            ).also {
                it.setTransform(currentSensorRotation, currentDisplayRotation)
            }
            pipe = startedPipe
            Log.i(TAG, "webcam encoder=$codecName component=${startedCodec.name} size=${outWidth}x${outHeight} bitrate=$bitrate")
            ActiveCodec(startedCodec, surface, startedPipe, outWidth, outHeight, camera, codecName).also { it.startDrain() }
        }.onFailure { e ->
            runCatching { pipe?.release() }
            runCatching { codec?.stop() }
            runCatching { codec?.release() }
            runCatching { codecSurface?.release() }
            Log.w(TAG, "can't start $codecName encoder for ${outWidth}x${outHeight}", e)
        }.getOrNull()
    }

    /**
     * Configures [codec], without the low latency keys if it refuses them: some
     * encoders (Qualcomm's OMX ones on Huawei's EMUI) fail outright rather than
     * ignore a mode they don't have, and a little more latency beats no webcam.
     */
    private fun configureEncoder(codec: MediaCodec, format: MediaFormat) {
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: RuntimeException) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw e
            Log.i(TAG, "${codec.name} refused low latency; trying without it")
            codec.reset()
            format.removeKey(MediaFormat.KEY_LOW_LATENCY)
            format.removeKey(MediaFormat.KEY_LATENCY)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        }
    }

    private inner class ActiveCodec(
        val codec: MediaCodec,
        val codecSurface: Surface,
        val pipe: GlSurfacePipe,
        val width: Int,
        val height: Int,
        val camera: String,
        val codecName: String,
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
            val maxHeight = if (supports4k) 2160u else 1080u
            val cfgBytes = webcamConfigCodec(codecName, width.toUInt(), height.toUInt(), camera, fps.toUInt(), maxHeight)
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
                    val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    val payload = try {
                        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
                        val buffer = codec.getOutputBuffer(index) ?: continue
                        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                            parameters = ByteArray(info.size)
                            buffer.position(info.offset)
                            buffer.get(parameters)
                            continue
                        }
                        if (info.size == 0) continue
                        WebcamPacket.frame(buffer, info.offset, info.size, parameters, key, codecName)
                    } finally {
                        // The queued packet owns its bytes; release the codec slot before sending.
                        codec.releaseOutputBuffer(index, false)
                    }
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
                    if (codecName == "h265") {
                        hevcFailed = true
                        main.post { if (running && activeCodec === this) bindCamera() }
                    }
                }
            }
        }
    }

    companion object {
        private const val TAG = "CameraEncoder"
        private const val WAIT_US = 40_000L
        private const val I_FRAME_INTERVAL_SEC = 2

        /** Only offer hardware HEVC encoding at the actual requested size/rate. */
        private fun hevcEncoder(width: Int, height: Int, fps: Int): String? = runCatching {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull { info ->
                info.isEncoder &&
                    (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) info.isHardwareAccelerated
                    else !info.name.startsWith("OMX.google.") && !info.name.startsWith("c2.android.")) &&
                    info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_HEVC, ignoreCase = true) } &&
                    runCatching {
                        val caps = info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC)
                        caps.colorFormats.contains(MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface) &&
                            caps.profileLevels.any { it.profile == MediaCodecInfo.CodecProfileLevel.HEVCProfileMain } &&
                            caps.videoCapabilities?.areSizeAndRateSupported(width, height, fps.toDouble()) == true
                    }.getOrDefault(false)
            }?.name
        }.getOrNull()

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

        private val flashCache = java.util.concurrent.ConcurrentHashMap<String, Boolean>()
        private val maxZoomCache = java.util.concurrent.ConcurrentHashMap<String, Float>()
        private val cachedSupports4k: Boolean by lazy {
            runCatching {
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
        }

        /** Checks whether the requested camera (`front` or `back`) has a flash unit. */
        fun cameraHasFlash(context: Context, camera: String): Boolean =
            flashCache.getOrPut(camera) {
                runCatching {
                    val cm = context.getSystemService(CameraManager::class.java) ?: return@runCatching false
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
            }

        /** Queries the maximum digital zoom ratio supported by the requested camera (`front` or `back`). */
        fun cameraMaxZoom(context: Context, camera: String): Float =
            maxZoomCache.getOrPut(camera) {
                runCatching {
                    val cm = context.getSystemService(CameraManager::class.java) ?: return@runCatching 4f
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
                            return@runCatching maxZ.coerceIn(2f, 8f)
                        }
                    }
                    4f
                }.getOrDefault(4f)
            }

        /** Checks whether any hardware AVC encoder on this device supports 3840×2160. */
        fun supports4kEncoder(): Boolean = cachedSupports4k

        private val uhdCache = java.util.concurrent.ConcurrentHashMap<String, Boolean>()

        /**
         * Whether [camera] (`front` or `back`) can stream 4K: a camera facing
         * that way outputs 3840 × 2160, and the encoder takes it.
         */
        fun supports4k(context: Context, camera: String): Boolean =
            supports4kEncoder() && uhdCache.getOrPut(camera) {
                runCatching {
                    val cm = context.getSystemService(CameraManager::class.java) ?: return@runCatching false
                    val wantedFacing = if (camera == "front") {
                        CameraCharacteristics.LENS_FACING_FRONT
                    } else {
                        CameraCharacteristics.LENS_FACING_BACK
                    }
                    cm.cameraIdList.any { id ->
                        val chars = cm.getCameraCharacteristics(id)
                        val sizes = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                            ?.getOutputSizes(android.graphics.SurfaceTexture::class.java)
                            .orEmpty()
                        chars.get(CameraCharacteristics.LENS_FACING) == wantedFacing &&
                            sizes.any { it.width >= 3840 && it.height >= 2160 }
                    }
                }.getOrDefault(false)
            }

    }
}
