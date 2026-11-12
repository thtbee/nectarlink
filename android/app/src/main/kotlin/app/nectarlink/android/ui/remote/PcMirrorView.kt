// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.app.Activity
import android.content.Context
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.database.ContentObserver
import android.graphics.SurfaceTexture
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.OrientationEventListener
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateCentroidSize
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.isSpecified
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.currentStateAsState
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.TouchPhase
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.roundToInt
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * Fitted video rectangle inside a container of `(containerWidth, containerHeight)`,
 * accounting for letterbox (top/bottom bars) or pillarbox (left/right bars) offsets.
 */
data class VideoViewportRect(
    val left: Float,
    val top: Float,
    val width: Float,
    val height: Float,
) {
    val right: Float get() = left + width
    val bottom: Float get() = top + height

    fun contains(x: Float, y: Float): Boolean =
        x >= left && x <= right && y >= top && y <= bottom
}

/**
 * How far the PC's screen is zoomed in on the phone: [scale] (1 is the whole
 * screen) and the picture's shift, as a fraction of the viewport's size, so it
 * keeps its place when the viewport changes size (rotating, full screen).
 */
data class PcZoom(val scale: Float = 1f, val x: Float = 0f, val y: Float = 0f) {
    val zoomed: Boolean get() = scale > 1f
}

/** What a two-finger touch on the PC's screen turns out to be. */
private enum class TwoFingerKind { Undecided, Scroll, Zoom }

/**
 * How long a touch waits before pressing on the PC, so a second finger landing
 * (to zoom or scroll) doesn't click first. Moving sooner presses at once.
 */
private const val PRESS_HOLD_MS = 90L

/**
 * Pure coordinate and orientation math for the PC screen viewer (`PcMirrorView`),
 * shared between the live composable and unit tests in `RemoteMathTest`.
 */
object PcMirrorMath {
    private const val DEFAULT_ASPECT = 16f / 9f

    /**
     * Computes the exact fitted video rectangle inside a container of `(containerWidth, containerHeight)`
     * while preserving `videoAspect` (`width / height`), letterboxing or pillarboxing as needed.
     */
    fun computeVideoViewport(
        containerWidth: Float,
        containerHeight: Float,
        videoAspect: Float,
    ): VideoViewportRect {
        val cw = containerWidth.coerceAtLeast(1f)
        val ch = containerHeight.coerceAtLeast(1f)
        val aspect = if (videoAspect.isFinite() && videoAspect > 0f) videoAspect else DEFAULT_ASPECT
        val containerAspect = cw / ch

        return if (containerAspect >= aspect) {
            // Container is wider than the video: pillarbox (bars on left and right).
            val fittedHeight = ch
            val fittedWidth = (ch * aspect).coerceAtMost(cw)
            val left = (cw - fittedWidth) * 0.5f
            VideoViewportRect(
                left = left,
                top = 0f,
                width = fittedWidth,
                height = fittedHeight,
            )
        } else {
            // Container is taller than the video: letterbox (bars on top and bottom).
            val fittedWidth = cw
            val fittedHeight = (cw / aspect).coerceAtMost(ch)
            val top = (ch - fittedHeight) * 0.5f
            VideoViewportRect(
                left = 0f,
                top = top,
                width = fittedWidth,
                height = fittedHeight,
            )
        }
    }

    /**
     * Maps a touch point `(touchX, touchY)` in container pixel coordinates into normalized
     * PC desktop coordinates in `[0f, 1f]`, accounting for letterbox or pillarbox bars.
     *
     * When `clampOutside` is `false` and the point falls in the black letterbox/pillarbox bars,
     * returns `null` so an initial tap on a black bar does not click the edge of the PC screen.
     */
    fun mapTouchToNormalized(
        touchX: Float,
        touchY: Float,
        viewport: VideoViewportRect,
        clampOutside: Boolean = true,
    ): Pair<Float, Float>? {
        if (!clampOutside && !viewport.contains(touchX, touchY)) {
            return null
        }
        val vw = viewport.width.coerceAtLeast(1f)
        val vh = viewport.height.coerceAtLeast(1f)
        val nx = ((touchX - viewport.left) / vw).coerceIn(0f, 1f)
        val ny = ((touchY - viewport.top) / vh).coerceIn(0f, 1f)
        return Pair(nx, ny)
    }

    /**
     * Convenience overload that computes the fitted viewport for `(containerWidth, containerHeight)`
     * and maps `(touchX, touchY)` to normalized PC coordinates in `[0f, 1f]`.
     */
    fun mapTouchToNormalized(
        touchX: Float,
        touchY: Float,
        containerWidth: Float,
        containerHeight: Float,
        videoAspect: Float,
        clampOutside: Boolean = true,
    ): Pair<Float, Float>? {
        val viewport = computeVideoViewport(containerWidth, containerHeight, videoAspect)
        return mapTouchToNormalized(touchX, touchY, viewport, clampOutside)
    }

    /** The most the PC's screen zooms in. */
    const val MAX_ZOOM = 5f

    /** Below this a zoom snaps back to the whole screen. */
    private const val ZOOM_SNAP = 1.03f

    /**
     * [zoom] after a pinch: scaled by [factor] around the fingers' midpoint
     * ([focusX], [focusY], in container pixels) and moved by ([panX], [panY]).
     * What's under the fingers stays under them, and the picture always fills
     * [viewport], so no black shows at its edges.
     */
    fun pinch(
        zoom: PcZoom,
        viewport: VideoViewportRect,
        focusX: Float,
        focusY: Float,
        factor: Float,
        panX: Float,
        panY: Float,
    ): PcZoom {
        val vw = viewport.width.coerceAtLeast(1f)
        val vh = viewport.height.coerceAtLeast(1f)
        val step = if (factor.isFinite() && factor > 0f) factor else 1f
        val scale = (zoom.scale * step).coerceIn(1f, MAX_ZOOM)
        if (scale < ZOOM_SNAP) return PcZoom()
        fun shift(focus: Float, start: Float, size: Float, old: Float, pan: Float): Float {
            // The picture's point under the fingers before the pinch...
            val under = (focus - start - old * size) / zoom.scale
            // ...stays under them after it.
            val moved = focus + pan - start - under * scale
            return (moved / size).coerceIn(1f - scale, 0f)
        }
        return PcZoom(
            scale = scale,
            x = shift(focusX, viewport.left, vw, zoom.x, panX),
            y = shift(focusY, viewport.top, vh, zoom.y, panY),
        )
    }

    /**
     * Where a touch at ([touchX], [touchY]) lands on the unzoomed picture, in
     * container pixels, so [mapTouchToNormalized] finds the PC's point under
     * the finger.
     */
    fun unzoom(touchX: Float, touchY: Float, viewport: VideoViewportRect, zoom: PcZoom): Pair<Float, Float> {
        if (!zoom.zoomed) return touchX to touchY
        val vw = viewport.width.coerceAtLeast(1f)
        val vh = viewport.height.coerceAtLeast(1f)
        return Pair(
            viewport.left + (touchX - viewport.left - zoom.x * vw) / zoom.scale,
            viewport.top + (touchY - viewport.top - zoom.y * vh) / zoom.scale,
        )
    }

    fun isPhysicalLandscape(degrees: Int): Boolean =
        degrees in 60..120 || degrees in 240..300

    fun isPhysicalPortrait(degrees: Int): Boolean =
        (degrees in 0..30) || (degrees in 330..359) || (degrees in 150..210)

