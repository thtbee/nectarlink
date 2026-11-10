// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.LocalNetwork
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.mirror.MirrorService
import app.nectarlink.android.recorder.RecorderService
import app.nectarlink.android.service.ConnectionService
import app.nectarlink.android.ui.deck.DeckScreen
import app.nectarlink.android.ui.guide.FeaturesGuideScreen
import app.nectarlink.android.ui.guide.GuideHighlightTarget
import app.nectarlink.android.ui.home.HomeScreen
import app.nectarlink.android.ui.pairing.PairingActions
import app.nectarlink.android.ui.pairing.PairingScreen
import app.nectarlink.android.ui.recorder.RecorderScreen
import app.nectarlink.android.ui.remote.RemoteMode
import app.nectarlink.android.ui.remote.RemoteScreen
import app.nectarlink.android.ui.settings.RelaySettingsDialog
import app.nectarlink.android.ui.settings.SettingsScreen
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.GlassBackdrop
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.NectarlinkTheme
import app.nectarlink.android.ui.theme.glassBackdropSource
import app.nectarlink.android.ui.theme.liquidGlass
import app.nectarlink.android.ui.theme.liquidGlassCard
import app.nectarlink.android.ui.walkthrough.WalkthroughScreen
import app.nectarlink.android.ui.walkthrough.WalkthroughStep
import app.nectarlink.android.ui.walkthrough.resolveStartupWalkthrough
import app.nectarlink.android.ui.webcam.WebcamScreen
import app.nectarlink.android.webcam.WebcamService
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    @Volatile
    private var activeRemotePcId: String? = null
    private var requestedRemotePc by mutableStateOf<String?>(null)
    private var requestedRemoteMode by mutableStateOf(RemoteMode.Touchpad)
    private var requestedDeckPc by mutableStateOf<String?>(null)
    private var requestedRecordPc by mutableStateOf<String?>(null)

    /**
     * A pairing link that arrived from outside the app (a web page, another app, the system
     * camera). Pairing waits for the user to confirm: any app or site can send such a link,
     * and pairing grants that PC this phone's notifications and messages.
     */
    private var pendingPairingLink by mutableStateOf<String?>(null)
    private var requestedWebcamPc by mutableStateOf<String?>(null)
    private var requestedWebcamAutoStart by mutableStateOf(false)
    private var requestedWebcamHeight by mutableStateOf<Int?>(null)
    private var requestedWebcamCamera by mutableStateOf<String?>(null)

    private val permissions =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            // The local network may have just been allowed: reconnect now.
            (application as NectarlinkApplication).core.refreshNotificationAccess()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val core = (application as NectarlinkApplication).core
        val preferences = Preferences(this)
        StarNotification.onAppOpened(this)

        // Notifications (transfers, calls, requests from PCs) and, on Android
        // 17, the local network, without which no PC can be reached.
        window.decorView.post {
            val wanted = buildList {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
                if (LocalNetwork.needed()) add(LocalNetwork.PERMISSION)
            }.filter { checkSelfPermission(it) != android.content.pm.PackageManager.PERMISSION_GRANTED }
            if (wanted.isNotEmpty()) permissions.launch(wanted.toTypedArray())
        }
        if (savedInstanceState != null) {
            pendingPairingLink = savedInstanceState.getString(STATE_PENDING_PAIRING_LINK)
        } else {
            handlePairingLink(intent)
            handleRemoteIntent(intent)
            handleDeckIntent(intent)
            handleRecordIntent(intent)
            handleWebcamIntent(intent)
        }

        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            val state by core.state.collectAsStateWithLifecycle()
            // Stay reachable in the background while a PC is paired.
            LaunchedEffect(state.devices.isNotEmpty(), state.status is CoreStatus.Ready) {
                if (state.devices.isNotEmpty()) {
                    ConnectionService.start(this@MainActivity)
                } else if (state.status is CoreStatus.Ready) {
                    ConnectionService.stop(this@MainActivity)
                }
            }
            LaunchedEffect(state.ringingPeer) {
                if (state.ringingPeer != null) startActivity(RingActivity.intent(this@MainActivity))
            }
            NectarlinkTheme(
                appearance = appearance,
                mediaSeedColor = state.activeMediaColor,
                backdrop = true,
            ) {
                // Every screen sits on the theme's background, not the
                // window's (which can't follow the app's own light/dark choice).
                Surface(color = MaterialTheme.colorScheme.background) {
                    App(
                        core = core,
                        state = state,
                        appearance = appearance,
                        preferences = preferences,
                        requestedRemotePc = requestedRemotePc,
                        requestedRemoteMode = requestedRemoteMode,
                        onRemoteConsumed = { requestedRemotePc = null },
                        requestedDeckPc = requestedDeckPc,
                        onDeckConsumed = { requestedDeckPc = null },
                        requestedRecordPc = requestedRecordPc,
                        onRecordConsumed = { requestedRecordPc = null },
                        requestedWebcamPc = requestedWebcamPc,
                        requestedWebcamAutoStart = requestedWebcamAutoStart,
                        requestedWebcamHeight = requestedWebcamHeight,
                        requestedWebcamCamera = requestedWebcamCamera,
                        onWebcamConsumed = {
                            requestedWebcamPc = null
                            requestedWebcamAutoStart = false
                            requestedWebcamHeight = null
                            requestedWebcamCamera = null
                        },
                        onActiveRemoteChanged = { activeRemotePcId = it },
                    )
                    pendingPairingLink?.let { link ->
                        val pcName = remember(link) {
                            android.net.Uri.parse(link).getQueryParameter("n")?.trim()?.takeIf { it.isNotEmpty() }
                        } ?: stringResource(R.string.pair_link_unknown_pc)
                        val clearPending = {
                            pendingPairingLink = null
                            if (intent?.action == Intent.ACTION_VIEW) {
                                setIntent(Intent(this@MainActivity, MainActivity::class.java))
                            }
                        }
                        androidx.compose.material3.AlertDialog(
                            onDismissRequest = clearPending,
                            title = { Text(stringResource(R.string.pair_link_title, pcName)) },
                            text = { Text(stringResource(R.string.pair_link_body)) },
                            confirmButton = {
                                androidx.compose.material3.TextButton(onClick = {
                                    clearPending()
                                    if (PairingLinks(this@MainActivity).consume(link)) {
                                        core.joinWhenReady(link)
                                    }
                                }) { Text(stringResource(R.string.pair_link_confirm)) }
                            },
                            dismissButton = {
                                androidx.compose.material3.TextButton(onClick = clearPending) {
                                    Text(stringResource(R.string.pair_link_cancel))
                                }
                            },
                        )
                    }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Back from Settings, maybe with notification access changed.
        val app = application as NectarlinkApplication
        if (app.core.state.value.status is CoreStatus.Ready) {
            app.core.refreshNotificationAccess()
        }
        if (app.updater.enabled) {
            // Back from allowing installs: the update the user started carries on.
            app.updater.onAppResumed()
            lifecycleScope.launch { app.updater.updateIfDue() }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString(STATE_PENDING_PAIRING_LINK, pendingPairingLink)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairingLink(intent)
        handleRemoteIntent(intent)
        handleDeckIntent(intent)
        handleRecordIntent(intent)
        handleWebcamIntent(intent)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        val pcId = activeRemotePcId
        if (pcId != null) {
            when (keyCode) {
                KeyEvent.KEYCODE_VOLUME_DOWN -> {
                    if (event?.repeatCount == 0) {
                        (application as NectarlinkApplication).core.remoteSlide(pcId, "next")
                    }
                    return true
                }
                KeyEvent.KEYCODE_VOLUME_UP -> {
                    if (event?.repeatCount == 0) {
                        (application as NectarlinkApplication).core.remoteSlide(pcId, "previous")
                    }
                    return true
                }
            }
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent?): Boolean {
        if (activeRemotePcId != null &&
            (keyCode == KeyEvent.KEYCODE_VOLUME_DOWN || keyCode == KeyEvent.KEYCODE_VOLUME_UP)
        ) {
            return true
        }
        return super.onKeyUp(keyCode, event)
    }

    private fun handleRemoteIntent(intent: Intent?) {
        val pc = intent?.getStringExtra("remote_pc") ?: return
        val mode = intent.getStringExtra("remote_mode")
        requestedRemoteMode = when {
            mode.equals("presentation", ignoreCase = true) -> RemoteMode.Presentation
            mode.equals("air", ignoreCase = true) ||
                mode.equals("air_mouse", ignoreCase = true) ||
                mode.equals("airmouse", ignoreCase = true) -> RemoteMode.AirMouse
            else -> RemoteMode.Touchpad
        }
        requestedRemotePc = pc
    }

    private fun handleDeckIntent(intent: Intent?) {
        val pc = intent?.getStringExtra("deck_pc") ?: return
        requestedDeckPc = pc
    }

    private fun handleRecordIntent(intent: Intent?) {
        val pc = intent?.getStringExtra("record_pc") ?: return
        requestedRecordPc = pc
    }

    private fun handleWebcamIntent(intent: Intent?) {
        val pc = intent?.getStringExtra("webcam_pc") ?: return
        // Auto-start only for our own request notification (see WebcamRequests).
        requestedWebcamAutoStart = intent.getBooleanExtra("webcam_auto_start", false) &&
            app.nectarlink.android.webcam.WebcamRequests.consumeAutoStartToken(intent.getStringExtra("webcam_token"))
        requestedWebcamHeight = intent.getIntExtra("webcam_height", 0).takeIf { it > 0 }
        requestedWebcamCamera = intent.getStringExtra("webcam_camera")
        requestedWebcamPc = pc
    }

    /**
     * A PC's pairing QR code opened from another app (e.g. the camera).
     * Each link holds a one-time secret, so a link is used once: Android
     * re-delivers the original intent when the task is restored or reopened
     * from Recents, and that must not start a stale pairing.
     */
    private fun handlePairingLink(intent: Intent?) {
        val link = intent?.takeIf { it.action == Intent.ACTION_VIEW }?.dataString ?: return
        if (!link.startsWith("nectarlink://pair?")) return
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        if (PairingLinks(this).isUsed(link)) return
        pendingPairingLink = link
    }

    private companion object {
        const val STATE_PENDING_PAIRING_LINK = "pending_pairing_link"
    }
}

private enum class Tab { Home, Settings }

private sealed interface ScreenRoute {
    data object Starting : ScreenRoute
    data class Failed(val message: String) : ScreenRoute
    data object Walkthrough : ScreenRoute
    data object FeaturesGuide : ScreenRoute
    data object Pairing : ScreenRoute
    data class Webcam(val pcId: String) : ScreenRoute
    data class Recorder(val pcId: String) : ScreenRoute
    data class Deck(val pcId: String) : ScreenRoute
    data class Desk(val pcId: String) : ScreenRoute
    data class Remote(val pcId: String) : ScreenRoute
    data object Main : ScreenRoute
}

@Composable
private fun App(
    core: Core,
    state: CoreState,
    appearance: Appearance,
    preferences: Preferences,
    requestedRemotePc: String?,
    requestedRemoteMode: RemoteMode,
    onRemoteConsumed: () -> Unit,
    requestedDeckPc: String?,
    onDeckConsumed: () -> Unit,
    requestedRecordPc: String?,
    onRecordConsumed: () -> Unit,
    requestedWebcamPc: String?,
    requestedWebcamAutoStart: Boolean,
    requestedWebcamHeight: Int?,
    requestedWebcamCamera: String?,
    onWebcamConsumed: () -> Unit,
    onActiveRemoteChanged: (String?) -> Unit,
) {
    var tab by rememberSaveable { mutableStateOf(Tab.Home) }
    var pairing by rememberSaveable { mutableStateOf(false) }
    var manualWalkthroughOpen by rememberSaveable { mutableStateOf(false) }
    var activeFirstRunWalkthrough by rememberSaveable { mutableStateOf(false) }
    var featuresGuideOpen by rememberSaveable { mutableStateOf(false) }
    var pendingHighlightTarget by rememberSaveable { mutableStateOf<GuideHighlightTarget?>(null) }
    var remotePcId by rememberSaveable { mutableStateOf<String?>(null) }
    var remoteInitialMode by rememberSaveable { mutableStateOf(RemoteMode.Touchpad) }
    var deckPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var deskPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var recordPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var webcamPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var webcamAutoStart by rememberSaveable { mutableStateOf(false) }
    var webcamInitialHeight by rememberSaveable { mutableStateOf<Int?>(null) }
    var webcamInitialCamera by rememberSaveable { mutableStateOf<String?>(null) }
    val walkthroughPersistence by preferences.walkthrough.collectAsStateWithLifecycle()
    val startupWalkthrough = remember(walkthroughPersistence, state.devices.isNotEmpty(), activeFirstRunWalkthrough) {
        resolveStartupWalkthrough(
            persistence = walkthroughPersistence,
            hasPairedDevices = state.devices.isNotEmpty(),
            walkthroughAlreadyActive = activeFirstRunWalkthrough,
        )
    }
    LaunchedEffect(state.status, startupWalkthrough.showWalkthrough, walkthroughPersistence.completed) {
        if (walkthroughPersistence.completed) {
            activeFirstRunWalkthrough = false
        } else if (state.status is CoreStatus.Ready && startupWalkthrough.showWalkthrough) {
            activeFirstRunWalkthrough = true
        }
    }
    LaunchedEffect(state.status, startupWalkthrough.markCompletedAutomatically) {
        if (state.status is CoreStatus.Ready && startupWalkthrough.markCompletedAutomatically) {
            preferences.updateWalkthrough { it.copy(completed = true) }
        }
    }
    val snackbar = remember { SnackbarHostState() }
    val undoLabel = stringResource(R.string.send_queue_undo)
    LaunchedEffect(Unit) { core.messages.collect { snackbar.showSnackbar(it) } }
    LaunchedEffect(undoLabel) {
        core.queuedNotices.collectLatest { notice ->
            val result = snackbar.showSnackbar(
                message = notice.message,
                actionLabel = undoLabel,
                duration = androidx.compose.material3.SnackbarDuration.Short,
            )
            if (result == androidx.compose.material3.SnackbarResult.ActionPerformed) {
                if (notice.queueId != null) {
                    core.cancelQueueItem(notice.queueId)
                } else if (notice.peerId != null) {
                    core.cancelAllQueueItems(notice.peerId)
                }
            }
        }
    }
    LaunchedEffect(Unit) {
        core.clipSuggestions.collectLatest { notice ->
            val result = snackbar.showSnackbar(
                message = notice.message,
                actionLabel = notice.actionLabel,
                duration = androidx.compose.material3.SnackbarDuration.Short,
            )
            if (result == androidx.compose.material3.SnackbarResult.ActionPerformed) {
                core.runClipSuggestion(notice.suggestion)
            }
        }
    }

    LaunchedEffect(requestedRemotePc, state.devices) {
        val req = requestedRemotePc ?: return@LaunchedEffect
        val target = if (req == "first") state.devices.firstOrNull()?.id else state.device(req)?.id
        if (target != null) {
            recordPcId = null
            deckPcId = null
            deskPcId = null
            webcamPcId = null
            remoteInitialMode = requestedRemoteMode
            remotePcId = target
            onRemoteConsumed()
        }
    }

    LaunchedEffect(requestedDeckPc, state.devices) {
        val req = requestedDeckPc ?: return@LaunchedEffect
        val target = if (req == "first") state.devices.firstOrNull()?.id else state.device(req)?.id
        if (target != null) {
            remotePcId = null
            recordPcId = null
            deskPcId = null
            webcamPcId = null
            deckPcId = target
            onDeckConsumed()
        }
    }

    LaunchedEffect(requestedRecordPc, state.devices) {
        val req = requestedRecordPc ?: return@LaunchedEffect
        val target = if (req == "first") state.devices.firstOrNull()?.id else state.device(req)?.id
        if (target != null) {
            remotePcId = null
            deckPcId = null
            deskPcId = null
            webcamPcId = null
            recordPcId = target
            onRecordConsumed()
        }
    }

    LaunchedEffect(requestedWebcamPc, state.devices) {
        val req = requestedWebcamPc ?: return@LaunchedEffect
        val target = if (req == "first") state.devices.firstOrNull()?.id else state.device(req)?.id
        if (target != null) {
            remotePcId = null
            deckPcId = null
            deskPcId = null
            recordPcId = null
            webcamAutoStart = requestedWebcamAutoStart
            webcamInitialHeight = requestedWebcamHeight
            webcamInitialCamera = requestedWebcamCamera
            webcamPcId = target
            onWebcamConsumed()
        }
    }

    val remoteDevice = remotePcId?.let { state.device(it) }
    val deckDevice = deckPcId?.let { state.device(it) }
    val deskDevice = deskPcId?.let { state.device(it) }
    val recordDevice = recordPcId?.let { state.device(it) }
    val webcamDevice = webcamPcId?.let { state.device(it) }
    LaunchedEffect(remoteDevice?.id) {
        onActiveRemoteChanged(remoteDevice?.id)
    }

    val actions = remember(core) {
        object : PairingActions {
            override fun visible(open: Boolean) = core.setPairingScreenOpen(open)
            override fun join(link: String) = core.joinWithLink(link)
            override fun pairNearby(id: String) = core.pairNearby(id)
            override fun confirm(matches: Boolean) = core.confirmCode(matches)
            override fun reset() = core.resetPairing()
            override suspend fun showCode(): String? = core.pairingLink()
        }
    }

    val showWalkthrough = manualWalkthroughOpen ||
        (state.status is CoreStatus.Ready && startupWalkthrough.showWalkthrough)

    val currentRoute: ScreenRoute = when {
        state.status is CoreStatus.Starting -> ScreenRoute.Starting
        state.status is CoreStatus.Failed -> ScreenRoute.Failed(state.status.message)
        showWalkthrough -> ScreenRoute.Walkthrough
        featuresGuideOpen -> ScreenRoute.FeaturesGuide
        pairing || state.pairing != PairingState.Idle -> ScreenRoute.Pairing
        webcamDevice != null -> ScreenRoute.Webcam(webcamDevice.id)
        recordDevice != null -> ScreenRoute.Recorder(recordDevice.id)
        deckDevice != null -> ScreenRoute.Deck(deckDevice.id)
        deskDevice != null -> ScreenRoute.Desk(deskDevice.id)
        remoteDevice != null -> ScreenRoute.Remote(remoteDevice.id)
        else -> ScreenRoute.Main
    }

    val reducedMotion = LocalReducedMotion.current
    AnimatedContent(
        targetState = currentRoute,
        transitionSpec = {
            if (reducedMotion || initialState is ScreenRoute.Starting) {
                EnterTransition.None togetherWith ExitTransition.None
            } else {
                val enteringSubScreen = targetState !is ScreenRoute.Main && initialState is ScreenRoute.Main
                val returningToMain = targetState is ScreenRoute.Main && initialState !is ScreenRoute.Main
                val spatialSpring = androidx.compose.animation.core.spring<androidx.compose.ui.unit.IntOffset>(
                    dampingRatio = androidx.compose.animation.core.Spring.DampingRatioNoBouncy,
                    stiffness = androidx.compose.animation.core.Spring.StiffnessMediumLow,
                )
                val scaleSpring = androidx.compose.animation.core.spring<Float>(
                    dampingRatio = androidx.compose.animation.core.Spring.DampingRatioNoBouncy,
                    stiffness = androidx.compose.animation.core.Spring.StiffnessMediumLow,
                )
                when {
                    enteringSubScreen -> (
                        fadeIn(tween(210, easing = FastOutSlowInEasing)) +
                            slideInHorizontally(animationSpec = spatialSpring) { (it * 0.08f).toInt() } +
                            scaleIn(initialScale = 0.95f, animationSpec = scaleSpring)
                        ) togetherWith (
                        fadeOut(tween(150)) +
                            slideOutHorizontally(animationSpec = spatialSpring) { -(it * 0.04f).toInt() } +
                            scaleOut(targetScale = 0.97f, animationSpec = scaleSpring)
                        )
                    returningToMain -> (
                        fadeIn(tween(200, easing = FastOutSlowInEasing)) +
                            slideInHorizontally(animationSpec = spatialSpring) { -(it * 0.04f).toInt() } +
                            scaleIn(initialScale = 0.97f, animationSpec = scaleSpring)
                        ) togetherWith (
                        fadeOut(tween(150)) +
                            slideOutHorizontally(animationSpec = spatialSpring) { (it * 0.08f).toInt() } +
                            scaleOut(targetScale = 0.95f, animationSpec = scaleSpring)
                        )
                    else -> fadeIn(tween(200)) togetherWith fadeOut(tween(150))
                }
            }
        },
        label = "screen_route",
    ) { route ->
        when (route) {
            ScreenRoute.Starting -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (reducedMotion) {
                    CircularProgressIndicator(progress = { 0.75f })
                } else {
                    CircularProgressIndicator()
                }
            }
            is ScreenRoute.Failed -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(stringResource(R.string.core_failed, route.message))
            }
            ScreenRoute.Walkthrough -> {
                BackHandler(
                    enabled = manualWalkthroughOpen && walkthroughPersistence.currentStep.previous == null,
                ) {
                    manualWalkthroughOpen = false
                }
                Scaffold(
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    WalkthroughScreen(
                        state = state,
                        step = walkthroughPersistence.currentStep,
                        pairingActions = actions,
                        onStepChange = { nextStep ->
                            preferences.updateWalkthrough { it.goToStep(nextStep) }
                        },
                        onSkip = {
                            core.resetPairing()
                            pairing = false
                            manualWalkthroughOpen = false
                            activeFirstRunWalkthrough = false
                            preferences.updateWalkthrough { it.copy(completed = true) }
                        },
                        onFinishToHome = {
                            core.resetPairing()
                            pairing = false
                            manualWalkthroughOpen = false
                            activeFirstRunWalkthrough = false
                            tab = Tab.Home
                            preferences.updateWalkthrough {
                                it.copy(completed = true, currentStepIndex = WalkthroughStep.Done.index)
                            }
                        },
                        onOpenFeaturesGuide = {
                            core.resetPairing()
                            pairing = false
                            manualWalkthroughOpen = false
                            activeFirstRunWalkthrough = false
                            featuresGuideOpen = true
                            preferences.updateWalkthrough {
                                it.copy(completed = true, currentStepIndex = WalkthroughStep.Done.index)
                            }
                        },
                        onAccessChanged = core::refreshNotificationAccess,
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            ScreenRoute.FeaturesGuide -> {
                BackHandler { featuresGuideOpen = false }
                Scaffold(
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    FeaturesGuideScreen(
                        state = state,
                        onBack = { featuresGuideOpen = false },
                        onShowMe = { target ->
                            core.resetPairing()
                            pairing = false
                            featuresGuideOpen = false
                            pendingHighlightTarget = target
                            tab = when (target) {
                                GuideHighlightTarget.HOME_PAIR_PC,
                                GuideHighlightTarget.HOME_PC_CARD,
                                GuideHighlightTarget.HOME_PC_AUDIO,
                                GuideHighlightTarget.HOME_CLIPBOARD_HISTORY,
                                GuideHighlightTarget.HOME_TIMELINE,
                                GuideHighlightTarget.HOME_SEND_QUEUE,
                                GuideHighlightTarget.HOME_SETUP_PERMISSIONS,
                                GuideHighlightTarget.HOME_LOCALSEND -> Tab.Home
                                GuideHighlightTarget.SETTINGS_DEVICES,
                                GuideHighlightTarget.SETTINGS_NOTIFICATIONS,
                                GuideHighlightTarget.SETTINGS_CONTROL,
                                GuideHighlightTarget.SETTINGS_BACKGROUND,
                                GuideHighlightTarget.SETTINGS_CALLS_CONTACTS,
                                GuideHighlightTarget.SETTINGS_PHOTOS_STORAGE,
                                GuideHighlightTarget.SETTINGS_CLIPBOARD,
                                GuideHighlightTarget.SETTINGS_LOCALSEND,
                                GuideHighlightTarget.SETTINGS_DATA_STORAGE -> Tab.Settings
                            }
                        },
                        onOpenWalkthrough = {
                            core.resetPairing()
                            pairing = false
                            featuresGuideOpen = false
                            preferences.updateWalkthrough { it.goToStep(WalkthroughStep.Welcome) }
                            manualWalkthroughOpen = true
                        },
                        onPairNew = {
                            featuresGuideOpen = false
                            pairing = true
                        },
                        onAccessChanged = core::refreshNotificationAccess,
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            ScreenRoute.Pairing -> {
                BackHandler(enabled = pairing || state.devices.isNotEmpty()) { core.resetPairing(); pairing = false }
                PairingScreen(state, actions, cancellable = pairing || state.devices.isNotEmpty()) { pairing = false }
            }
            is ScreenRoute.Webcam -> {
                val dev = state.device(route.pcId) ?: return@AnimatedContent
                BackHandler { webcamPcId = null }
                Scaffold(
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    WebcamScreen(
                        device = dev,
                        autoStart = webcamAutoStart,
                        initialHeight = webcamInitialHeight,
                        initialCamera = webcamInitialCamera,
                        onAutoStartConsumed = {
                            webcamAutoStart = false
                            webcamInitialHeight = null
                            webcamInitialCamera = null
                        },
                        onAccessChanged = core::refreshNotificationAccess,
                        onBack = { webcamPcId = null },
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            is ScreenRoute.Recorder -> {
                val dev = state.device(route.pcId) ?: return@AnimatedContent
                BackHandler { recordPcId = null }
                Scaffold(
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    RecorderScreen(
                        device = dev,
                        state = state,
                        core = core,
                        onBack = { recordPcId = null },
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            is ScreenRoute.Deck -> {
                val dev = state.device(route.pcId) ?: return@AnimatedContent
                BackHandler { deckPcId = null }
                Scaffold(
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    DeckScreen(
                        device = dev,
                        core = core,
                        onBack = { deckPcId = null },
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            is ScreenRoute.Desk -> {
                val dev = state.device(route.pcId) ?: return@AnimatedContent
                BackHandler { deskPcId = null }
                app.nectarlink.android.ui.desk.DeskScreen(
                    device = dev,
                    core = core,
                    onBack = { deskPcId = null },
                    onOpenRemote = { mode ->
                        val id = dev.id
                        deskPcId = null
                        remoteInitialMode = mode
                        remotePcId = id
                    },
                    onOpenDeck = {
                        val id = dev.id
                        deskPcId = null
                        deckPcId = id
                    },
                    modifier = Modifier.fillMaxSize(),
                )
            }
            is ScreenRoute.Remote -> {
                val dev = state.device(route.pcId) ?: return@AnimatedContent
                BackHandler { remotePcId = null }
                val sensitivity by preferences.touchpadSensitivity.collectAsStateWithLifecycle()
                Scaffold(
                    contentWindowInsets = androidx.compose.foundation.layout.WindowInsets(0, 0, 0, 0),
                    snackbarHost = { SnackbarHost(snackbar) },
                ) { padding ->
                    RemoteScreen(
                        device = dev,
                        core = core,
                        sensitivity = sensitivity,
                        initialMode = remoteInitialMode,
                        onBack = { remotePcId = null },
                        modifier = Modifier.fillMaxSize().padding(padding),
                    )
                }
            }
            ScreenRoute.Main -> Box(Modifier.fillMaxSize()) {
                val isGlass = appearance.theme == "glass"
                val glassBackdrop = app.nectarlink.android.ui.theme.rememberGlassBackdrop()
                val onlineCount by remember(state.devices) {
                    androidx.compose.runtime.derivedStateOf { state.devices.count { it.online } }
                }
                val pairedCount = state.devices.size
                Scaffold(
                    containerColor = androidx.compose.ui.graphics.Color.Transparent,
                    snackbarHost = { SnackbarHost(snackbar) },
                    bottomBar = {
                        FloatingBottomPillDock(
                            selectedTab = tab,
                            onSelectTab = { tab = it },
                            onlineCount = onlineCount,
                            pairedCount = pairedCount,
                            isGlass = isGlass,
                            reducedMotion = reducedMotion,
                            backdrop = glassBackdrop,
                        )
                    },
                ) { padding ->
                    val context = LocalContext.current
                    val mirrorSession by MirrorService.session.collectAsStateWithLifecycle()
                    val webcamSession by WebcamService.session.collectAsStateWithLifecycle()
                    val micStreamingTo by app.nectarlink.android.mic.MicService.streaming.collectAsStateWithLifecycle()
                    val recorderFlow = remember {
                        RecorderService.session.distinctUntilChanged { a, b ->
                            a.active == b.active &&
                                a.paused == b.paused &&
                                a.pcId == b.pcId &&
                                (a.elapsedMs / 1000L) == (b.elapsedMs / 1000L)
                        }
                    }
                    val recorderInitial = remember { RecorderService.currentSession() }
                    val recorderSession by recorderFlow.collectAsStateWithLifecycle(recorderInitial)
                    val touchpadSensitivity by preferences.touchpadSensitivity.collectAsStateWithLifecycle()
                    val suggestClipboardActions by preferences.suggestClipboardActions.collectAsStateWithLifecycle()
                    val volumeKeysControlPc by preferences.volumeKeysControlPc.collectAsStateWithLifecycle()
                    val backgroundServiceEnabled by preferences.backgroundServiceEnabled.collectAsStateWithLifecycle()
                    val startWithoutAsking by app.nectarlink.android.service.PcAutoStart.pcs(context).collectAsStateWithLifecycle()
                    val hiveInviteShown by preferences.hiveInviteShown.collectAsStateWithLifecycle()
                    val saveableStateHolder = androidx.compose.runtime.saveable.rememberSaveableStateHolder()

                    // Only apply top scaffold padding here so LazyColumns in HomeScreen/SettingsScreen
                    // (which have bottom = 104.dp contentPadding) glide smoothly behind the floating glass dock.
                    // Tabs slide toward the side of the tab you picked, on a soft spring.
                    AnimatedContent(
                        targetState = tab,
                        transitionSpec = {
                            if (reducedMotion) {
                                EnterTransition.None togetherWith ExitTransition.None
                            } else {
                                val forward = targetState.ordinal > initialState.ordinal
                                val slide = androidx.compose.animation.core.spring<androidx.compose.ui.unit.IntOffset>(
                                    dampingRatio = 0.82f,
                                    stiffness = androidx.compose.animation.core.Spring.StiffnessMediumLow,
                                )
                                (
                                    slideInHorizontally(slide) { w -> if (forward) w / 6 else -w / 6 } +
                                        fadeIn(tween(220, easing = FastOutSlowInEasing))
                                    ) togetherWith (
                                    slideOutHorizontally(slide) { w -> if (forward) -w / 10 else w / 10 } +
                                        fadeOut(tween(120))
                                    )
                            }
                        },
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(top = padding.calculateTopPadding())
                            .glassBackdropSource(glassBackdrop, enabled = isGlass),
                        label = "main_tab",
                    ) { shownTab ->
                        when (shownTab) {
                            Tab.Home -> saveableStateHolder.SaveableStateProvider("tab_home") {
                                HomeScreen(
                                    state = state,
                                    onRing = core::ring,
                                    onStopRinging = core::stopRinging,
                                    onPairNew = { pairing = true },
                                    onRefresh = core::refresh,
                                    onPower = core::pcPower,
                                    onWake = core::wake,
                                    onSetReconnect = core::setReconnect,
                                    onRemote = { id ->
                                        remoteInitialMode = RemoteMode.Touchpad
                                        remotePcId = id
                                    },
                                    onDeck = { id ->
                                        deckPcId = id
                                    },
                                    onDesk = { id ->
                                        deskPcId = id
                                    },
                                    onRecord = { id ->
                                        recordPcId = id
                                    },
                                    onWebcam = { id ->
                                        webcamAutoStart = false
                                        webcamPcId = id
                                    },
                                    updater = (context.applicationContext as NectarlinkApplication).updater,
                                    onSendFiles = core::sendFiles,
                                    onSendFolder = core::sendFolder,
                                    onAccessChanged = core::refreshNotificationAccess,
                                    onCancelTransfer = core::cancelTransfer,
                                    onPauseTransfer = core::pauseTransfer,
                                    onResumeTransfer = core::resumeTransfer,
                                    onAcceptTransfer = core::acceptTransfer,
                                    onCancelQueueItem = core::cancelQueueItem,
                                    onCancelAllQueueItems = { core.cancelAllQueueItems(null) },
                                    onRetryQueueItem = core::retryQueueItem,
                                    onClearFinishedQueueItems = { core.clearFinishedQueueItems(null) },
                                    onRefreshLocalSend = core::refreshLocalSend,
                                    onAllowStorage = { pcId -> core.setStorageEnabled(pcId, true) },
                                    onDismissStorageRequest = core::dismissStorageRequest,
                                    onAcceptWebcamRequest = { req ->
                                        core.clearWebcamRequest(req.pcId)
                                        webcamInitialHeight = req.height
                                        webcamInitialCamera = req.camera
                                        webcamAutoStart = true
                                        webcamPcId = req.pcId
                                    },
                                    onDismissWebcamRequest = { core.clearWebcamRequest() },
                                    onCopyClipboardHistory = core::copyClipboardHistory,
                                    onPinClipboardHistory = core::pinClipboardHistory,
                                    onDeleteClipboardHistory = core::deleteClipboardHistory,
                                    onClearClipboardHistory = core::clearClipboardHistory,
                                    loadClipboardHistoryImage = core::clipboardHistoryImage,
                                    queryTimelinePage = { kind, dev, search, offset ->
                                        core.queryTimelinePage(kind, dev, search, offset)
                                    },
                                    onDeleteTimelineEntry = core::deleteTimelineEntry,
                                    onClearTimeline = core::clearTimeline,
                                    onResendTimelineEntry = core::resendTimelineEntry,
                                    onSetPcAudio = core::setPcAudio,
                                    onStopKeyboardFromPc = core::stopKeyboardFromPc,
                                    onListPcFiles = core::listPcFiles,
                                    onDownloadPcFile = core::downloadPcFile,
                                    mirrorSession = mirrorSession,
                                    webcamSession = webcamSession,
                                    recorderSession = recorderSession,
                                    micStreamingTo = micStreamingTo,
                                    onDeclineMicRequest = core::declineMicRequest,
                                    onStartMic = { pcId ->
                                        // Asks for the microphone permission if needed, then starts.
                                        context.startActivity(
                                            android.content.Intent(context, app.nectarlink.android.mic.MicStartActivity::class.java)
                                                .putExtra(app.nectarlink.android.mic.MicStartActivity.EXTRA_PC, pcId),
                                        )
                                    },
                                    onStopMic = { app.nectarlink.android.mic.MicService.stopFromUser(context) },
                                    onStopMirror = { MirrorService.stopCurrent() },
                                    onOpenWebcam = { pcId ->
                                        webcamAutoStart = false
                                        webcamPcId = pcId
                                    },
                                    onStopWebcam = { WebcamService.stop(context) },
                                    onOpenRecorder = { pcId -> recordPcId = pcId },
                                    onStopRecorder = { RecorderService.stop(context) },
                                    showWhatsNewGuideBanner = startupWalkthrough.showWhatsNewGuideBanner,
                                    showHiveInvite = hiveInviteShown && appearance.theme != "hive",
                                    onTryHive = {
                                        preferences.answerHiveInvite()
                                        preferences.update { it.copy(theme = "hive") }
                                    },
                                    onDismissHiveInvite = preferences::answerHiveInvite,
                                    backgroundServiceEnabled = backgroundServiceEnabled,
                                    onTurnOnBackgroundService = {
                                        preferences.updateBackgroundServiceEnabled(true)
                                        core.start()
                                        ConnectionService.start(context)
                                    },
                                    onDismissWhatsNewGuide = {
                                        preferences.updateWalkthrough { it.dismissWhatsNewGuide() }
                                    },
                                    onOpenFeaturesGuide = {
                                        preferences.updateWalkthrough { it.dismissWhatsNewGuide() }
                                        featuresGuideOpen = true
                                    },
                                    onOpenWalkthrough = {
                                        preferences.updateWalkthrough { it.goToStep(WalkthroughStep.Welcome) }
                                        manualWalkthroughOpen = true
                                    },
                                    highlightTarget = pendingHighlightTarget,
                                    onHighlightConsumed = { pendingHighlightTarget = null },
                                    modifier = Modifier.fillMaxSize(),
                                )
                            }
                            Tab.Settings -> saveableStateHolder.SaveableStateProvider("tab_settings") {
                                var relaySettingsOpen by remember { mutableStateOf(false) }
                                if (relaySettingsOpen) {
                                    RelaySettingsDialog(
                                        core = core,
                                        awayMode = state.awayMode,
                                        onDismiss = { relaySettingsOpen = false },
                                    )
                                }
                                SettingsScreen(
                                    state = state,
                                    appearance = appearance,
                                    onAppearance = preferences::update,
                                    touchpadSensitivity = touchpadSensitivity,
                                    onTouchpadSensitivity = preferences::updateTouchpadSensitivity,
                                    onUnpair = core::unpair,
                                    onPairNew = { pairing = true },
                                    onAccessChanged = core::refreshNotificationAccess,
                                    updater = (context.applicationContext as NectarlinkApplication).updater,
                                    onSetStorageEnabled = core::setStorageEnabled,
                                    onSetMicEnabled = core::setMicEnabled,
                                    startWithoutAsking = startWithoutAsking,
                                    onSetStartWithoutAsking = { pc, on -> app.nectarlink.android.service.PcAutoStart.set(context, pc, on) },
                                    onAddSafFolder = core::addSafFolder,
                                    onRemoveSafFolder = core::removeSafFolder,
                                    onSetClipboardHistoryEnabled = core::setClipboardHistoryEnabled,
                                    onClearClipboardHistory = core::clearClipboardHistory,
                                    suggestClipboardActions = suggestClipboardActions,
                                    onSetSuggestClipboardActions = preferences::updateSuggestClipboardActions,
                                    volumeKeysControlPc = volumeKeysControlPc,
                                    onSetVolumeKeysControlPc = preferences::updateVolumeKeysControlPc,
                                    backgroundServiceEnabled = backgroundServiceEnabled,
                                    onToggleBackgroundService = { enabled ->
                                        preferences.updateBackgroundServiceEnabled(enabled)
                                        if (enabled) {
                                            core.start()
                                            ConnectionService.start(context)
                                        } else {
                                            ConnectionService.stop(context)
                                        }
                                    },
                                    onSetTimelineRetentionDays = core::setTimelineRetentionDays,
                                    onClearTimeline = core::clearTimeline,
                                    onClearMessageCache = core::clearMessageCache,
                                    onClearPhotoCache = core::clearPhotoThumbnailsCache,
                                    onClearReceivedFileHistory = core::clearReceivedFileHistory,
                                    onClearEverything = core::clearEverything,
                                    onToggleLocalSend = core::setLocalSendEnabled,
                                    onToggleAwayMode = core::setAwayMode,
                                    onOpenRelaySettings = { relaySettingsOpen = true },
                                    onOpenWalkthrough = {
                                        preferences.updateWalkthrough { it.goToStep(WalkthroughStep.Welcome) }
                                        manualWalkthroughOpen = true
                                    },
                                    onOpenFeaturesGuide = {
                                        featuresGuideOpen = true
                                    },
                                    highlightTarget = pendingHighlightTarget,
                                    onHighlightConsumed = { pendingHighlightTarget = null },
                                    modifier = Modifier.fillMaxSize(),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun FloatingBottomPillDock(
    selectedTab: Tab,
    onSelectTab: (Tab) -> Unit,
    onlineCount: Int,
    pairedCount: Int,
    isGlass: Boolean,
    reducedMotion: Boolean,
    backdrop: GlassBackdrop? = null,
) {
    val scheme = MaterialTheme.colorScheme
    val dark = scheme.surface.luminance() < 0.5f
    val baseFill = if (isGlass) {
        scheme.surfaceContainerHighest.copy(alpha = 0.92f)
    } else {
        // Solid: the list scrolls under it, and text showed through.
        scheme.surfaceContainerHigh
    }
    val rimColor = if (isGlass) scheme.outlineVariant else scheme.outlineVariant.copy(alpha = 0.45f)
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .windowInsetsPadding(androidx.compose.foundation.layout.WindowInsets.navigationBars)
            .padding(horizontal = 24.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        androidx.compose.foundation.layout.Row(
            modifier = Modifier
                .selectableGroup()
                .then(
                    if (isGlass) {
                        // Content scrolling beneath bends at the rim (Android 13+); a light
                        // tint keeps the labels readable over anything.
                        Modifier.liquidGlass(
                            backdrop = backdrop,
                            enabled = true,
                            cornerRadius = 30.dp,
                            tint = scheme.surface.copy(alpha = if (dark) 0.42f else 0.50f),
                            fallbackColor = baseFill,
                            rimColor = androidx.compose.ui.graphics.Color.White,
                            dark = dark,
                        )
                    } else {
                        Modifier.liquidGlassCard(
                            cornerRadius = 30.dp,
                            isGlass = false,
                            baseColor = baseFill,
                            tintColor = scheme.primary,
                            outlineColor = rimColor,
                        )
                    },
                )
                .padding(horizontal = 6.dp, vertical = 5.dp),
            horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FloatingDockTabPill(
                selected = selectedTab == Tab.Home,
                onClick = { onSelectTab(Tab.Home) },
                iconRes = R.drawable.ic_home,
                label = stringResource(R.string.home_title),
                statusDotColor = when {
                    onlineCount > 0 -> scheme.primary
                    pairedCount > 0 -> scheme.outline
                    else -> null
                },
                reducedMotion = reducedMotion,
            )
            FloatingDockTabPill(
                selected = selectedTab == Tab.Settings,
                onClick = { onSelectTab(Tab.Settings) },
                iconRes = R.drawable.ic_settings,
                label = stringResource(R.string.settings_title),
                statusDotColor = null,
                reducedMotion = reducedMotion,
            )
        }
    }
}

@Composable
private fun FloatingDockTabPill(
    selected: Boolean,
    onClick: () -> Unit,
    iconRes: Int,
    label: String,
    statusDotColor: androidx.compose.ui.graphics.Color?,
    reducedMotion: Boolean,
) {
    val scheme = MaterialTheme.colorScheme
    val scaleState = androidx.compose.animation.core.animateFloatAsState(
        targetValue = if (selected) 1f else 0.95f,
        animationSpec = if (reducedMotion) {
            androidx.compose.animation.core.snap()
        } else {
            androidx.compose.animation.core.spring(
                dampingRatio = 0.72f,
                stiffness = androidx.compose.animation.core.Spring.StiffnessMedium,
                visibilityThreshold = 0.005f,
            )
        },
        visibilityThreshold = 0.005f,
        label = "pill_scale",
    )
    val bgColorState = androidx.compose.animation.animateColorAsState(
        targetValue = if (selected) scheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent,
        animationSpec = if (reducedMotion) {
            androidx.compose.animation.core.snap()
        } else {
            tween(200, easing = FastOutSlowInEasing)
        },
        label = "pill_bg",
    )
    val contentColor = if (selected) scheme.onPrimaryContainer else scheme.onSurfaceVariant
    androidx.compose.foundation.layout.Row(
        modifier = Modifier
            .semantics {
                role = Role.Tab
                this.selected = selected
            }
            .graphicsLayer {
                val s = scaleState.value
                scaleX = s
                scaleY = s
            }
            .drawWithCache {
                val r = 24.dp.toPx()
                val cr = androidx.compose.ui.geometry.CornerRadius(r, r)
                onDrawBehind {
                    val bg = bgColorState.value
                    if (bg.alpha > 0f) {
                        drawRoundRect(color = bg, cornerRadius = cr)
                    }
                }
            }
            .clickable(
                interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() },
                indication = null,
                onClick = onClick,
            )
            .padding(horizontal = 18.dp, vertical = 9.dp),
        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            painter = painterResource(iconRes),
            contentDescription = null,
            tint = contentColor,
            modifier = Modifier.size(18.dp),
        )
        Text(
            text = label,
            color = contentColor,
            style = MaterialTheme.typography.labelLarge,
        )
        if (statusDotColor != null) {
            androidx.compose.foundation.layout.Spacer(
                modifier = Modifier
                    .size(6.dp)
                    .drawBehind {
                        drawCircle(color = statusDotColor)
                    },
            )
        }
    }
}

