// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.webcam

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Device
import app.nectarlink.android.webcam.WebcamService
import java.util.Locale

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun WebcamScreen(
    device: Device,
    autoStart: Boolean = false,
    initialHeight: Int? = null,
    initialCamera: String? = null,
    onAutoStartConsumed: () -> Unit = {},
    onAccessChanged: () -> Unit = {},
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val view = LocalView.current
    val session by WebcamService.session.collectAsStateWithLifecycle()

    // Keep the phone's screen on while the webcam viewfinder/controls are open.
    DisposableEffect(view) {
        val prev = view.keepScreenOn
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = prev }
    }

    var hasCameraPermission by remember { mutableStateOf(WebcamService.hasPermission(context)) }
    var startAfterGrant by remember { mutableStateOf(false) }
    val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        hasCameraPermission = WebcamService.hasPermission(context)
        onAccessChanged()
        if (granted && startAfterGrant) {
            startAfterGrant = false
            WebcamService.start(
                context = context,
                pcId = device.id,
                height = initialHeight ?: session.height,
                camera = initialCamera ?: session.camera,
            )
        } else {
            startAfterGrant = false
        }
    }

    LaunchedEffect(initialHeight, initialCamera) {
        if (initialHeight != null) WebcamService.setResolutionHeight(initialHeight)
        if (initialCamera != null) WebcamService.setCamera(initialCamera)
    }

    LaunchedEffect(autoStart, hasCameraPermission, device.online) {
        if (autoStart) {
            onAutoStartConsumed()
            if (hasCameraPermission && device.online) {
                WebcamService.start(
                    context = context,
                    pcId = device.id,
                    height = initialHeight ?: session.height,
                    camera = initialCamera ?: session.camera,
                )
            } else if (!hasCameraPermission) {
                startAfterGrant = true
                askCamera.launch(Manifest.permission.CAMERA)
            }
        }
    }

    val previewView = remember {
        PreviewView(context).apply {
            scaleType = PreviewView.ScaleType.FIT_CENTER
            implementationMode = PreviewView.ImplementationMode.COMPATIBLE
        }
    }

    DisposableEffect(previewView) {
        WebcamService.attachUiPreview(previewView.surfaceProvider)
        onDispose {
            WebcamService.attachUiPreview(null)
        }
    }

    val isStreamingHere = session.active && session.pcId == device.id

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header: Back, PC name, online dot
        item {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(
                    onClick = onBack,
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Text("← " + stringResource(R.string.action_back), style = MaterialTheme.typography.labelLarge)
                }
                Spacer(Modifier.width(4.dp))
                Text(
                    device.name,
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.weight(1f),
                )
                Surface(
                    shape = CircleShape,
                    color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                    modifier = Modifier.size(10.dp),
                ) {}
            }
        }

        // Camera permission prompt card when CAMERA permission is not granted yet
        if (!hasCameraPermission) {
            item {
                Surface(
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text(
                            stringResource(R.string.webcam_permission_title),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.webcam_permission_text, device.name),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(12.dp))
                        Button(
                            onClick = {
                                startAfterGrant = device.online
                                askCamera.launch(Manifest.permission.CAMERA)
                            },
                        ) {
                            Text(stringResource(R.string.webcam_allow_camera))
                        }
                    }
                }
            }
        }

        // Offline notice
        if (!device.online) {
            item {
                Surface(
                    shape = MaterialTheme.shapes.large,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                ) {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text(
                            stringResource(R.string.not_connected),
                            style = MaterialTheme.typography.titleSmall,
                        )
                        Spacer(Modifier.height(2.dp))
                        Text(
                            stringResource(R.string.webcam_offline_hint, device.name),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        // Live 16:9 camera preview box
        item {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(16f / 9f)
                    .clip(MaterialTheme.shapes.extraLarge)
                    .background(Color(0xFF101216)),
                contentAlignment = Alignment.Center,
            ) {
                if (isStreamingHere && hasCameraPermission) {
                    AndroidView(
                        factory = { previewView },
                        modifier = Modifier.fillMaxSize(),
                    )
                } else {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.padding(24.dp),
                    ) {
                        Text(
                            text = if (isStreamingHere && session.connecting) {
                                stringResource(R.string.webcam_status_connecting, device.name)
                            } else {
                                stringResource(R.string.webcam_status_ready, device.name)
                            },
                            style = MaterialTheme.typography.titleMedium,
                            color = Color.White,
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            text = "${session.width} × ${session.height} · ${session.fps} fps",
                            style = MaterialTheme.typography.bodySmall,
                            color = Color.White.copy(alpha = 0.72f),
                        )
                    }
                }

                // Overlay badge in top-left corner
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    color = Color.Black.copy(alpha = 0.58f),
                    contentColor = Color.White,
                    modifier = Modifier
                        .align(Alignment.TopStart)
                        .padding(12.dp),
                ) {
                    val camLabel = stringResource(
                        if (session.camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
                    )
                    Text(
                        text = if (isStreamingHere && !session.connecting) {
                            stringResource(R.string.webcam_status_streaming, session.width, session.height, device.name) +
                                " · $camLabel"
                        } else {
                            "${session.width} × ${session.height} · $camLabel"
                        },
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    )
                }
            }
        }

        // Controls Card: Start/Stop, Front/Back switch, Torch, Resolution, Zoom
        item {
            Surface(
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    // Primary Start/Stop + Flip camera + Torch
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        if (isStreamingHere) {
                            Button(
                                onClick = { WebcamService.stop(context) },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.colorScheme.error,
                                    contentColor = MaterialTheme.colorScheme.onError,
                                ),
                            ) {
                                Text(stringResource(R.string.webcam_stop))
                            }
                        } else {
                            Button(
                                enabled = device.online,
                                onClick = {
                                    if (!hasCameraPermission) {
                                        startAfterGrant = true
                                        askCamera.launch(Manifest.permission.CAMERA)
                                    } else {
                                        WebcamService.start(
                                            context = context,
                                            pcId = device.id,
                                            height = session.height,
                                            camera = session.camera,
                                        )
                                    }
                                },
                            ) {
                                Text(stringResource(R.string.webcam_start))
                            }
                        }

                        FilledTonalButton(
                            onClick = {
                                val next = if (session.camera == "front") "back" else "front"
                                WebcamService.setCamera(next)
                            },
                        ) {
                            val currentLabel = stringResource(
                                if (session.camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
                            )
                            Text(stringResource(R.string.webcam_switch_camera) + " ($currentLabel)")
                        }

                        if (session.camera == "back" && (!isStreamingHere || session.hasFlash)) {
                            OutlinedButton(
                                enabled = isStreamingHere && session.hasFlash,
                                onClick = { WebcamService.setTorch(!session.torchOn) },
                            ) {
                                Text(
                                    stringResource(
                                        if (session.torchOn) R.string.webcam_torch_on else R.string.webcam_torch_off,
                                    ),
                                )
                            }
                        }
                    }

                    // Resolution picker: 720p, 1080p, and 4K (when supported)
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(
                            stringResource(R.string.webcam_resolution_label),
                            style = MaterialTheme.typography.labelLarge,
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            val resolutions = buildList {
                                add(720 to "720p")
                                add(1080 to "1080p")
                                if (session.supports4k) add(2160 to "4K")
                            }
                            for ((h, label) in resolutions) {
                                FilterChip(
                                    selected = session.height == h,
                                    onClick = { WebcamService.setResolutionHeight(h) },
                                    label = { Text(label) },
                                )
                            }
                        }
                    }

                    // Zoom controls: 1x / 2x presets + smooth slider
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                stringResource(
                                    R.string.webcam_zoom_label,
                                    String.format(Locale.US, "%.1f", session.zoomRatio).toFloat(),
                                ),
                                style = MaterialTheme.typography.labelLarge,
                                modifier = Modifier.weight(1f),
                            )
                            FilterChip(
                                selected = kotlin.math.abs(session.zoomRatio - 1f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(1f) },
                                label = { Text("1×") },
                            )
                            Spacer(Modifier.width(8.dp))
                            FilterChip(
                                selected = kotlin.math.abs(session.zoomRatio - 2f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(2f.coerceAtMost(session.maxZoomRatio)) },
                                label = { Text("2×") },
                            )
                        }
                        val minZ = session.minZoomRatio
                        val maxZ = session.maxZoomRatio.coerceAtLeast(minZ + 0.1f)
                        Slider(
                            value = session.zoomRatio.coerceIn(minZ, maxZ),
                            onValueChange = { WebcamService.setZoomRatio(it) },
                            valueRange = minZ..maxZ,
                            enabled = isStreamingHere,
                        )
                    }
                }
            }
        }
    }
}