    fun shouldAutoEnterFullScreen(
        sensorDegrees: Int,
        autoRotateEnabled: Boolean,
        isFullScreen: Boolean,
        suppressWhileInLandscape: Boolean,
    ): Boolean =
        autoRotateEnabled &&
            !isFullScreen &&
            !suppressWhileInLandscape &&
            isPhysicalLandscape(sensorDegrees)

    fun shouldAutoExitFullScreen(
        sensorDegrees: Int,
        autoRotateEnabled: Boolean,
        isFullScreen: Boolean,
        enteredViaButton: Boolean,
        sawPhysicalLandscapeSinceEnter: Boolean,
    ): Boolean =
        autoRotateEnabled &&
            isFullScreen &&
            (!enteredViaButton || sawPhysicalLandscapeSinceEnter) &&
            isPhysicalPortrait(sensorDegrees)
}

/**
 * Extracts leading H.264 SPS (NAL type 7) and PPS (NAL type 8) parameter sets from an Annex-B
 * keyframe so hardware decoders on Qualcomm/Exynos/MediaTek receive explicit `BUFFER_FLAG_CODEC_CONFIG`.
 */
private fun extractH264CodecConfig(data: ByteArray): Pair<ByteArray, ByteArray>? {
    var i = 0
    var firstPlayableNalOffset = -1
    var sawSpsOrPps = false
    while (i + 4 < data.size) {
        val scLen = if (data[i].toInt() == 0 && data[i + 1].toInt() == 0 && data[i + 2].toInt() == 1) {
            3
        } else if (i + 4 < data.size && data[i].toInt() == 0 && data[i + 1].toInt() == 0 && data[i + 2].toInt() == 0 && data[i + 3].toInt() == 1) {
            4
        } else {
            i++
            continue
        }
        val nalHeaderIdx = i + scLen
        if (nalHeaderIdx >= data.size) break
        val nalType = data[nalHeaderIdx].toInt() and 0x1F
        if (nalType == 7 || nalType == 8) {
            sawSpsOrPps = true
        } else if (nalType == 5 || nalType == 1) {
            firstPlayableNalOffset = i
            break
        }
        i = nalHeaderIdx + 1
    }
    return if (sawSpsOrPps && firstPlayableNalOffset > 0 && firstPlayableNalOffset < data.size) {
        Pair(
            data.copyOfRange(0, firstPlayableNalOffset),
            data.copyOfRange(firstPlayableNalOffset, data.size),
        )
    } else {
        null
    }
}

private fun isSystemAutoRotateOn(context: Context): Boolean =
    runCatching {
        Settings.System.getInt(
            context.contentResolver,
            Settings.System.ACCELEROMETER_ROTATION,
            0,
        ) == 1
    }.getOrDefault(false)

/**
 * Live PC-to-Phone Reverse Mirroring viewer ("View Your PC").
 * Decodes the PC's low-latency H.264/H.265/AV1 stream onto a single persistent [TextureView]
 * across portrait and immersive landscape full-screen modes, mapping touch, drag,
 * two-finger scroll, right-click, and keyboard input back to the PC.
 *
 * With [watch] (Play on phone), it shows what that PC player plays instead, full screen,
 * and sends the PC nothing: a tap calls [onTap].
 *
 * With [touchpadSensitivity], full screen offers a Touchpad switch: the whole
 * screen then works like a laptop's touchpad instead of pointing directly.
 */
/** How wide the space beside the picture has to be to hold the controls. */
private val MIN_RAIL_WIDTH = 72.dp

/** The tallest and shortest a full-screen control gets. */
private val RAIL_TILE_MAX = 56.dp
private val RAIL_TILE_MIN = 40.dp
private val RAIL_TILE_MAX_WIDTH = 132.dp
private val RAIL_GAP = 6.dp
private val RAIL_SHAPE = RoundedCornerShape(12.dp)

/**
 * A column of full-screen controls in the space beside the picture. Both
 * columns share one tile size ([tiles] is the longer one's count), so the two
 * sides line up and every control has the same shape.
 */
@Composable
private fun FullScreenRail(
    width: Dp,
    tiles: Int,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.(tileHeight: Dp) -> Unit,
) {
    BoxWithConstraints(
        modifier = modifier
            .fillMaxHeight()
            .width(width)
            // Taps between the buttons stay here instead of reaching the picture.
            .pointerInput(Unit) { detectTapGestures { } },
    ) {
        val vertical = 12.dp
        val fits = (maxHeight - vertical * 2 - RAIL_GAP * (tiles - 1)) / tiles.coerceAtLeast(1)
        val tileHeight = fits.coerceIn(RAIL_TILE_MIN, RAIL_TILE_MAX)
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 6.dp, vertical = vertical),
            verticalArrangement = Arrangement.spacedBy(RAIL_GAP, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            content(tileHeight)
        }
    }
}

@Composable
private fun RailButton(
    label: String,
    height: Dp,
    selected: Boolean = false,
    description: String = label,
    onClick: () -> Unit,
) {
    RailTile(height = height, selected = selected, description = description, onClick = onClick) {
        // One word stays whole on one line and shrinks to fit; two words may
        // take two lines, broken between them.
        val words = label.trim().split(' ').count { it.isNotEmpty() }
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            textAlign = TextAlign.Center,
            maxLines = if (words > 1) 2 else 1,
            softWrap = words > 1,
            autoSize = TextAutoSize.StepBased(
                minFontSize = 9.sp,
                maxFontSize = MaterialTheme.typography.labelLarge.fontSize,
                stepSize = 0.5.sp,
            ),
        )
    }
}

/** A button label on one line that shrinks to fit, rather than cut a word off, on narrow phones or with large fonts. */
@Composable
private fun FitLabel(text: String) {
    val size = LocalTextStyle.current.fontSize.takeIf { it.isSpecified } ?: 14.sp
    Text(
        text,
        maxLines = 1,
        softWrap = false,
        autoSize = TextAutoSize.StepBased(minFontSize = 9.sp, maxFontSize = size, stepSize = 0.5.sp),
    )
}

/** The one shape and size every full-screen control has. */
@Composable
private fun RailTile(
    height: Dp,
    selected: Boolean,
    description: String,
    onClick: () -> Unit,
    content: @Composable () -> Unit,
) {
    Surface(
        onClick = onClick,
        shape = RAIL_SHAPE,
        // Selected reads like the bar's tonal Enter button: light on the black.
        color = if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.White.copy(alpha = 0.08f),
        contentColor = if (selected) MaterialTheme.colorScheme.onSecondaryContainer else Color.White,
        modifier = Modifier
            // A wide black margin (a tablet, a narrow PC screen) doesn't stretch them into slabs.
            .widthIn(max = RAIL_TILE_MAX_WIDTH)
            .fillMaxWidth()
            .height(height)
            .semantics {
                contentDescription = description
                this.selected = selected
            },
    ) {
        Box(contentAlignment = Alignment.Center, modifier = Modifier.padding(horizontal = 6.dp)) {
            content()
        }
    }
}

