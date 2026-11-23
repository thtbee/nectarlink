// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.nectarlink.android.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

class AudioMixerTileService : TileService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var watcher: Job? = null

    override fun onStartListening() {
        super.onStartListening()
        watcher?.cancel()
        watcher = scope.launch {
            AudioRouting.state.map { state -> state.listening to subtitle(state) }.distinctUntilChanged()
                .collect { (listening, subtitle) ->
                    qsTile?.apply {
                        state = if (listening) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) setSubtitle(subtitle)
                        updateTile()
                    }
                }
        }
    }

    override fun onStopListening() {
        watcher?.cancel()
        watcher = null
        super.onStopListening()
    }

    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        val intent = AudioMixerActivity.openIntent(this, startListening = true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun subtitle(state: AudioRouting.State): String {
        if (!state.listening) return getString(R.string.audio_mixer_off)
        state.outputPeer()?.let { return getString(R.string.audio_mixer_on_pc, state.nameOf(it.deviceId)) }
        val count = state.peers.count { it.online && state.lanes[it.deviceId]?.active == true }
        return if (count == 0) {
            getString(R.string.audio_mixer_listening)
        } else {
            resources.getQuantityString(R.plurals.audio_mixer_active_pc_count, count, count)
        }
    }

    companion object {
        fun requestUpdate(context: Context) {
            TileService.requestListeningState(context, ComponentName(context, AudioMixerTileService::class.java))
        }
    }
}
