// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.theme.NectarlinkTheme
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** The find-device alarm controls, shown once the device is unlocked. */
class RingActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val core = (application as NectarlinkApplication).core
        if (core.state.value.ringingPeer == null) {
            finish()
            return
        }
        lifecycleScope.launch {
            core.state.first { it.ringingPeer == null }
            finish()
        }
        val preferences = Preferences(this)
        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            NectarlinkTheme(appearance = appearance) {
                Surface(modifier = Modifier.fillMaxSize()) {
                    Box(
                        modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(24.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Button(
                            onClick = { core.stopRinging() },
                            modifier = Modifier.sizeIn(minWidth = 220.dp, minHeight = 96.dp),
                        ) {
                            Text(
                                stringResource(R.string.action_stop).uppercase(LocalConfiguration.current.locales[0]),
                                style = MaterialTheme.typography.headlineLarge,
                            )
                        }
                    }
                }
            }
        }
    }

    companion object {
        internal fun intent(context: Context) = Intent(context, RingActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