@Composable
fun PcMirrorView(
    pcId: String,
    pcName: String,
    core: Core,
    modifier: Modifier = Modifier,
    isFullScreen: Boolean = false,
    onFullScreenChange: (Boolean) -> Unit = {},
    watch: String? = null,
    onTap: () -> Unit = {},
    touchpadSensitivity: Float? = null,
    onStatus: (Core.RemoteAccess) -> Unit = {},
) {
    val context = LocalContext.current
    val view = LocalView.current
    val configuration = LocalConfiguration.current
    val reducedMotion = LocalReducedMotion.current
    val keyboardController = LocalSoftwareKeyboardController.current
    val keyboardFocusRequester = remember { FocusRequester() }

    val mirrorConfig by core.pcMirrorConfig.collectAsStateWithLifecycle()
    val activeConfig = mirrorConfig?.takeIf { it.pcId == pcId }
    val effectiveCodec = activeConfig?.codec ?: "h264"

    // Video looks its best at 1080p; the PC's desktop starts at 720p.
    var highQuality by rememberSaveable { mutableStateOf(watch != null) }
    var showKeyboardBar by rememberSaveable { mutableStateOf(false) }
    // Full screen only: the screen moves the pointer like a touchpad, for precision.
    var touchpadMode by rememberSaveable { mutableStateOf(false) }
    val touchpadActive = isFullScreen && touchpadMode && touchpadSensitivity != null && watch == null
    var typed by rememberSaveable(stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue("")) }
    // Pinched in on the PC's screen. Touchpad mode moves the PC's own pointer,
    // so it shows the whole screen.
    var zoom by remember(pcId) { mutableStateOf(PcZoom()) }
    LaunchedEffect(touchpadActive) { if (touchpadActive) zoom = PcZoom() }
    var surface by remember { mutableStateOf<Surface?>(null) }
    var hasRenderedFrame by remember(pcId) { mutableStateOf(false) }
    var reconnectTick by remember { mutableIntStateOf(0) }

    // Full-screen overlay & auto-rotate state
    var overlayVisible by remember { mutableStateOf(true) }
    var overlayRevealTick by remember { mutableIntStateOf(0) }
    var enteredViaButton by rememberSaveable { mutableStateOf(false) }
    var sawPhysicalLandscapeSinceEnter by rememberSaveable { mutableStateOf(false) }
    var suppressAutoEnterInLandscape by rememberSaveable { mutableStateOf(false) }
    var autoRotateEnabled by remember { mutableStateOf(isSystemAutoRotateOn(context)) }

    val currentIsFullScreen by rememberUpdatedState(isFullScreen)
    val currentOnFullScreenChange by rememberUpdatedState(onFullScreenChange)

    val initialRequestedOrientation = remember {
        (context as? Activity)?.requestedOrientation ?: ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
    }

    fun bumpOverlay() {
        overlayVisible = true
        overlayRevealTick++
    }

    fun enterFullScreen(fromButton: Boolean) {
        enteredViaButton = fromButton
        sawPhysicalLandscapeSinceEnter = !fromButton
        overlayVisible = true
        overlayRevealTick++
        currentOnFullScreenChange(true)
    }

    fun exitFullScreen(fromManualCollapse: Boolean) {
        if (fromManualCollapse) {
            suppressAutoEnterInLandscape = true
        }
        enteredViaButton = false
        sawPhysicalLandscapeSinceEnter = false
        currentOnFullScreenChange(false)
    }

    BackHandler(enabled = isFullScreen) {
        exitFullScreen(fromManualCollapse = true)
    }

    // Observe Settings.System.ACCELEROMETER_ROTATION changes in real time.
    DisposableEffect(context) {
        autoRotateEnabled = isSystemAutoRotateOn(context)
        val uri = Settings.System.getUriFor(Settings.System.ACCELEROMETER_ROTATION)
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                autoRotateEnabled = isSystemAutoRotateOn(context)
            }
        }
        if (uri != null) {
            runCatching {
                context.contentResolver.registerContentObserver(uri, false, observer)
            }
        }
        onDispose {
            runCatching { context.contentResolver.unregisterContentObserver(observer) }
        }
    }

    // Listen to physical orientation via OrientationEventListener so auto-rotate into and out of
    // full-screen works even while requestedOrientation is locked to SENSOR_LANDSCAPE.
    DisposableEffect(context) {
        val listener = object : OrientationEventListener(context) {
            override fun onOrientationChanged(orientation: Int) {
                if (orientation == ORIENTATION_UNKNOWN) return
                val sysAuto = isSystemAutoRotateOn(context)
                autoRotateEnabled = sysAuto
                if (PcMirrorMath.isPhysicalLandscape(orientation)) {
                    if (currentIsFullScreen) {
                        sawPhysicalLandscapeSinceEnter = true
                    } else if (
                        PcMirrorMath.shouldAutoEnterFullScreen(
                            sensorDegrees = orientation,
                            autoRotateEnabled = sysAuto,
                            isFullScreen = currentIsFullScreen,
                            suppressWhileInLandscape = suppressAutoEnterInLandscape,
                        )
                    ) {
                        enterFullScreen(fromButton = false)
                    }
                } else if (PcMirrorMath.isPhysicalPortrait(orientation)) {
                    if (suppressAutoEnterInLandscape) {
                        suppressAutoEnterInLandscape = false
                        val activity = context as? Activity
                        if (!currentIsFullScreen && sysAuto) {
                            activity?.requestedOrientation = initialRequestedOrientation
                        }
                    }
                    if (
                        PcMirrorMath.shouldAutoExitFullScreen(
                            sensorDegrees = orientation,
                            autoRotateEnabled = sysAuto,
                            isFullScreen = currentIsFullScreen,
                            enteredViaButton = enteredViaButton,
                            sawPhysicalLandscapeSinceEnter = sawPhysicalLandscapeSinceEnter,
                        )
                    ) {
                        exitFullScreen(fromManualCollapse = false)
                    }
                }
            }
        }
        // Watching stays full screen, however the phone is held.
        if (watch == null && listener.canDetectOrientation()) {
            listener.enable()
        }
        onDispose {
            listener.disable()
        }
    }

    // Also follow configuration orientation changes when system auto-rotate is enabled
    // (for example, emulator window rotation).
    LaunchedEffect(configuration.orientation, autoRotateEnabled) {
        if (!autoRotateEnabled || watch != null) return@LaunchedEffect
        if (
            configuration.orientation == Configuration.ORIENTATION_LANDSCAPE &&
            !isFullScreen &&
            !suppressAutoEnterInLandscape
        ) {
            enterFullScreen(fromButton = false)
        } else if (
            configuration.orientation == Configuration.ORIENTATION_PORTRAIT &&
            isFullScreen &&
            !enteredViaButton
        ) {
            exitFullScreen(fromManualCollapse = false)
        }
    }

    var appliedFullScreen by remember { mutableStateOf(false) }

    // Apply orientation, keep-screen-on, and immersive system bars for full-screen vs portrait.
    LaunchedEffect(isFullScreen, autoRotateEnabled, suppressAutoEnterInLandscape, view) {
        val activity = context as? Activity ?: return@LaunchedEffect
        val window = activity.window ?: return@LaunchedEffect
        val controller = WindowCompat.getInsetsController(window, view)
        view.keepScreenOn = true
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        if (isFullScreen) {
            appliedFullScreen = true
            activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            controller.systemBarsBehavior =
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
        } else if (appliedFullScreen) {
            controller.show(WindowInsetsCompat.Type.systemBars())
            activity.requestedOrientation = if (!autoRotateEnabled || suppressAutoEnterInLandscape) {
                ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
            } else {
                initialRequestedOrientation
            }
        }
    }

    // Restore previous requestedOrientation and system bars when leaving PcMirrorView.
    DisposableEffect(view) {
        val activity = context as? Activity
        val window = activity?.window
        onDispose {
            if (window != null) {
                WindowCompat.getInsetsController(window, view)
                    .show(WindowInsetsCompat.Type.systemBars())
            }
            activity?.requestedOrientation = initialRequestedOrientation
        }
    }

    // Auto-fade the full-screen slim overlay 3 seconds after the last tap or action.
    LaunchedEffect(isFullScreen, overlayVisible, overlayRevealTick) {
        if (isFullScreen && overlayVisible) {
            delay(3_000L)
            overlayVisible = false
        }
    }

    // Focus the text field and open the IME when the keyboard bar is toggled on.
    LaunchedEffect(showKeyboardBar) {
        if (showKeyboardBar) {
            runCatching { keyboardFocusRequester.requestFocus() }
            keyboardController?.show()
        } else {
            keyboardController?.hide()
        }
    }

    val maxSize = if (highQuality) 1920 else 1280
    val bitrate = if (highQuality) 8_000_000 else 4_000_000

    // Nobody's watching once the phone is on something else or its screen is off, and video
    // costs the PC its CPU and the phone its data. Play on phone keeps going: its sound does.
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val showing = watch != null || lifecycleState.isAtLeast(Lifecycle.State.STARTED)
    var asked by remember(pcId) { mutableStateOf(false) }

    LaunchedEffect(pcId, maxSize, bitrate, showing) {
        hasRenderedFrame = false
        if (!showing) {
            if (asked) {
                android.util.Log.i("PcMirrorView", "stopPcMirror, not on screen")
                core.stopPcMirror(pcId)
                asked = false
            }
            return@LaunchedEffect
        }
        asked = true
        android.util.Log.i(
            "PcMirrorView",
            "startPcMirror maxSize=$maxSize fps=30 bitrate=$bitrate",
        )
        core.startPcMirror(pcId, maxSize = maxSize, fps = 30, bitrate = bitrate, watch = watch)
    }

    DisposableEffect(pcId) {
        onDispose {
            android.util.Log.i("PcMirrorView", "stopPcMirror")
            core.stopPcMirror(pcId)
        }
    }

    val decoderMutex = remember { Mutex() }
    var useSoftwareDecoder by remember(pcId, effectiveCodec) { mutableStateOf(false) }

    // Hardware MediaCodec decode loop bound to the current Surface and normalized codec.
    // Resolution and orientation changes keep the same Surface and MediaCodec instance.
    LaunchedEffect(
        pcId,
        surface,
        effectiveCodec,
        useSoftwareDecoder,
        reconnectTick,
    ) {
        val targetSurface = surface?.takeIf { it.isValid } ?: return@LaunchedEffect
        hasRenderedFrame = false
        val width = (activeConfig?.width ?: 1280).coerceAtLeast(64)
        val height = (activeConfig?.height ?: 720).coerceAtLeast(64)
        val mime = when (effectiveCodec) {
            "h265" -> MediaFormat.MIMETYPE_VIDEO_HEVC
            "av1" -> "video/av01"
            else -> MediaFormat.MIMETYPE_VIDEO_AVC
        }

        withContext(Dispatchers.Default) {
            decoderMutex.withLock {
                ensureActive()
                var codec: MediaCodec? = null
                val codecLock = Any()
                try {
                    val created = createVideoDecoder(mime, preferSoftware = useSoftwareDecoder)
                    codec = created
                    ensureActive()
                    val lowLatencyFormat = MediaFormat.createVideoFormat(mime, width, height).apply {
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                            setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
                        }
                        setInteger(MediaFormat.KEY_PRIORITY, 0)
                    }
                    try {
                        created.configure(lowLatencyFormat, targetSurface, null, 0)
                    } catch (_: Throwable) {
                        ensureActive()
                        runCatching { created.reset() }
                        val plainFormat = MediaFormat.createVideoFormat(mime, width, height)
                        created.configure(plainFormat, targetSurface, null, 0)
                    }
                    ensureActive()
                    created.start()

                    fun queueRaw(bytes: ByteArray, timeUs: Long, flags: Int): Boolean = synchronized(codecLock) {
                        val inIdx = created.dequeueInputBuffer(10_000L)
                        if (inIdx < 0) return false
                        val inBuf = created.getInputBuffer(inIdx)
                        if (inBuf != null && bytes.size <= inBuf.capacity()) {
                            inBuf.clear()
                            inBuf.put(bytes)
                            created.queueInputBuffer(inIdx, 0, bytes.size, timeUs, flags)
                            return true
                        } else {
                            created.queueInputBuffer(inIdx, 0, 0, 0L, 0)
                            return false
                        }
                    }

                    fun drainOutputs(renderedLocal: java.util.concurrent.atomic.AtomicBoolean): Boolean {
                        var newlyRendered = false
                        val bufferInfo = MediaCodec.BufferInfo()
                        synchronized(codecLock) {
                            while (targetSurface.isValid) {
                                val outIdx = created.dequeueOutputBuffer(bufferInfo, 0L)
                                if (outIdx >= 0) {
                                    created.releaseOutputBuffer(outIdx, true)
                                    if (renderedLocal.compareAndSet(false, true)) {
                                        newlyRendered = true
                                    }
                                } else if (outIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                                    continue
                                } else {
                                    break
                                }
                            }
                        }
                        return newlyRendered
                    }

                    fun queuePacket(
                        packet: Core.PcMirrorPacket,
                        renderedLocal: java.util.concurrent.atomic.AtomicBoolean,
                    ): Boolean {
                        if (packet.keyframe && effectiveCodec == "h264") {
                            val split = extractH264CodecConfig(packet.data)
                            if (split != null) {
                                val (csd, idrSlice) = split
                                queueRaw(csd, packet.timeUs, MediaCodec.BUFFER_FLAG_CODEC_CONFIG)
                                drainOutputs(renderedLocal)
                                if (!queueRaw(idrSlice, packet.timeUs, MediaCodec.BUFFER_FLAG_KEY_FRAME)) {
                                    core.requestPcMirrorKeyframe(pcId)
                                }
                                return drainOutputs(renderedLocal)
                            }
                        }
                        val flags = if (packet.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
                        if (!queueRaw(packet.data, packet.timeUs, flags)) {
                            core.requestPcMirrorKeyframe(pcId)
                        }
                        return drainOutputs(renderedLocal)
                    }

                    coroutineScope {
                        val renderedLocal = java.util.concurrent.atomic.AtomicBoolean(false)
                        // Periodic output drainer so static PC screens render even when no new packet arrives.
                        val drainJob = launch {
                            while (isActive && targetSurface.isValid) {
                                if (drainOutputs(renderedLocal)) {
                                    withContext(Dispatchers.Main) { hasRenderedFrame = true }
                                }
                                delay(12L)
                            }
                        }

                        // Keyframe retry watchdog until the first frame is rendered on screen.
                        val watchdogJob = launch {
                            delay(1500L)
                            while (isActive && !renderedLocal.get()) {
                                core.requestPcMirrorKeyframe(pcId)
                                delay(1500L)
                            }
                        }

                        try {
                            core.latestPcMirrorKeyframe(pcId)?.let { cached ->
                                if (targetSurface.isValid && queuePacket(cached, renderedLocal)) {
                                    withContext(Dispatchers.Main) { hasRenderedFrame = true }
                                }
                            }
                            core.pcMirrorPackets.collect { packet ->
                                if (packet.pcId != pcId || !targetSurface.isValid) return@collect
                                if (queuePacket(packet, renderedLocal)) {
                                    withContext(Dispatchers.Main) { hasRenderedFrame = true }
                                }
                            }
                        } finally {
                            watchdogJob.cancel()
                            drainJob.cancel()
                        }
                    }
                } catch (e: Throwable) {
                    if (e is CancellationException) throw e
                    android.util.Log.w("PcMirrorView", "decoder failed (software=$useSoftwareDecoder); retrying", e)
                    if (!useSoftwareDecoder) {
                        withContext(Dispatchers.Main) { useSoftwareDecoder = true }
                        core.requestPcMirrorKeyframe(pcId)
                    } else {
                        delay(800L)
                        withContext(Dispatchers.Main) { reconnectTick++ }
                    }
                } finally {
                    synchronized(codecLock) {
                        runCatching { codec?.stop() }
                        runCatching { codec?.release() }
                    }
                }
            }
        }
    }

    val aspect = remember(activeConfig?.width, activeConfig?.height) {
        val w = activeConfig?.width ?: 16
        val h = activeConfig?.height ?: 9
        if (w > 0 && h > 0) w.toFloat() / h.toFloat() else 16f / 9f
    }
    val currentAspect by rememberUpdatedState(aspect)

    // The black space beside the picture, where full screen puts its controls.
    var viewportSize by remember { mutableStateOf(IntSize.Zero) }
    val railWidth = with(LocalDensity.current) {
        if (viewportSize.width <= 0 || viewportSize.height <= 0) {
            0.dp
        } else {
            PcMirrorMath.computeVideoViewport(
                viewportSize.width.toFloat(),
                viewportSize.height.toFloat(),
                aspect,
            ).left.toDp()
        }
    }
    val useRails = isFullScreen && watch == null && railWidth >= MIN_RAIL_WIDTH
    val viewportDescription = stringResource(R.string.pc_mirror_viewport_desc, pcName)
    val fullScreenLabel = stringResource(R.string.pc_mirror_fullscreen)
    val exitFullScreenLabel = stringResource(R.string.pc_mirror_exit_fullscreen)
    val rightClickLabel = stringResource(R.string.pc_mirror_right_click)
    val taskViewLabel = stringResource(R.string.pc_mirror_task_view)
    val touchpadLabel = stringResource(R.string.remote_mode_touchpad)
    val escLabel = stringResource(R.string.remote_key_esc)
    val escDescription = stringResource(R.string.remote_key_esc_desc)
    val copyLabel = stringResource(R.string.remote_key_copy)
    val pasteLabel = stringResource(R.string.remote_key_paste)
    val undoLabel = stringResource(R.string.remote_key_undo)
    val switchWindowLabel = stringResource(R.string.deck_default_switch_window)
    val keyboardLabel = stringResource(
        if (showKeyboardBar) R.string.pc_mirror_hide_keyboard else R.string.pc_mirror_keyboard,
    )
    val qualityShortLabel = stringResource(
        if (highQuality) R.string.pc_mirror_quality_1080p else R.string.pc_mirror_quality_720p,
    )
    val qualityA11yLabel = stringResource(R.string.pc_mirror_quality_desc, qualityShortLabel)

    // Unified Column so AndroidView(TextureView) stays in the exact same composition slot
    // across portrait and full-screen landscape transitions.
    Column(
        modifier = modifier
            .fillMaxSize()
            .then(
                if (isFullScreen) {
                    Modifier
                        .background(Color.Black)
                        .imePadding()
                } else {
                    Modifier
                },
            ),
        verticalArrangement = Arrangement.spacedBy(if (isFullScreen) 0.dp else 10.dp),
    ) {
        // Video viewport container: rounded card in portrait, edge-to-edge black canvas in full screen.
        val outerShape = if (isFullScreen) RoundedCornerShape(0.dp) else RoundedCornerShape(18.dp)
        val innerShape = if (isFullScreen) RoundedCornerShape(0.dp) else RoundedCornerShape(12.dp)

        Box(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .clip(outerShape)
                .background(Color.Black)
                .onSizeChanged { viewportSize = it },
        ) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .semantics { contentDescription = viewportDescription }
                    .pointerInput(pcId, watch, touchpadActive, touchpadSensitivity) {
                        if (watch != null) {
                            // Watching: nothing goes to the PC.
                            detectTapGestures(onTap = { onTap() })
                            return@pointerInput
                        }
                        if (touchpadActive && touchpadSensitivity != null) {
                            touchpadGestures(
                                pcId = pcId,
                                core = core,
                                sensitivity = touchpadSensitivity,
                                onStatus = onStatus,
                                onTap = { bumpOverlay() },
                            )
                            return@pointerInput
                        }
                        val tapSlop = viewConfiguration.touchSlop
                        val scrollStepPx = 16.dp.toPx().coerceAtLeast(1f)
                        val tapTimeoutMs = 280L
                        val doubleTapWindowMs = 300L
                        var lastTapUpTimeMs = 0L
                        var lastTapPos = Offset.Zero

                        awaitEachGesture {
                            val cw = size.width.toFloat().coerceAtLeast(1f)
                            val ch = size.height.toFloat().coerceAtLeast(1f)
                            val viewport = PcMirrorMath.computeVideoViewport(cw, ch, currentAspect)

                            // The PC's point under a finger, through the zoom.
                            fun pcPoint(at: Offset): Pair<Float, Float>? {
                                val (x, y) = PcMirrorMath.unzoom(at.x, at.y, viewport, zoom)
                                return PcMirrorMath.mapTouchToNormalized(x, y, viewport, clampOutside = true)
                            }

                            val first = awaitFirstDown(requireUnconsumed = false)
                            val downTime = System.currentTimeMillis()
                            val downPos = first.position
                            // A touch on the black bars beside the picture doesn't click its edge.
                            val initialMapped = if (viewport.contains(downPos.x, downPos.y)) pcPoint(downPos) else null
                            val touchStartedInVideo = initialMapped != null
                            var lastX = initialMapped?.first ?: 0.5f
                            var lastY = initialMapped?.second ?: 0.5f
                            var twoFingerGesture = false
                            var didTwoFingerScroll = false
                            var didPinch = false
                            var maxPointers = 1
                            var totalMoved = 0f
                            var scrollAccumX = 0f
                            var scrollAccumY = 0f
                            // Two fingers: undecided until they spread or close (zoom) or
                            // move together (scroll) past the touch slop.
                            var twoFingerKind = TwoFingerKind.Undecided
                            var spreadAtStart = 0f
                            var centroidAtStart = Offset.Zero

                            // The press waits a moment, so a second finger landing to zoom or
                            // scroll never clicks the PC first. Moving or holding sends it.
                            var downSent = false
                            fun sendDown() {
                                if (downSent || !touchStartedInVideo) return
                                downSent = true
                                core.sendPcMirrorInput(pcId, MirrorInputEvent.Touch(TouchPhase.DOWN, lastX, lastY))
                            }

                            while (true) {
                                val holding = touchStartedInVideo && !downSent && !twoFingerGesture
                                val event = if (holding) {
                                    val waited = System.currentTimeMillis() - downTime
                                    withTimeoutOrNull((PRESS_HOLD_MS - waited).coerceAtLeast(1L)) { awaitPointerEvent() }
                                } else {
                                    awaitPointerEvent()
                                }
                                if (event == null) {
                                    sendDown()
                                    continue
                                }
                                val changes = event.changes
                                var pressedCount = 0
                                var primaryChange: androidx.compose.ui.input.pointer.PointerInputChange? = null
                                for (i in 0 until changes.size) {
                                    val c = changes[i]
                                    if (c.pressed) {
                                        if (primaryChange == null) primaryChange = c
                                        pressedCount++
                                    }
                                }
                                if (pressedCount > maxPointers) {
                                    maxPointers = pressedCount
                                }
                                if (pressedCount == 0 || primaryChange == null) {
                                    if (touchStartedInVideo && !twoFingerGesture) {
                                        sendDown()
                                        core.sendPcMirrorInput(
                                            pcId,
                                            MirrorInputEvent.Touch(TouchPhase.UP, lastX, lastY),
                                        )
                                    }
                                    break
                                }
                                if (pressedCount >= 2) {
                                    if (!twoFingerGesture) {
                                        twoFingerGesture = true
                                        // Only a press already sent (a drag) needs letting go.
                                        if (downSent) {
                                            core.sendPcMirrorInput(
                                                pcId,
                                                MirrorInputEvent.Touch(TouchPhase.UP, lastX, lastY),
                                            )
                                        }
                                        spreadAtStart = event.calculateCentroidSize()
                                        centroidAtStart = event.calculateCentroid()
                                    }
                                    if (twoFingerKind == TwoFingerKind.Undecided) {
                                        val spread = event.calculateCentroidSize()
                                        val centroid = event.calculateCentroid()
                                        twoFingerKind = when {
                                            abs(spread - spreadAtStart) > tapSlop -> TwoFingerKind.Zoom
                                            (centroid - centroidAtStart).getDistance() > tapSlop -> TwoFingerKind.Scroll
                                            else -> TwoFingerKind.Undecided
                                        }
                                    }
                                    when (twoFingerKind) {
                                        TwoFingerKind.Zoom -> {
                                            val centroid = event.calculateCentroid()
                                            val pan = event.calculatePan()
                                            if (centroid.isSpecified) {
                                                didPinch = true
                                                zoom = PcMirrorMath.pinch(
                                                    zoom = zoom,
                                                    viewport = viewport,
                                                    focusX = centroid.x - pan.x,
                                                    focusY = centroid.y - pan.y,
                                                    factor = event.calculateZoom(),
                                                    panX = pan.x,
                                                    panY = pan.y,
                                                )
                                            }
                                        }
                                        TwoFingerKind.Scroll -> {
                                            val prev = primaryChange.previousPosition
                                            val cur = primaryChange.position
                                            val d = primaryChange.positionChange()
                                            totalMoved += hypot(d.x, d.y)
                                            scrollAccumY += (prev.y - cur.y) / scrollStepPx
                                            scrollAccumX += (cur.x - prev.x) / scrollStepPx
                                            if (abs(scrollAccumX) > 0.05f || abs(scrollAccumY) > 0.05f) {
                                                didTwoFingerScroll = true
                                                val sendDx = scrollAccumX.coerceIn(-10f, 10f)
                                                val sendDy = scrollAccumY.coerceIn(-10f, 10f)
                                                scrollAccumX = 0f
                                                scrollAccumY = 0f
                                                core.sendPcMirrorInput(
                                                    pcId,
                                                    MirrorInputEvent.Scroll(lastX, lastY, sendDx, sendDy),
                                                )
                                            }
                                        }
                                        TwoFingerKind.Undecided -> Unit
                                    }
                                    for (i in 0 until changes.size) {
                                        changes[i].consume()
                                    }
                                } else if (!twoFingerGesture && touchStartedInVideo) {
                                    val cur = primaryChange.position
                                    val d = primaryChange.positionChange()
                                    totalMoved += hypot(d.x, d.y)
                                    // A drag starts where the finger went down.
                                    if (totalMoved > tapSlop) sendDown()
                                    val mapped = if (downSent) pcPoint(cur) else null
                                    if (mapped != null) {
                                        val (nx, ny) = mapped
                                        if (abs(nx - lastX) > 0.002f || abs(ny - lastY) > 0.002f) {
                                            lastX = nx
                                            lastY = ny
                                            core.sendPcMirrorInput(
                                                pcId,
                                                MirrorInputEvent.Touch(TouchPhase.MOVE, lastX, lastY),
                                            )
                                        }
                                    }
                                    for (i in 0 until changes.size) {
                                        changes[i].consume()
                                    }
                                }
                            }

                            val upTime = System.currentTimeMillis()
                            val elapsed = upTime - downTime
                            if (twoFingerGesture && !didTwoFingerScroll && !didPinch && elapsed <= tapTimeoutMs &&
                                touchStartedInVideo
                            ) {
                                // Two-finger quick tap performs a right click at the tapped PC location.
                                core.remoteButton(pcId, "right", "click")
                                lastTapUpTimeMs = 0L
                            } else if (maxPointers == 1 && elapsed <= tapTimeoutMs && totalMoved <= tapSlop * 1.5f) {
                                val isDoubleTap =
                                    (upTime - lastTapUpTimeMs) <= doubleTapWindowMs &&
                                        hypot(downPos.x - lastTapPos.x, downPos.y - lastTapPos.y) <= tapSlop * 3f
                                if (isDoubleTap && zoom.zoomed) {
                                    // Zoomed in, a double tap shows the whole screen again.
                                    lastTapUpTimeMs = 0L
                                    zoom = PcZoom()
                                } else if (isDoubleTap && !currentIsFullScreen) {
                                    lastTapUpTimeMs = 0L
                                    enterFullScreen(fromButton = true)
                                } else {
                                    lastTapUpTimeMs = upTime
                                    lastTapPos = downPos
                                    if (currentIsFullScreen) {
                                        bumpOverlay()
                                    }
                                }
                            } else {
                                lastTapUpTimeMs = 0L
                            }
                        }
                    },
            ) {
                // Single persistent TextureView positioned to the exact fitted video rectangle.
                Box(
                    modifier = Modifier
                        .layout { measurable, constraints ->
                            val cw = constraints.maxWidth.toFloat().coerceAtLeast(1f)
                            val ch = constraints.maxHeight.toFloat().coerceAtLeast(1f)
                            val vp = PcMirrorMath.computeVideoViewport(cw, ch, aspect)
                            val childW = vp.width.roundToInt().coerceIn(1, constraints.maxWidth.coerceAtLeast(1))
                            val childH = vp.height.roundToInt().coerceIn(1, constraints.maxHeight.coerceAtLeast(1))
                            val placeable = measurable.measure(Constraints.fixed(childW, childH))
                            layout(constraints.maxWidth, constraints.maxHeight) {
                                placeable.place(vp.left.roundToInt(), vp.top.roundToInt())
                            }
                        }
                        .clip(innerShape)
                        // Zoomed, inside the picture's own frame: the bars beside it stay put.
                        .graphicsLayer {
                            transformOrigin = TransformOrigin(0f, 0f)
                            scaleX = zoom.scale
                            scaleY = zoom.scale
                            translationX = zoom.x * size.width
                            translationY = zoom.y * size.height
                        },
                ) {
                    AndroidView(
                        factory = { ctx ->
                            TextureView(ctx).apply {
                                isOpaque = false
                                surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                                    private var activeSt: SurfaceTexture? = null

                                    override fun onSurfaceTextureAvailable(
                                        st: SurfaceTexture,
                                        width: Int,
                                        height: Int,
                                    ) {
                                        if (activeSt === st && surface?.isValid == true) {
                                            return
                                        }
                                        activeSt = st
                                        surface?.release()
                                        surface = Surface(st)
                                    }

                                    override fun onSurfaceTextureSizeChanged(
                                        st: SurfaceTexture,
                                        width: Int,
                                        height: Int,
                                    ) = Unit

                                    override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                                        if (activeSt === st) {
                                            activeSt = null
                                        }
                                        surface?.release()
                                        surface = null
                                        return true
                                    }

                                    override fun onSurfaceTextureUpdated(st: SurfaceTexture) = Unit
                                }
                            }
                        },
                        modifier = Modifier.fillMaxSize(),
                    )
                }

                if (!hasRenderedFrame) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                        modifier = Modifier
                            .align(Alignment.Center)
                            .background(Color.Black.copy(alpha = 0.65f), RoundedCornerShape(16.dp))
                            .padding(horizontal = 20.dp, vertical = 16.dp),
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(28.dp),
                            strokeWidth = 2.5.dp,
                            color = Color.White,
                        )
                        Text(
                            text = stringResource(R.string.pc_mirror_connecting, pcName),
                            style = MaterialTheme.typography.bodyMedium,
                            color = Color.White,
                        )
                    }
                }

            }

            // Expand button in the corner of the portrait viewer
            if (!isFullScreen && watch == null) {
                IconButton(
                    onClick = { enterFullScreen(fromButton = true) },
                    colors = IconButtonDefaults.iconButtonColors(
                        containerColor = Color.Black.copy(alpha = 0.58f),
                        contentColor = Color.White,
                    ),
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(10.dp)
                        .size(38.dp)
                        .semantics { contentDescription = fullScreenLabel },
                ) {
                    ExpandCollapseIcon(isFullScreen = false)
                }
            }

            // Landscape full screen leaves black space beside the picture: the controls
            // live there, always shown, and never cover the PC's screen.
            if (useRails) {
                // One tile size for both sides, from the longer one.
                val tiles = if (touchpadSensitivity != null) 6 else 5
                FullScreenRail(width = railWidth, tiles = tiles, modifier = Modifier.align(Alignment.CenterStart)) { h ->
                    if (touchpadSensitivity != null) {
                        RailButton(touchpadLabel, h, selected = touchpadMode) { touchpadMode = !touchpadMode }
                    }
                    RailButton(rightClickLabel, h) { core.remoteButton(pcId, "right", "click", onStatus) }
                    RailButton(keyboardLabel, h, selected = showKeyboardBar) { showKeyboardBar = !showKeyboardBar }
                    RailButton(escLabel, h, description = escDescription) { core.remoteKey(pcId, "escape", onStatus = onStatus) }
                    RailButton(copyLabel, h) { core.remoteKey(pcId, "copy", onStatus = onStatus) }
                    RailButton(pasteLabel, h) { core.remoteKey(pcId, "paste", onStatus = onStatus) }
                }
                FullScreenRail(width = railWidth, tiles = tiles, modifier = Modifier.align(Alignment.CenterEnd)) { h ->
                    RailButton(taskViewLabel, h) { core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("recents")) }
                    RailButton(switchWindowLabel, h) { core.remoteKey(pcId, "tab", listOf("alt"), onStatus) }
                    RailButton(undoLabel, h) { core.remoteKey(pcId, "undo", onStatus = onStatus) }
                    RailButton(qualityShortLabel, h, description = qualityA11yLabel) {
                        highQuality = !highQuality
                        reconnectTick++
                    }
                    RailTile(
                        height = h,
                        selected = false,
                        description = exitFullScreenLabel,
                        onClick = { exitFullScreen(fromManualCollapse = true) },
                    ) {
                        ExpandCollapseIcon(isFullScreen = true)
                    }
                }
            }

            // Slim overlay row in full screen: appears on tap and fades out after 3 s
            if (isFullScreen && watch == null && !useRails) {
                androidx.compose.animation.AnimatedVisibility(
                    visible = overlayVisible,
                    enter = if (reducedMotion) EnterTransition.None else fadeIn(tween(180)),
                    exit = if (reducedMotion) ExitTransition.None else fadeOut(tween(220)),
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                ) {
                    Surface(
                        shape = RoundedCornerShape(24.dp),
                        color = Color(0xCC14161B),
                        contentColor = Color.White,
                        border = BorderStroke(1.dp, Color.White.copy(alpha = 0.16f)),
                    ) {
                        Row(
                            modifier = Modifier
                                .horizontalScroll(rememberScrollState())
                                .padding(horizontal = 10.dp, vertical = 4.dp),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            TextButton(
                                onClick = {
                                    bumpOverlay()
                                    core.remoteButton(pcId, "right", "click")
                                },
                                colors = ButtonDefaults.textButtonColors(contentColor = Color.White),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                                modifier = Modifier
                                    .height(36.dp)
                                    .semantics { contentDescription = rightClickLabel },
                            ) {
                                Text(
                                    rightClickLabel,
                                    style = MaterialTheme.typography.labelLarge,
                                    maxLines = 1,
                                )
                            }
                            TextButton(
                                onClick = {
                                    bumpOverlay()
                                    core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("recents"))
                                },
                                colors = ButtonDefaults.textButtonColors(contentColor = Color.White),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                                modifier = Modifier
                                    .height(36.dp)
                                    .semantics { contentDescription = taskViewLabel },
                            ) {
                                Text(
                                    taskViewLabel,
                                    style = MaterialTheme.typography.labelLarge,
                                    maxLines = 1,
                                )
                            }
                            TextButton(
                                onClick = {
                                    bumpOverlay()
                                    showKeyboardBar = !showKeyboardBar
                                },
                                colors = ButtonDefaults.textButtonColors(
                                    contentColor = if (showKeyboardBar) {
                                        MaterialTheme.colorScheme.primary
                                    } else {
                                        Color.White
                                    },
                                ),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                                modifier = Modifier
                                    .height(36.dp)
                                    .semantics { contentDescription = keyboardLabel },
                            ) {
                                Text(
                                    keyboardLabel,
                                    style = MaterialTheme.typography.labelLarge,
                                    maxLines = 1,
                                )
                            }
                            if (touchpadSensitivity != null) {
                                TextButton(
                                    onClick = {
                                        bumpOverlay()
                                        touchpadMode = !touchpadMode
                                    },
                                    colors = ButtonDefaults.textButtonColors(
                                        contentColor = if (touchpadMode) {
                                            MaterialTheme.colorScheme.primary
                                        } else {
                                            Color.White
                                        },
                                    ),
                                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                                    modifier = Modifier
                                        .height(36.dp)
                                        .semantics {
                                            contentDescription = touchpadLabel
                                            selected = touchpadMode
                                        },
                                ) {
                                    Text(
                                        touchpadLabel,
                                        style = MaterialTheme.typography.labelLarge,
                                        maxLines = 1,
                                    )
                                }
                            }
                            TextButton(
                                onClick = {
                                    bumpOverlay()
                                    highQuality = !highQuality
                                    reconnectTick++
                                },
                                colors = ButtonDefaults.textButtonColors(contentColor = Color.White),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                                modifier = Modifier
                                    .height(36.dp)
                                    .semantics { contentDescription = qualityA11yLabel },
                            ) {
                                Text(
                                    qualityShortLabel,
                                    style = MaterialTheme.typography.labelLarge,
                                    maxLines = 1,
                                )
                            }
                            IconButton(
                                onClick = { exitFullScreen(fromManualCollapse = true) },
                                colors = IconButtonDefaults.iconButtonColors(
                                    containerColor = Color.White.copy(alpha = 0.12f),
                                    contentColor = Color.White,
                                ),
                                modifier = Modifier
                                    .size(34.dp)
                                    .semantics { contentDescription = exitFullScreenLabel },
                            ) {
                                ExpandCollapseIcon(isFullScreen = true)
                            }
                        }
                    }
                }
            }
        }

        // Keyboard input bar: sits below the weight(1f) video container inside an imePadding() Column,
        // so opening the IME shifts the video up and never covers the picture.
        if (showKeyboardBar) {
            val typePlaceholder = stringResource(R.string.remote_type_placeholder, pcName)
            val enterLabel = stringResource(R.string.pc_mirror_enter)
            val backspaceLabel = stringResource(R.string.pc_mirror_backspace)

            // Every letter reaches the PC as it's typed. The field keeps what was
            // typed so the keyboard can autocorrect; a correction goes over as
            // Backspaces and the new word.
            fun type(next: TextFieldValue) {
                val diff = typingDiff(typed.text, next.text)
                repeat(diff.backspaces) { core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("backspace")) }
                if (diff.added.isNotEmpty()) core.sendPcMirrorInput(pcId, MirrorInputEvent.Text(diff.added))
                typed = next
            }

            fun pressEnter() {
                core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("enter"))
                typed = TextFieldValue("")
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .then(
                        if (isFullScreen) {
                            Modifier
                                .background(Color(0xFF101216))
                                .padding(horizontal = 12.dp, vertical = 6.dp)
                        } else {
                            Modifier
                        },
                    ),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = typed,
                    onValueChange = {
                        type(it)
                        if (isFullScreen) bumpOverlay()
                    },
                    placeholder = { Text(typePlaceholder) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { pressEnter() }),
                    // Full screen puts the bar on black: light text, so what's typed shows.
                    colors = if (isFullScreen) {
                        OutlinedTextFieldDefaults.colors(
                            focusedTextColor = Color.White,
                            unfocusedTextColor = Color.White,
                            cursorColor = Color.White,
                            focusedBorderColor = Color.White.copy(alpha = 0.7f),
                            unfocusedBorderColor = Color.White.copy(alpha = 0.3f),
                            focusedPlaceholderColor = Color.White.copy(alpha = 0.55f),
                            unfocusedPlaceholderColor = Color.White.copy(alpha = 0.55f),
                        )
                    } else {
                        OutlinedTextFieldDefaults.colors()
                    },
                    modifier = Modifier
                        .weight(1f)
                        .focusRequester(keyboardFocusRequester)
                        .semantics { contentDescription = typePlaceholder },
                )
                FilledTonalButton(
                    onClick = { pressEnter() },
                    modifier = Modifier.semantics { contentDescription = enterLabel },
                ) {
                    Text(enterLabel)
                }
                OutlinedButton(
                    onClick = {
                        val text = typed.text
                        if (text.isEmpty()) {
                            core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("backspace"))
                        } else {
                            // The last character goes from the field and the PC alike.
                            val kept = text.substring(0, text.offsetByCodePoints(text.length, -1))
                            type(TextFieldValue(kept, TextRange(kept.length)))
                        }
                    },
                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
                    modifier = Modifier.semantics { contentDescription = backspaceLabel },
                ) {
                    Text("⌫")
                }
            }
        }

        // Portrait action bar
        if (!isFullScreen && watch == null) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    onClick = { core.remoteButton(pcId, "right", "click") },
                    modifier = Modifier
                        .weight(1f)
                        .semantics { contentDescription = rightClickLabel },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 10.dp),
                ) {
                    FitLabel(rightClickLabel)
                }
                OutlinedButton(
                    onClick = { core.sendPcMirrorInput(pcId, MirrorInputEvent.Key("recents")) },
                    modifier = Modifier
                        .weight(1f)
                        .semantics { contentDescription = taskViewLabel },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 10.dp),
                ) {
                    FitLabel(taskViewLabel)
                }
                FilledTonalButton(
                    onClick = { showKeyboardBar = !showKeyboardBar },
                    modifier = Modifier
                        .weight(1f)
                        .semantics { contentDescription = keyboardLabel },
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 10.dp),
                ) {
                    FitLabel(keyboardLabel)
                }
                OutlinedButton(
                    onClick = {
                        highQuality = !highQuality
                        reconnectTick++
                    },
                    modifier = Modifier.semantics { contentDescription = qualityA11yLabel },
                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 10.dp),
                ) {
                    FitLabel(qualityShortLabel)
                }
            }
        }
    }
}

