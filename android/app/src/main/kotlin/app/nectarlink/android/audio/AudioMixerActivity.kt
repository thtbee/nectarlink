// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.theme.NectarlinkTheme
import app.nectarlink.android.ui.theme.glassModalContainerColor
import app.nectarlink.android.ui.theme.glassModalScrimColor

/** Translucent volume panel opened by the notification and Quick Settings. */
class AudioMixerActivity : ComponentActivity() {
    private var selectedLane: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        selectedLane = savedInstanceState?.getString(KEY_SELECTED_LANE)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.setBackgroundDrawableResource(android.R.color.transparent)
        val preferences = Preferences(applicationContext)
        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            NectarlinkTheme(appearance) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
                    Box(
                        Modifier.fillMaxSize().background(glassModalScrimColor())
                            .clickable(interactionSource = null, indication = null) { finish() },
                    )
                    Surface(
                        modifier = Modifier.fillMaxWidth().statusBarsPadding().padding(top = 24.dp),
                        shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
                        color = glassModalContainerColor(),
                        contentColor = androidx.compose.material3.MaterialTheme.colorScheme.onSurface,
                    ) {
                        AudioMixerPanel(
                            modifier = Modifier.navigationBarsPadding()
                                .verticalScroll(rememberScrollState()).padding(20.dp),
                            onExpand = ::openApp,
                            onLaneSelected = { selectedLane = it },
                        )
                    }
                }
            }
        }
        if (savedInstanceState == null) startFromTile(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        startFromTile(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString(KEY_SELECTED_LANE, selectedLane)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        val direction = when (keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP -> 1
            KeyEvent.KEYCODE_VOLUME_DOWN -> -1
            else -> return super.onKeyDown(keyCode, event)
        }
        val selected = activeVolumeLane() ?: return super.onKeyDown(keyCode, event)
        AudioRouting.stepLaneGain(selected, direction)
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent?): Boolean {
        if ((keyCode == KeyEvent.KEYCODE_VOLUME_UP || keyCode == KeyEvent.KEYCODE_VOLUME_DOWN) &&
            activeVolumeLane() != null
        ) {
            return true
        }
        return super.onKeyUp(keyCode, event)
    }

    private fun activeVolumeLane(): String? {
        if (!lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return null
        val state = AudioRouting.state.value
        val online = state.peers.filter { it.online && it.canSource }
        return selectedLane?.takeIf { id -> online.any { it.deviceId == id } }
            ?: state.outputPeer()?.deviceId
            ?: online.firstOrNull { state.lanes[it.deviceId]?.active == true }?.deviceId
            ?: online.firstOrNull()?.deviceId
    }

    private fun startFromTile(intent: Intent) {
        if (intent.getBooleanExtra(EXTRA_START_LISTENING, false)) AudioListeningService.start(this)
    }

    private fun openApp() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP))
        finish()
    }

    companion object {
        private const val EXTRA_START_LISTENING = "start_listening"
        private const val KEY_SELECTED_LANE = "selected_lane"

        fun openIntent(context: Context, startListening: Boolean = false): Intent =
            Intent(context, AudioMixerActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_START_LISTENING, startListening)
    }
}
