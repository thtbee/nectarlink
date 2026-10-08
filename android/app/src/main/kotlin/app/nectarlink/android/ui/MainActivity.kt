// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.service.ConnectionService
import app.nectarlink.android.ui.home.HomeScreen
import app.nectarlink.android.ui.pairing.PairingActions
import android.view.KeyEvent
import app.nectarlink.android.core.LocalNetwork
import app.nectarlink.android.ui.deck.DeckScreen
import app.nectarlink.android.ui.pairing.PairingScreen
import app.nectarlink.android.ui.recorder.RecorderScreen
import app.nectarlink.android.ui.remote.RemoteMode
import app.nectarlink.android.ui.remote.RemoteScreen
import app.nectarlink.android.ui.settings.SettingsScreen
import app.nectarlink.android.ui.theme.NectarlinkTheme
import app.nectarlink.android.ui.webcam.WebcamScreen

class MainActivity : ComponentActivity() {
    @Volatile
    private var activeRemotePcId: String? = null
    private var requestedRemotePc by mutableStateOf<String?>(null)
    private var requestedRemoteMode by mutableStateOf(RemoteMode.Touchpad)
    private var requestedDeckPc by mutableStateOf<String?>(null)
    private var requestedRecordPc by mutableStateOf<String?>(null)
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