@Composable
private fun ExpandCollapseIcon(
    isFullScreen: Boolean,
    modifier: Modifier = Modifier,
) {
    val path = remember { Path() }
    Canvas(modifier = modifier.size(18.dp)) {
        val w = size.width
        val h = size.height
        val stroke = Stroke(
            width = 1.9.dp.toPx(),
            cap = StrokeCap.Round,
            join = StrokeJoin.Round,
        )
        fun sx(v: Float) = (v / 24f) * w
        fun sy(v: Float) = (v / 24f) * h

        path.rewind()
        if (!isFullScreen) {
            // Four outward corner brackets (expand to full screen)
            path.moveTo(sx(4f), sy(9f))
            path.lineTo(sx(4f), sy(4f))
            path.lineTo(sx(9f), sy(4f))

            path.moveTo(sx(15f), sy(4f))
            path.lineTo(sx(20f), sy(4f))
            path.lineTo(sx(20f), sy(9f))

            path.moveTo(sx(20f), sy(15f))
            path.lineTo(sx(20f), sy(20f))
            path.lineTo(sx(15f), sy(20f))

            path.moveTo(sx(9f), sy(20f))
            path.lineTo(sx(4f), sy(20f))
            path.lineTo(sx(4f), sy(15f))
        } else {
            // Four inward corner brackets (collapse full screen)
            path.moveTo(sx(4f), sy(9f))
            path.lineTo(sx(9f), sy(9f))
            path.lineTo(sx(9f), sy(4f))

            path.moveTo(sx(15f), sy(4f))
            path.lineTo(sx(15f), sy(9f))
            path.lineTo(sx(20f), sy(9f))

            path.moveTo(sx(20f), sy(15f))
            path.lineTo(sx(15f), sy(15f))
            path.lineTo(sx(15f), sy(20f))

            path.moveTo(sx(9f), sy(20f))
            path.lineTo(sx(9f), sy(15f))
            path.lineTo(sx(4f), sy(15f))
        }
        drawPath(path = path, color = Color.White, style = stroke)
    }
}

private fun createVideoDecoder(mime: String, preferSoftware: Boolean): MediaCodec {
    val defaultCodec = runCatching { MediaCodec.createDecoderByType(mime) }.getOrNull()
    val isGoldfish = defaultCodec?.name?.startsWith("c2.goldfish.", ignoreCase = true) == true
    if (!preferSoftware && defaultCodec != null && !isGoldfish) {
        return defaultCodec
    }
    runCatching { defaultCodec?.release() }
    val candidates = when (mime) {
        MediaFormat.MIMETYPE_VIDEO_AVC -> listOf("c2.android.avc.decoder", "OMX.google.h264.decoder")
        MediaFormat.MIMETYPE_VIDEO_HEVC -> listOf("c2.android.hevc.decoder", "OMX.google.hevc.decoder")
        "video/av01" -> listOf("c2.android.av1.decoder", "c2.android.av1-dav1d.decoder")
        else -> emptyList()
    }
    for (name in candidates) {
        val codec = runCatching { MediaCodec.createByCodecName(name) }.getOrNull()
        if (codec != null) {
            return codec
        }
    }
    return MediaCodec.createDecoderByType(mime)
}

