// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.camera

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.core.graphics.scale
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.NectarlinkTheme
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Full-screen Continuity Camera capture activity launched when a paired PC
 * requests a photo (`"photo"`) or document scan (`"scan"`).
 */
class ContinuityCameraActivity : ComponentActivity() {
    private var pcId: String = ""
    private var requestId: String = ""
    private var initialMode: String = "photo"
    private var pcName: String = ""
    private var completedOrCancelled = false

    private val cameraPermissionGranted = mutableStateOf(false)

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        cameraPermissionGranted.value = granted
        if (!granted) {
            cancelAndFinish(getString(R.string.continuity_camera_permission_denied))
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (!readIntent(intent)) {
            finish()
            return
        }
        ContinuityCameraRequests.dismissNotification(this, pcId)

        val hasPerm = ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        cameraPermissionGranted.value = hasPerm
        if (!hasPerm) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }

        val app = application as NectarlinkApplication
        val preferences = Preferences(this)

        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            val activeRequest by ContinuityCameraRequests.active.collectAsStateWithLifecycle()
            val hasCamera by cameraPermissionGranted

            // Close automatically if the PC cancelled this capture request.
            LaunchedEffect(activeRequest) {
                if (activeRequest == null && !completedOrCancelled) {
                    completedOrCancelled = true
                    finish()
                }
            }