        // Notifications (transfers, calls, requests from PCs) and, on Android
        // 17, the local network, without which no PC can be reached.
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
            if (LocalNetwork.needed()) add(LocalNetwork.PERMISSION)
        }.filter { checkSelfPermission(it) != android.content.pm.PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) permissions.launch(wanted.toTypedArray())
        if (savedInstanceState == null) {
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
            NectarlinkTheme(appearance) {
                // Every screen sits on the theme's background, not the
                // window's (which can't follow the app's own light/dark choice).
                Surface(color = MaterialTheme.colorScheme.background) {
                    App(
                        core = core,
                        state = state,
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
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Back from Settings, maybe with notification access changed.
        val app = application as NectarlinkApplication
        app.core.refreshNotificationAccess()
        if (app.updater.enabled) lifecycleScope.launch { app.updater.checkIfDue() }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
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
        requestedWebcamAutoStart = intent.getBooleanExtra("webcam_auto_start", false)
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
        if (!PairingLinks(this).consume(link)) return
        (application as NectarlinkApplication).core.joinWhenReady(link)
    }
}

private enum class Tab { Home, Settings }

@Composable
private fun App(
    core: Core,
    state: CoreState,
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
    var remotePcId by rememberSaveable { mutableStateOf<String?>(null) }
    var remoteInitialMode by rememberSaveable { mutableStateOf(RemoteMode.Touchpad) }
    var deckPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var recordPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var webcamPcId by rememberSaveable { mutableStateOf<String?>(null) }
    var webcamAutoStart by rememberSaveable { mutableStateOf(false) }
    var webcamInitialHeight by rememberSaveable { mutableStateOf<Int?>(null) }
    var webcamInitialCamera by rememberSaveable { mutableStateOf<String?>(null) }
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { core.messages.collect { snackbar.showSnackbar(it) } }

    LaunchedEffect(requestedRemotePc, state.devices) {
        val req = requestedRemotePc ?: return@LaunchedEffect
        val target = if (req == "first") state.devices.firstOrNull()?.id else state.device(req)?.id
        if (target != null) {
            recordPcId = null
            deckPcId = null
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
        }
    }

    when {
        state.status is CoreStatus.Starting -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            CircularProgressIndicator()
        }
        state.status is CoreStatus.Failed -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text(stringResource(R.string.core_failed, state.status.message))
        }
        state.devices.isEmpty() || pairing || state.pairing != PairingState.Idle -> {
            BackHandler(enabled = pairing && state.devices.isNotEmpty()) { core.resetPairing(); pairing = false }
            PairingScreen(state, actions, cancellable = state.devices.isNotEmpty()) { pairing = false }
        }
        webcamDevice != null -> {
            BackHandler { webcamPcId = null }
            Scaffold(
                snackbarHost = { SnackbarHost(snackbar) },
            ) { padding ->
                WebcamScreen(
                    device = webcamDevice,
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
        recordDevice != null -> {
            BackHandler { recordPcId = null }
            Scaffold(
                snackbarHost = { SnackbarHost(snackbar) },
            ) { padding ->
                RecorderScreen(
                    device = recordDevice,
                    state = state,
                    core = core,
                    onBack = { recordPcId = null },
                    modifier = Modifier.fillMaxSize().padding(padding),
                )
            }
        }
        deckDevice != null -> {
            BackHandler { deckPcId = null }
            Scaffold(
                snackbarHost = { SnackbarHost(snackbar) },
            ) { padding ->
                DeckScreen(
                    device = deckDevice,
                    core = core,
                    onBack = { deckPcId = null },
                    modifier = Modifier.fillMaxSize().padding(padding),
                )
            }
        }
        remoteDevice != null -> {
            BackHandler { remotePcId = null }
            val sensitivity by preferences.touchpadSensitivity.collectAsStateWithLifecycle()
            Scaffold(
                snackbarHost = { SnackbarHost(snackbar) },
            ) { padding ->
                RemoteScreen(
                    device = remoteDevice,
                    core = core,
                    sensitivity = sensitivity,
                    initialMode = remoteInitialMode,
                    onBack = { remotePcId = null },
                    modifier = Modifier.fillMaxSize().padding(padding),
                )
            }
        }
        else -> Scaffold(
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                NavigationBar {
                    NavigationBarItem(
                        selected = tab == Tab.Home,
                        onClick = { tab = Tab.Home },
                        icon = { Icon(painterResource(R.drawable.ic_home), contentDescription = null) },
                        label = { Text(stringResource(R.string.home_title)) },
                    )
                    NavigationBarItem(
                        selected = tab == Tab.Settings,
                        onClick = { tab = Tab.Settings },
                        icon = { Icon(painterResource(R.drawable.ic_settings), contentDescription = null) },
                        label = { Text(stringResource(R.string.settings_title)) },
                    )
                }
            },
        ) { padding ->
            val modifier = Modifier.fillMaxSize().padding(padding)
            when (tab) {
                Tab.Home -> HomeScreen(
                    state,
                    core::ring,
                    core::stopRinging,
                    onPairNew = { pairing = true },
                    onRefresh = core::refresh,
                    onPower = core::pcPower,
                    onWake = core::wake,
                    onRemote = { id ->
                        remoteInitialMode = RemoteMode.Touchpad
                        remotePcId = id
                    },
                    onDeck = { id ->
                        deckPcId = id
                    },
                    onRecord = { id ->
                        recordPcId = id
                    },
                    onWebcam = { id ->
                        webcamAutoStart = false
                        webcamPcId = id
                    },
                    updater = (LocalContext.current.applicationContext as NectarlinkApplication).updater,
                    onSendFiles = core::sendFiles,
                    onSendFolder = core::sendFolder,
                    onAccessChanged = core::refreshNotificationAccess,
                    onCancelTransfer = core::cancelTransfer,
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
                    modifier = modifier,
                )
                Tab.Settings -> SettingsScreen(
                    state = state,
                    appearance = preferences.appearance.collectAsStateWithLifecycle().value,
                    onAppearance = preferences::update,
                    touchpadSensitivity = preferences.touchpadSensitivity.collectAsStateWithLifecycle().value,
                    onTouchpadSensitivity = preferences::updateTouchpadSensitivity,
                    onUnpair = core::unpair,
                    onPairNew = { pairing = true },
                    onAccessChanged = core::refreshNotificationAccess,
                    updater = (LocalContext.current.applicationContext as NectarlinkApplication).updater,
                    onSetStorageEnabled = core::setStorageEnabled,
                    onAddSafFolder = core::addSafFolder,
                    onRemoveSafFolder = core::removeSafFolder,
                    modifier = modifier,
                )
            }
        }
    }
}
