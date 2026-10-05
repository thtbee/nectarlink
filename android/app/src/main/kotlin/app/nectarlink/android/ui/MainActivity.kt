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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.service.ConnectionService
import app.nectarlink.android.ui.home.HomeScreen
import app.nectarlink.android.ui.pairing.PairingActions
import app.nectarlink.android.ui.pairing.PairingScreen
import app.nectarlink.android.ui.settings.SettingsScreen
import app.nectarlink.android.ui.theme.NectarlinkTheme

class MainActivity : ComponentActivity() {
    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val core = (application as NectarlinkApplication).core
        val preferences = Preferences(this)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (savedInstanceState == null) handlePairingLink(intent)

        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            val state by core.state.collectAsStateWithLifecycle()
            // Stay reachable in the background once a PC is paired.
            LaunchedEffect(state.devices.isNotEmpty()) {
                if (state.devices.isNotEmpty()) ConnectionService.start(this@MainActivity)
            }
            NectarlinkTheme(appearance) {
                // Every screen sits on the theme's background, not the
                // window's (which can't follow the app's own light/dark choice).
                Surface(color = MaterialTheme.colorScheme.background) {
                    App(core, state, preferences)
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Back from Settings, maybe with notification access changed.
        (application as NectarlinkApplication).core.refreshNotificationAccess()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePairingLink(intent)
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
private fun App(core: Core, state: CoreState, preferences: Preferences) {
    var tab by rememberSaveable { mutableStateOf(Tab.Home) }
    var pairing by rememberSaveable { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { core.messages.collect { snackbar.showSnackbar(it) } }

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
                Tab.Home -> HomeScreen(state, core::ring, core::stopRinging, onPairNew = { pairing = true }, modifier = modifier)
                Tab.Settings -> SettingsScreen(
                    state = state,
                    appearance = preferences.appearance.collectAsStateWithLifecycle().value,
                    onAppearance = preferences::update,
                    onUnpair = core::unpair,
                    onPairNew = { pairing = true },
                    modifier = modifier,
                )
            }
        }
    }
}