            NectarlinkTheme(appearance) {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background,
                ) {
                    if (hasCamera) {
                        ContinuityCameraScreen(
                            pcName = pcName,
                            initialMode = initialMode,
                            onSendResult = { mode, fileName, width, height, jpegBytes ->
                                completedOrCancelled = true
                                ContinuityCameraRequests.clearActive(pcId, requestId)
                                ContinuityCameraRequests.dismissNotification(this@ContinuityCameraActivity, pcId)
                                app.core.sendCameraCaptureResult(
                                    pcId = pcId,
                                    requestId = requestId,
                                    mode = mode,
                                    fileName = fileName,
                                    mime = "image/jpeg",
                                    width = width,
                                    height = height,
                                    data = jpegBytes,
                                )
                                finish()
                            },
                            onCancel = { reason ->
                                cancelAndFinish(reason)
                            },
                        )
                    } else {
                        Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .padding(24.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                Text(
                                    text = stringResource(R.string.webcam_permission_title),
                                    style = MaterialTheme.typography.titleMedium,
                                )
                                Spacer(Modifier.height(12.dp))
                                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                    OutlinedButton(onClick = { cancelAndFinish(getString(R.string.continuity_camera_cancelled)) }) {
                                        Text(stringResource(R.string.action_cancel))
                                    }
                                    Button(onClick = { permissionLauncher.launch(Manifest.permission.CAMERA) }) {
                                        Text(stringResource(R.string.action_allow_camera))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (readIntent(intent)) {
            completedOrCancelled = false
            ContinuityCameraRequests.dismissNotification(this, pcId)
        }
    }

    override fun onDestroy() {
        if (isFinishing && !completedOrCancelled && pcId.isNotEmpty() && requestId.isNotEmpty()) {
            completedOrCancelled = true
            ContinuityCameraRequests.clearActive(pcId, requestId)
            (application as? NectarlinkApplication)?.core?.cancelCameraCapture(
                pcId = pcId,
                requestId = requestId,
                reason = getString(R.string.continuity_camera_cancelled),
            )
        }
        super.onDestroy()
    }

    private fun readIntent(intent: Intent?): Boolean {
        val i = intent ?: return false
        val id = i.getStringExtra(ContinuityCameraRequests.EXTRA_PC_ID).orEmpty()
        val req = i.getStringExtra(ContinuityCameraRequests.EXTRA_REQUEST_ID).orEmpty()
        if (id.isEmpty() || req.isEmpty()) return false
        pcId = id
        requestId = req
        initialMode = if (i.getStringExtra(ContinuityCameraRequests.EXTRA_MODE) == "scan") "scan" else "photo"
        pcName = i.getStringExtra(ContinuityCameraRequests.EXTRA_PC_NAME).orEmpty()
            .ifEmpty { getString(R.string.your_pc) }
        return true
    }

    private fun cancelAndFinish(reason: String) {
        if (!completedOrCancelled && pcId.isNotEmpty() && requestId.isNotEmpty()) {
            completedOrCancelled = true
            ContinuityCameraRequests.clearActive(pcId, requestId)
            ContinuityCameraRequests.dismissNotification(this, pcId)
            (application as? NectarlinkApplication)?.core?.cancelCameraCapture(
                pcId = pcId,
                requestId = requestId,
                reason = reason,
            )
        }
        finish()
    }
}

@Composable
private fun ContinuityCameraScreen(
    pcName: String,
    initialMode: String,
    onSendResult: (mode: String, fileName: String, width: Int, height: Int, jpegBytes: ByteArray) -> Unit,
    onCancel: (reason: String) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()

    var mode by remember(initialMode) { mutableStateOf(initialMode) }
    var useFrontCamera by remember { mutableStateOf(false) }
    var flashOn by remember { mutableStateOf(false) }
    var liveQuad by remember { mutableStateOf(DocumentScannerProcessor.DEFAULT_QUAD) }

    var capturedBitmap by remember { mutableStateOf<Bitmap?>(null) }
    var editQuad by remember { mutableStateOf(DocumentScannerProcessor.DEFAULT_QUAD) }
    var enhanceScan by remember { mutableStateOf(true) }
    var showRectifiedPreview by remember { mutableStateOf(false) }
    var rectifiedBitmap by remember { mutableStateOf<Bitmap?>(null) }
    var busy by remember { mutableStateOf(false) }

    val previewView = remember {
        PreviewView(context).apply {
            scaleType = PreviewView.ScaleType.FIT_CENTER
        }
    }
    val cameraExecutor = remember { Executors.newSingleThreadExecutor() }
    var imageCapture by remember { mutableStateOf<ImageCapture?>(null) }
    var boundCamera by remember { mutableStateOf<Camera?>(null) }
    val cancelledText = stringResource(R.string.continuity_camera_cancelled)

    BackHandler {
        if (capturedBitmap != null && !busy) {
            capturedBitmap = null
            rectifiedBitmap = null
            showRectifiedPreview = false
        } else if (!busy) {
            onCancel(cancelledText)
        }
    }

    // Recompute rectified preview whenever the user toggles preview in scan mode
    LaunchedEffect(capturedBitmap, editQuad, enhanceScan, showRectifiedPreview) {
        val src = capturedBitmap
        if (src != null && mode == "scan" && showRectifiedPreview) {
            rectifiedBitmap = withContext(Dispatchers.Default) {
                DocumentScannerProcessor.rectifyDocument(src, editQuad, enhanceScan)
            }
        } else {
            rectifiedBitmap = null
        }
    }

    DisposableEffect(lifecycleOwner, mode, useFrontCamera, capturedBitmap == null) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        if (capturedBitmap == null) {
            providerFuture.addListener({
                runCatching {
                    val provider = providerFuture.get()
                    val preview = Preview.Builder().build().also {
                        it.surfaceProvider = previewView.surfaceProvider
                    }
                    val capture = ImageCapture.Builder()
                        .setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
                        .setFlashMode(
                            if (flashOn) ImageCapture.FLASH_MODE_ON else ImageCapture.FLASH_MODE_OFF,
                        )
                        .build()
                    imageCapture = capture

                    val selector = if (useFrontCamera && mode == "photo") {
                        CameraSelector.DEFAULT_FRONT_CAMERA
                    } else {
                        CameraSelector.DEFAULT_BACK_CAMERA
                    }

                    provider.unbindAll()
                    val cam = if (mode == "scan") {
                        val analysis = ImageAnalysis.Builder()
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                        var lastDetectMs = 0L
                        analysis.setAnalyzer(cameraExecutor) { proxy ->
                            proxy.use { frame ->
                                val now = System.currentTimeMillis()
                                if (now - lastDetectMs >= 120L) {
                                    lastDetectMs = now
                                    val lumaData = extractUprightLuma(frame)
                                    if (lumaData != null) {
                                        val (luma, w, h) = lumaData
                                        val quad = DocumentScannerProcessor.detectDocumentQuad(luma, w, h)
                                        ContextCompat.getMainExecutor(context).execute {
                                            liveQuad = quad
                                        }
                                    }
                                }
                            }
                        }
                        provider.bindToLifecycle(lifecycleOwner, selector, preview, capture, analysis)
                    } else {
                        provider.bindToLifecycle(lifecycleOwner, selector, preview, capture)
                    }
                    boundCamera = cam
                    cam.cameraControl.enableTorch(flashOn && (!useFrontCamera || mode == "scan"))
                }
            }, ContextCompat.getMainExecutor(context))
        }

        onDispose {
            if (capturedBitmap != null && providerFuture.isDone) {
                runCatching { providerFuture.get().unbindAll() }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            cameraExecutor.shutdown()
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .statusBarsPadding()
            .navigationBarsPadding(),
    ) {
        // Top header bar
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(
                        if (mode == "scan") R.string.continuity_camera_scan_title else R.string.continuity_camera_photo_title,
                    ),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = stringResource(R.string.continuity_camera_for_pc, pcName),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            if (capturedBitmap == null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(
                        selected = mode == "photo",
                        onClick = { mode = "photo" },
                        label = { Text(stringResource(R.string.continuity_camera_mode_photo)) },
                    )
                    FilterChip(
                        selected = mode == "scan",
                        onClick = {
                            mode = "scan"
                            useFrontCamera = false
                        },
                        label = { Text(stringResource(R.string.continuity_camera_mode_scan)) },
                    )
                }
            }

            Spacer(Modifier.width(8.dp))
            TextButton(
                onClick = { onCancel(cancelledText) },
                enabled = !busy,
            ) {
                Text(stringResource(R.string.action_cancel))
            }
        }

        // Main camera or review area
        Box(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .background(Color.Black),
            contentAlignment = Alignment.Center,
        ) {
            val currentCaptured = capturedBitmap
            if (currentCaptured == null) {
                AndroidView(
                    factory = { previewView },
                    modifier = Modifier.fillMaxSize(),
                )
                if (mode == "scan") {
                    val primaryColor = MaterialTheme.colorScheme.primary
                    Canvas(modifier = Modifier.fillMaxSize()) {
                        val q = liveQuad.ordered()
                        val p1 = Offset(q.topLeft.x * size.width, q.topLeft.y * size.height)
                        val p2 = Offset(q.topRight.x * size.width, q.topRight.y * size.height)
                        val p3 = Offset(q.bottomRight.x * size.width, q.bottomRight.y * size.height)
                        val p4 = Offset(q.bottomLeft.x * size.width, q.bottomLeft.y * size.height)
                        val path = Path().apply {
                            moveTo(p1.x, p1.y)
                            lineTo(p2.x, p2.y)
                            lineTo(p3.x, p3.y)
                            lineTo(p4.x, p4.y)
                            close()
                        }
                        drawPath(path, color = primaryColor.copy(alpha = 0.18f))
                        drawPath(path, color = primaryColor, style = Stroke(width = 3.dp.toPx()))
                    }
                }
            } else {
                val displayBitmap = if (mode == "scan" && showRectifiedPreview && rectifiedBitmap != null) {
                    rectifiedBitmap!!
                } else {
                    currentCaptured
                }
                var boxSize by remember { mutableStateOf(IntSize.Zero) }
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(16.dp)
                        .onSizeChanged { boxSize = it },
                    contentAlignment = Alignment.Center,
                ) {
                    Image(
                        bitmap = displayBitmap.asImageBitmap(),
                        contentDescription = stringResource(R.string.continuity_camera_preview_desc),
                        contentScale = ContentScale.Fit,
                        modifier = Modifier.fillMaxSize(),
                    )
                    if (mode == "scan" && !showRectifiedPreview && boxSize.width > 0 && boxSize.height > 0) {
                        QuadCornerEditorOverlay(
                            bitmapWidth = currentCaptured.width,
                            bitmapHeight = currentCaptured.height,
                            containerSize = boxSize,
                            quad = editQuad,
                            onQuadChanged = { editQuad = it },
                        )
                    }
                }
            }

            if (busy) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.45f)),
                    contentAlignment = Alignment.Center,
                ) {
                    if (LocalReducedMotion.current) {
                        CircularProgressIndicator(progress = { 0.75f })
                    } else {
                        CircularProgressIndicator()
                    }
                }
            }
        }

        // Bottom controls
        val currentCaptured = capturedBitmap
        if (currentCaptured == null) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp, vertical = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                OutlinedButton(
                    onClick = {
                        flashOn = !flashOn
                        boundCamera?.cameraControl?.enableTorch(flashOn && (!useFrontCamera || mode == "scan"))
                        imageCapture?.flashMode = if (flashOn) {
                            ImageCapture.FLASH_MODE_ON
                        } else {
                            ImageCapture.FLASH_MODE_OFF
                        }
                    },
                    enabled = !busy,
                ) {
                    Text(if (flashOn) stringResource(R.string.webcam_torch_on) else stringResource(R.string.webcam_torch_off))
                }

                // Shutter button
                val shutterDesc = stringResource(
                    if (mode == "scan") R.string.continuity_camera_shutter_scan else R.string.continuity_camera_shutter_photo,
                )
                Box(
                    modifier = Modifier
                        .size(72.dp)
                        .clip(CircleShape)
                        .border(4.dp, MaterialTheme.colorScheme.primary, CircleShape)
                        .padding(6.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.primary)
                        .semantics {
                            role = Role.Button
                            contentDescription = shutterDesc
                        }
                        .clickable(enabled = !busy) {
                            val cap = imageCapture
                            if (cap == null) {
                                val fallback = previewView.bitmap ?: return@clickable
                                onPhotoCaptured(
                                    bmp = fallback,
                                    mode = mode,
                                    fallbackQuad = liveQuad,
                                    onReady = { bmp, q ->
                                        capturedBitmap = bmp
                                        editQuad = q
                                    },
                                )
                                return@clickable
                            }
                            busy = true
                            cap.takePicture(
                                cameraExecutor,
                                object : ImageCapture.OnImageCapturedCallback() {
                                    override fun onCaptureSuccess(image: ImageProxy) {
                                        val decoded = imageProxyToBitmap(image, mirrorHorizontally = useFrontCamera && mode == "photo")
                                        image.close()
                                        val detectedQuad = if (decoded != null && mode == "scan") {
                                            DocumentScannerProcessor.detectDocumentQuad(decoded)
                                        } else {
                                            liveQuad
                                        }
                                        ContextCompat.getMainExecutor(context).execute {
                                            busy = false
                                            val finalBmp = decoded ?: previewView.bitmap
                                            if (finalBmp != null) {
                                                capturedBitmap = finalBmp
                                                editQuad = detectedQuad
                                                showRectifiedPreview = false
                                            }
                                        }
                                    }

                                    override fun onError(exception: ImageCaptureException) {
                                        ContextCompat.getMainExecutor(context).execute {
                                            busy = false
                                            val fallback = previewView.bitmap
                                            if (fallback != null) {
                                                onPhotoCaptured(
                                                    bmp = fallback,
                                                    mode = mode,
                                                    fallbackQuad = liveQuad,
                                                    onReady = { bmp, q ->
                                                        capturedBitmap = bmp
                                                        editQuad = q
                                                    },
                                                )
                                            }
                                        }
                                    }
                                },
                            )
                        },
                )

                if (mode == "photo") {
                    OutlinedButton(
                        onClick = { useFrontCamera = !useFrontCamera },
                        enabled = !busy,
                    ) {
                        Text(stringResource(R.string.webcam_switch_camera))
                    }
                } else {
                    Spacer(Modifier.width(88.dp))
                }
            }
        } else {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (mode == "scan") {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        FilterChip(
                            selected = enhanceScan,
                            onClick = { enhanceScan = !enhanceScan },
                            label = { Text(stringResource(R.string.continuity_camera_enhance)) },
                        )
                        FilterChip(
                            selected = showRectifiedPreview,
                            onClick = { showRectifiedPreview = !showRectifiedPreview },
                            label = {
                                Text(
                                    stringResource(
                                        if (showRectifiedPreview) R.string.continuity_camera_adjust_corners
                                        else R.string.continuity_camera_preview_scan,
                                    ),
                                )
                            },
                        )
                    }
                }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OutlinedButton(
                        onClick = {
                            capturedBitmap = null
                            rectifiedBitmap = null
                            showRectifiedPreview = false
                        },
                        enabled = !busy,
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.continuity_camera_retake))
                    }
                    Button(
                        onClick = {
                            if (busy) return@Button
                            busy = true
                            scope.launch {
                                val result = withContext(Dispatchers.Default) {
                                    val finalBmp = if (mode == "scan") {
                                        DocumentScannerProcessor.rectifyDocument(currentCaptured, editQuad, enhanceScan)
                                    } else {
                                        scaleDownIfNeeded(currentCaptured, MAX_OUTPUT_PX)
                                    }
                                    val jpeg = encodeJpegUnderLimit(finalBmp)
                                    val prefix = if (mode == "scan") "scan" else "photo"
                                    val name = "$prefix-${System.currentTimeMillis()}.jpg"
                                    Triple(name, finalBmp.width to finalBmp.height, jpeg)
                                }
                                val (fileName, dims, jpegBytes) = result
                                onSendResult(mode, fileName, dims.first, dims.second, jpegBytes)
                            }
                        },
                        enabled = !busy,
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(
                            stringResource(
                                if (mode == "scan") R.string.continuity_camera_send_scan
                                else R.string.continuity_camera_use_photo,
                            ),
                        )
                    }
                }
            }
        }
    }
}

