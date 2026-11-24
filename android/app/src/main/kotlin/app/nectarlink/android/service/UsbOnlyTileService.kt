// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.nectarlink.android.NectarlinkApplication
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** The "USB only" Quick Settings tile: on, PCs are reached only through a cable. */
class UsbOnlyTileService : TileService() {
    private val core get() = (application as NectarlinkApplication).core
    private var scope: CoroutineScope? = null

    override fun onStartListening() {
        super.onStartListening()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate).also { this.scope = it }
        scope.launch {
            core.state.map { it.usbOnly }.distinctUntilChanged().collect { render(it) }
        }
    }

    override fun onStopListening() {
        scope?.cancel()
        scope = null
        super.onStopListening()
    }

    override fun onClick() {
        super.onClick()
        core.setUsbOnly(!core.state.value.usbOnly)
    }

    private fun render(usbOnly: Boolean) {
        val tile = qsTile ?: return
        tile.state = if (usbOnly) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.updateTile()
    }
}
