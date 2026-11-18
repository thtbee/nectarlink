// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.webcam

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.detached
import app.nectarlink.android.webcam.WebcamService

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
    val session by WebcamService.session.collectAsStateWithLifecycle()

    // Keep the phone's screen on while it streams to this PC.
    app.nectarlink.android.ui.KeepScreenOn(session.active && session.pcId == device.id)

    var hasCameraPermission by remember { mutableStateOf(WebcamService.hasPermission(context)) }
    var startAfterGrant by remember { mutableStateOf(false) }
    val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        hasCameraPermission = WebcamService.hasPermission(context)
        WebcamService.refreshCapabilities(context)
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

    LaunchedEffect(Unit) {
        WebcamService.refreshCapabilities(context)
    }

    LaunchedEffect(initialHeight, initialCamera) {
        if (initialHeight != null) WebcamService.setResolutionHeight(initialHeight)
        if (initialCamera != null) WebcamService.setCamera(initialCamera, context)
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

    val previewView = remember { WebcamPreviewView(context, WebcamService::attachUiPreview) }
    val previewShape = MaterialTheme.shapes.extraLarge
    val previewDensity = LocalDensity.current
    val previewDirection = LocalLayoutDirection.current
    LifecycleStartEffect(previewView) {
        previewView.setPreviewVisible(true)
        onStopOrDispose { previewView.setPreviewVisible(false) }
    }

    val isStreamingHere = session.active && session.pcId == device.id

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header: Back icon button, PC name, online dot
        item {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = onBack) {
                    Icon(
                        painter = painterResource(R.drawable.ic_back),
                        contentDescription = stringResource(R.string.action_back),
                    )
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
                        factory = { previewView.detached() },
                        update = { it.setPreviewShape(previewShape, previewDensity, previewDirection) },
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
                            text = stringResource(R.string.webcam_stream_spec, session.width, session.height, session.fps),
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
                            stringResource(
                                R.string.webcam_status_streaming_camera,
                                session.width,
                                session.height,
                                device.name,
                                camLabel,
                            )
                        } else {
                            stringResource(R.string.webcam_stream_camera_spec, session.width, session.height, camLabel)
                        },
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    )
                }
            }
        }

        // Controls Card: Start/Stop, Front/Back switch, Torch (when camera has flash), Resolution, Zoom
        item {
            Surface(
                shape = MaterialTheme.shapes.extraLarge,
                color = MaterialTheme.colorScheme.surfaceContainer,
                contentColor = MaterialTheme.colorScheme.onSurface,
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    // Start/Stop across the card, then the camera and torch side by side
                    // at the same height (the camera alone when there's no flash).
                    if (isStreamingHere) {
                        Button(
                            onClick = { WebcamService.stop(context) },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.error,
                                contentColor = MaterialTheme.colorScheme.onError,
                            ),
                            modifier = Modifier.fillMaxWidth().height(ControlHeight),
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
                            modifier = Modifier.fillMaxWidth().height(ControlHeight),
                        ) {
                            Text(stringResource(R.string.webcam_start))
                        }
                    }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalButton(
                            onClick = {
                                val next = if (session.camera == "front") "back" else "front"
                                WebcamService.setCamera(next, context)
                            },
                            modifier = Modifier.weight(1.3f).height(ControlHeight),
                        ) {
                            // Says what tapping does (#68), not which camera is on.
                            Text(
                                stringResource(
                                    if (session.camera == "front") R.string.webcam_switch_to_back else R.string.webcam_switch_to_front,
                                ),
                                textAlign = TextAlign.Center,
                                maxLines = 2,
                            )
                        }

                        if (session.hasFlash) {
                            OutlinedButton(
                                enabled = isStreamingHere,
                                onClick = { WebcamService.setTorch(!session.torchOn) },
                                modifier = Modifier.weight(1f).height(ControlHeight),
                            ) {
                                Text(
                                    stringResource(
                                        if (session.torchOn) R.string.webcam_turn_torch_off else R.string.webcam_turn_torch_on,
                                    ),
                                    textAlign = TextAlign.Center,
                                    maxLines = 2,
                                )
                            }
                        }
                    }

                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)

                    // Resolution picker: 720p, 1080p, and 4K (when supported)
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(
                            stringResource(R.string.webcam_resolution_label),
                            style = MaterialTheme.typography.labelLarge,
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            val res720p = stringResource(R.string.webcam_res_720p)
                            val res1080p = stringResource(R.string.webcam_res_1080p)
                            val res4k = stringResource(R.string.webcam_res_4k)
                            val resolutions = buildList {
                                add(720 to res720p)
                                add(1080 to res1080p)
                                if (session.supports4k) add(2160 to res4k)
                            }
                            for ((h, label) in resolutions) {
                                FilterChip(
                                    selected = session.resolution == h,
                                    onClick = { WebcamService.setResolutionHeight(h) },
                                    label = { Text(label) },
                                )
                            }
                        }
                    }

                    // Zoom controls: presets + smooth high-contrast slider
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        val zoomLabel = stringResource(
                            R.string.webcam_zoom_label,
                            session.zoomRatio,
                        )
                        Text(
                            zoomLabel,
                            style = MaterialTheme.typography.labelLarge,
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            FilterChip(
                                selected = kotlin.math.abs(session.zoomRatio - 1f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(1f) },
                                label = { Text(stringResource(R.string.webcam_zoom_preset, 1)) },
                            )
                            FilterChip(
                                selected = kotlin.math.abs(session.zoomRatio - 2f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(2f.coerceAtMost(session.maxZoomRatio)) },
                                label = { Text(stringResource(R.string.webcam_zoom_preset, 2)) },
                            )
                            if (session.maxZoomRatio >= 4f) {
                                FilterChip(
                                    selected = kotlin.math.abs(session.zoomRatio - 4f) < 0.08f,
                                    onClick = { WebcamService.setZoomRatio(4f.coerceAtMost(session.maxZoomRatio)) },
                                    label = { Text(stringResource(R.string.webcam_zoom_preset, 4)) },
                                )
                            }
                        }
                        val minZ = session.minZoomRatio
                        val maxZ = session.maxZoomRatio.coerceAtLeast(minZ + 0.1f)
                        Slider(
                            value = session.zoomRatio.coerceIn(minZ, maxZ),
                            onValueChange = { WebcamService.setZoomRatio(it) },
                            valueRange = minZ..maxZ,
                            colors = SliderDefaults.colors(
                                thumbColor = MaterialTheme.colorScheme.primary,
                                activeTrackColor = MaterialTheme.colorScheme.primary,
                                inactiveTrackColor = MaterialTheme.colorScheme.secondaryContainer,
                                disabledThumbColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                                disabledActiveTrackColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f),
                                disabledInactiveTrackColor = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.7f),
                            ),
                            modifier = Modifier.semantics { contentDescription = zoomLabel },
                        )
                    }
                }
            }
        }
    }
}

/** One height for the webcam's buttons, so they line up. */
private val ControlHeight = 52.dp