private const val MAX_OUTPUT_PX = 2400
private const val MAX_JPEG_BYTES = 16 * 1024 * 1024

private fun onPhotoCaptured(
    bmp: Bitmap,
    mode: String,
    fallbackQuad: QuadCorners,
    onReady: (Bitmap, QuadCorners) -> Unit,
) {
    val q = if (mode == "scan") {
        runCatching { DocumentScannerProcessor.detectDocumentQuad(bmp) }.getOrDefault(fallbackQuad)
    } else {
        fallbackQuad
    }
    onReady(bmp, q)
}

@Composable
private fun QuadCornerEditorOverlay(
    bitmapWidth: Int,
    bitmapHeight: Int,
    containerSize: IntSize,
    quad: QuadCorners,
    onQuadChanged: (QuadCorners) -> Unit,
) {
    val primaryColor = MaterialTheme.colorScheme.primary
    val surfaceColor = MaterialTheme.colorScheme.surface
    val cornersDesc = stringResource(R.string.continuity_camera_corners_desc)

    val containerW = containerSize.width.toFloat()
    val containerH = containerSize.height.toFloat()
    val bmpAspect = bitmapWidth.toFloat() / bitmapHeight.coerceAtLeast(1).toFloat()
    val containerAspect = containerW / containerH.coerceAtLeast(1f)

    val (drawW, drawH) = if (bmpAspect > containerAspect) {
        containerW to (containerW / bmpAspect)
    } else {
        (containerH * bmpAspect) to containerH
    }
    val offsetX = (containerW - drawW) * 0.5f
    val offsetY = (containerH - drawH) * 0.5f

    fun toScreen(pt: PointF2D): Offset = Offset(
        x = offsetX + pt.x * drawW,
        y = offsetY + pt.y * drawH,
    )

    fun toNormalized(screen: Offset): PointF2D = PointF2D(
        x = ((screen.x - offsetX) / drawW.coerceAtLeast(1f)).coerceIn(0.01f, 0.99f),
        y = ((screen.y - offsetY) / drawH.coerceAtLeast(1f)).coerceIn(0.01f, 0.99f),
    )

    var activeCorner by remember { mutableStateOf(-1) }
    var currentQuad by remember(quad) { mutableStateOf(quad) }

    Canvas(
        modifier = Modifier
            .fillMaxSize()
            .semantics { contentDescription = cornersDesc }
            .pointerInput(drawW, drawH, offsetX, offsetY) {
                detectDragGestures(
                    onDragStart = { startOffset ->
                        val pts = listOf(
                            toScreen(currentQuad.topLeft),
                            toScreen(currentQuad.topRight),
                            toScreen(currentQuad.bottomRight),
                            toScreen(currentQuad.bottomLeft),
                        )
                        var bestIdx = -1
                        var bestDist = Float.MAX_VALUE
                        for (i in pts.indices) {
                            val d = hypot(pts[i].x - startOffset.x, pts[i].y - startOffset.y)
                            if (d < bestDist) {
                                bestDist = d
                                bestIdx = i
                            }
                        }
                        activeCorner = bestIdx
                    },
                    onDragEnd = {
                        activeCorner = -1
                        onQuadChanged(currentQuad.ordered())
                    },
                    onDragCancel = {
                        activeCorner = -1
                    },
                    onDrag = { change, _ ->
                        change.consume()
                        val norm = toNormalized(change.position)
                        currentQuad = when (activeCorner) {
                            0 -> currentQuad.copy(topLeft = norm)
                            1 -> currentQuad.copy(topRight = norm)
                            2 -> currentQuad.copy(bottomRight = norm)
                            3 -> currentQuad.copy(bottomLeft = norm)
                            else -> currentQuad
                        }
                        onQuadChanged(currentQuad)
                    },
                )
            },
    ) {
        val p1 = toScreen(currentQuad.topLeft)
        val p2 = toScreen(currentQuad.topRight)
        val p3 = toScreen(currentQuad.bottomRight)
        val p4 = toScreen(currentQuad.bottomLeft)

        val path = Path().apply {
            moveTo(p1.x, p1.y)
            lineTo(p2.x, p2.y)
            lineTo(p3.x, p3.y)
            lineTo(p4.x, p4.y)
            close()
        }
        drawPath(path, color = primaryColor.copy(alpha = 0.20f))
        drawPath(path, color = primaryColor, style = Stroke(width = 3.dp.toPx()))

        for (pt in listOf(p1, p2, p3, p4)) {
            drawCircle(color = primaryColor, radius = 12.dp.toPx(), center = pt)
            drawCircle(color = surfaceColor, radius = 6.dp.toPx(), center = pt)
        }
    }
}

