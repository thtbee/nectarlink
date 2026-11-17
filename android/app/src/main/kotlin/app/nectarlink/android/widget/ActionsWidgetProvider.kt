// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.content.Context
import android.os.Bundle
import android.widget.RemoteViews
import app.nectarlink.android.R
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.CoreState

/**
 * Quick actions: send the clipboard or files, ring the PC, or open its power
 * menu. A row at 4×1, a grid of tiles once it's two cells tall.
 */
class ActionsWidgetProvider : BloomWidget() {

    override fun render(context: Context, id: Int, options: Bundle, state: CoreState?): RemoteViews {
        val layout = if (Widgets.heightDp(options) >= GRID_MIN_HEIGHT_DP) R.layout.widget_actions_grid else R.layout.widget_actions_row
        val views = RemoteViews(context.packageName, layout)
        val pc = Widgets.pcFor(context, id, state)
        if (pc == null) {
            // Nothing to act on yet: every tile opens the app to pair.
            val open = Widgets.openApp(context)
            for (tile in TILES) views.setOnClickPendingIntent(tile, open)
            return views
        }
        views.setOnClickPendingIntent(
            R.id.act_clip,
            Widgets.activity(context, REQ_SEND_CLIP, SendActivity.sendClipboardIntent(context, pc.id)),
        )
        views.setOnClickPendingIntent(
            R.id.act_files,
            Widgets.activity(context, REQ_SEND_FILES, SendActivity.sendFilesIntent(context, pc.id)),
        )
        // Ring needs the PC connected and dims while it isn't. The power
        // menu opens either way: it offers Wake for a PC that's off.
        views.setFloat(R.id.act_ring, "setAlpha", if (pc.online) 1f else 0.38f)
        views.setOnClickPendingIntent(
            R.id.act_ring,
            if (pc.online) Widgets.action(context, PcWidgetProvider.ACTION_RING_PC, pc.id) else Widgets.openApp(context),
        )
        views.setOnClickPendingIntent(R.id.act_power, Widgets.powerMenu(context, id, pc.id))
        return views
    }

    private companion object {
        const val GRID_MIN_HEIGHT_DP = 120
        const val REQ_SEND_CLIP = 101
        const val REQ_SEND_FILES = 105
        val TILES = intArrayOf(R.id.act_clip, R.id.act_files, R.id.act_ring, R.id.act_power)
    }
}
