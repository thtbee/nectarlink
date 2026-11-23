// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.remote

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.android.ui.Preferences

/** Quick Settings tile that opens PC Remote for the last used or online paired PC. */
class RemoteTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        val app = application as? NectarlinkApplication
        val devices = app?.core?.state?.value?.devices.orEmpty().filterNot { it.isAndroid }
        val online = devices.firstOrNull { it.online }
        qsTile?.apply {
            state = if (online != null) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                subtitle = online?.name ?: getString(R.string.remote_tile_subtitle_offline)
            }
            updateTile()
        }
    }

    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        val intent = openRemoteIntent(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                PendingIntent.getActivity(
                    this,
                    REQUEST_CODE,
                    intent,
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }

    companion object {
        private const val REQUEST_CODE = 0x524D

        fun openRemoteIntent(
            context: Context,
            pcId: String? = null,
            mode: RemoteMode? = null,
        ): Intent {
            val chosenPc = pcId ?: Preferences.lastRemotePcId(context) ?: "auto"
            return Intent(context, MainActivity::class.java).apply {
                action = Intent.ACTION_VIEW
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                putExtra("remote_pc", chosenPc)
                mode?.let { putExtra("remote_mode", it.wireName) }
            }
        }

        fun requestUpdate(context: Context) {
            runCatching {
                TileService.requestListeningState(context, ComponentName(context, RemoteTileService::class.java))
            }
        }

        /** Requests pinning a home-screen launcher shortcut for PC Remote (optionally targeting one PC). */
        fun requestPinShortcut(
            context: Context,
            pcId: String? = null,
            pcName: String? = null,
            mode: RemoteMode? = null,
        ): Boolean {
            val manager = context.getSystemService(ShortcutManager::class.java) ?: return false
            if (!manager.isRequestPinShortcutSupported) return false
            val id = if (pcId != null) "remote_${pcId}_${mode?.wireName ?: "default"}" else "remote_auto"
            val label = if (!pcName.isNullOrBlank()) {
                context.getString(R.string.remote_shortcut_label_pc, pcName)
            } else {
                context.getString(R.string.remote_tile_label)
            }
            val shortcut = ShortcutInfo.Builder(context, id)
                .setShortLabel(label)
                .setLongLabel(label)
                .setIcon(Icon.createWithResource(context, R.drawable.ic_touchpad))
                .setIntent(openRemoteIntent(context, pcId, mode))
                .build()
            return runCatching { manager.requestPinShortcut(shortcut, null) }.getOrDefault(false)
        }
    }
}