private fun extractUprightLuma(image: ImageProxy): Triple<ByteArray, Int, Int>? = runCatching {
    val plane = image.planes.firstOrNull() ?: return null
    val buf = plane.buffer
    val rowStride = plane.rowStride
    val pixelStride = plane.pixelStride
    val srcW = image.width
    val srcH = image.height
    val rot = ((image.imageInfo.rotationDegrees % 360) + 360) % 360
    val uprightW = if (rot == 90 || rot == 270) srcH else srcW
    val uprightH = if (rot == 90 || rot == 270) srcW else srcH
    val scale = min(1f, 180f / max(uprightW, uprightH).toFloat())
    val dstW = (uprightW * scale).toInt().coerceAtLeast(32)
    val dstH = (uprightH * scale).toInt().coerceAtLeast(32)
    val out = ByteArray(dstW * dstH)
    val limit = buf.limit()

    for (dy in 0 until dstH) {
        val uy = dy * uprightH / dstH
        for (dx in 0 until dstW) {
            val ux = dx * uprightW / dstW
            val sx: Int
            val sy: Int
            when (rot) {
                90 -> {
                    sx = uy
                    sy = srcH - 1 - ux
                }
                180 -> {
                    sx = srcW - 1 - ux
                    sy = srcH - 1 - uy
                }
                270 -> {
                    sx = srcW - 1 - uy
                    sy = ux
                }
                else -> {
                    sx = ux
                    sy = uy
                }
            }
            val idx = sy * rowStride + sx * pixelStride
            out[dy * dstW + dx] = if (idx in 0 until limit) buf.get(idx) else 0
        }
    }
    Triple(out, dstW, dstH)
}.getOrNull()

private fun imageProxyToBitmap(image: ImageProxy, mirrorHorizontally: Boolean): Bitmap? = runCatching {
    val rawBitmap = if (image.format == ImageFormat.JPEG || image.planes.size == 1) {
        val buf = image.planes[0].buffer
        val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
    } else {
        yuv420ToBitmap(image)
    } ?: return null

    val rot = ((image.imageInfo.rotationDegrees % 360) + 360) % 360
    val rotated = if (rot != 0 || mirrorHorizontally) {
        val matrix = Matrix().apply {
            if (rot != 0) postRotate(rot.toFloat())
            if (mirrorHorizontally) postScale(-1f, 1f)
        }
        Bitmap.createBitmap(rawBitmap, 0, 0, rawBitmap.width, rawBitmap.height, matrix, true)
    } else {
        rawBitmap
    }
    scaleDownIfNeeded(rotated, MAX_OUTPUT_PX)
}.getOrNull()

private fun yuv420ToBitmap(image: ImageProxy): Bitmap? {
    val w = image.width
    val h = image.height
    val yPlane = image.planes[0]
    val uPlane = image.planes[1]
    val vPlane = image.planes[2]
    val yBuf = yPlane.buffer
    val uBuf = uPlane.buffer
    val vBuf = vPlane.buffer
    val yRowStride = yPlane.rowStride
    val yPixStride = yPlane.pixelStride
    val uvRowStride = uPlane.rowStride
    val uvPixStride = uPlane.pixelStride
    val pixels = IntArray(w * h)

    for (y in 0 until h) {
        val yRow = y * yRowStride
        val uvRow = (y ushr 1) * uvRowStride
        for (x in 0 until w) {
            val yVal = (yBuf.get(yRow + x * yPixStride).toInt() and 0xFF)
            val uvIdx = uvRow + (x ushr 1) * uvPixStride
            val uVal = (uBuf.get(uvIdx).toInt() and 0xFF) - 128
            val vVal = (vBuf.get(uvIdx).toInt() and 0xFF) - 128
            val r = (yVal + (1.370705f * vVal).toInt()).coerceIn(0, 255)
            val g = (yVal - (0.698001f * vVal).toInt() - (0.337633f * uVal).toInt()).coerceIn(0, 255)
            val b = (yVal + (1.732446f * uVal).toInt()).coerceIn(0, 255)
            pixels[y * w + x] = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
        }
    }
    return Bitmap.createBitmap(pixels, w, h, Bitmap.Config.ARGB_8888)
}

private fun scaleDownIfNeeded(bitmap: Bitmap, maxSide: Int): Bitmap {
    val longest = max(bitmap.width, bitmap.height)
    if (longest <= maxSide) return bitmap
    val scale = maxSide.toFloat() / longest.toFloat()
    val targetW = (bitmap.width * scale).toInt().coerceAtLeast(1)
    val targetH = (bitmap.height * scale).toInt().coerceAtLeast(1)
    return bitmap.scale(targetW, targetH)
}

private fun encodeJpegUnderLimit(bitmap: Bitmap): ByteArray {
    for (q in intArrayOf(90, 82, 72)) {
        val bytes = ByteArrayOutputStream().use { out ->
            bitmap.compress(Bitmap.CompressFormat.JPEG, q, out)
            out.toByteArray()
        }
        if (bytes.size <= MAX_JPEG_BYTES) return bytes
    }
    val smaller = scaleDownIfNeeded(bitmap, 1600)
    return ByteArrayOutputStream().use { out ->
        smaller.compress(Bitmap.CompressFormat.JPEG, 80, out)
        out.toByteArray()
    }
}
