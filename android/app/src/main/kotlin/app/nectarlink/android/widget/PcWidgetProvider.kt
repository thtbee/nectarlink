// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.view.View
import android.widget.RemoteViews
import android.widget.Toast
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.ui.MainActivity

/**
 * Battery-free home-screen widget showing the primary paired PC's connection
 * status, Wake-on-LAN button when offline, and one-tap quick actions (Send
 * clipboard, Ring PC, Lock PC). Has `updatePeriodMillis="0"` so it never wakes
 * the device on a timer; [Core] refreshes it whenever state changes.
 */
class PcWidgetProvider : AppWidgetProvider() {

    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray,
    ) {
        val core = (context.applicationContext as? NectarlinkApplication)?.core
        core?.start()
        updateWidgets(context, appWidgetManager, appWidgetIds, core?.state?.value)
    }

    override fun onReceive(context: Context, intent: Intent) {
        super.onReceive(context, intent)
        val action = intent.action ?: return
        if (action != ACTION_RING_PC && action != ACTION_LOCK_PC && action != ACTION_WAKE_PC) return

        val app = context.applicationContext as? NectarlinkApplication ?: return
        app.core.start()
        val state = app.core.state.value
        val pcId = intent.getStringExtra(EXTRA_PC_ID)?.takeIf { it.isNotBlank() }
            ?: primaryPc(state)?.id
            ?: return
        val pcName = state.nameOf(pcId)?.takeIf { it.isNotBlank() }
            ?: context.getString(R.string.your_pc)

        when (action) {
            ACTION_RING_PC -> {
                app.core.ring(pcId, true)
                Toast.makeText(
                    context,
                    context.getString(R.string.widget_pc_ringing_toast, pcName),
                    Toast.LENGTH_SHORT,
                ).show()
            }
            ACTION_LOCK_PC -> {
                app.core.pcPower(pcId, sleep = false)
                Toast.makeText(
                    context,
                    context.getString(R.string.pc_locked, pcName),
                    Toast.LENGTH_SHORT,
                ).show()
            }
            ACTION_WAKE_PC -> {
                app.core.wake(pcId)
                Toast.makeText(
                    context,
                    context.getString(R.string.widget_pc_waking_toast, pcName),
                    Toast.LENGTH_SHORT,
                ).show()
            }
        }
    }

    companion object {
        const val ACTION_RING_PC = "app.nectarlink.widget.ACTION_RING_PC"
        const val ACTION_LOCK_PC = "app.nectarlink.widget.ACTION_LOCK_PC"
        const val ACTION_WAKE_PC = "app.nectarlink.widget.ACTION_WAKE_PC"
        const val EXTRA_PC_ID = "app.nectarlink.widget.EXTRA_PC_ID"

        private const val REQ_OPEN_APP = 100
        private const val REQ_SEND_CLIP = 101
        private const val REQ_RING = 102
        private const val REQ_LOCK = 103
        private const val REQ_WAKE = 104

        private val COLOR_ONLINE_TEXT = Color.parseColor("#9EF0B4")
        private val COLOR_OFFLINE_TEXT = Color.parseColor("#D5CFC2")

        private fun primaryPc(state: CoreState?): Device? =
            state?.devices?.firstOrNull { it.online } ?: state?.devices?.firstOrNull()

        fun refreshAll(context: Context, state: CoreState? = null) {
            val manager = AppWidgetManager.getInstance(context) ?: return
            val ids = runCatching {
                manager.getAppWidgetIds(ComponentName(context, PcWidgetProvider::class.java))
            }.getOrNull() ?: return
            if (ids.isEmpty()) return

            val effectiveState = state
                ?: (context.applicationContext as? NectarlinkApplication)?.core?.state?.value
            updateWidgets(context, manager, ids, effectiveState)
        }

        private fun updateWidgets(
            context: Context,
            appWidgetManager: AppWidgetManager,
            appWidgetIds: IntArray,
            state: CoreState?,
        ) {
            if (appWidgetIds.isEmpty()) return
            val pc = primaryPc(state)
            val views = RemoteViews(context.packageName, R.layout.widget_pc)

            val launchIntent = Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            val launchPending = PendingIntent.getActivity(
                context,
                REQ_OPEN_APP,
                launchIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_root, launchPending)

            if (pc == null) {
                views.setTextViewText(R.id.widget_pc_name, context.getString(R.string.widget_pc_no_pc))
                views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_offline))
                views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                views.setViewVisibility(R.id.widget_pc_subtitle, View.VISIBLE)
                views.setTextViewText(R.id.widget_pc_subtitle, context.getString(R.string.widget_pc_tap_to_pair))
                views.setViewVisibility(R.id.widget_btn_wake, View.GONE)
            } else {
                val name = pc.name.ifBlank { context.getString(R.string.your_pc) }
                views.setTextViewText(R.id.widget_pc_name, name)
                views.setViewVisibility(R.id.widget_pc_subtitle, View.GONE)

                when {
                    pc.online -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_connected))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_online)
                        views.setTextColor(R.id.widget_pc_status, COLOR_ONLINE_TEXT)
                    }
                    pc.wakeState == WakeState.Waking -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_waking))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                        views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                    }
                    else -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_offline))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                        views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                    }
                }

                if (!pc.online && pc.canWake) {
                    views.setViewVisibility(R.id.widget_btn_wake, View.VISIBLE)
                    views.setOnClickPendingIntent(
                        R.id.widget_btn_wake,
                        actionBroadcast(context, REQ_WAKE, ACTION_WAKE_PC, pc.id),
                    )
                } else {
                    views.setViewVisibility(R.id.widget_btn_wake, View.GONE)
                }
            }

            val clipIntent = SendActivity.sendClipboardIntent(context, pc?.id)
            val clipPending = PendingIntent.getActivity(
                context,
                REQ_SEND_CLIP,
                clipIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_btn_send_clip, clipPending)

            views.setOnClickPendingIntent(
                R.id.widget_btn_ring,
                actionBroadcast(context, REQ_RING, ACTION_RING_PC, pc?.id),
            )
            views.setOnClickPendingIntent(
                R.id.widget_btn_lock,
                actionBroadcast(context, REQ_LOCK, ACTION_LOCK_PC, pc?.id),
            )

            for (id in appWidgetIds) {
                appWidgetManager.updateAppWidget(id, views)
            }
        }

        private fun actionBroadcast(
            context: Context,
            requestCode: Int,
            action: String,
            pcId: String?,
        ): PendingIntent {
            val intent = Intent(context, PcWidgetProvider::class.java).apply {
                this.action = action
                if (!pcId.isNullOrEmpty()) {
                    putExtra(EXTRA_PC_ID, pcId)
                } else {
                    removeExtra(EXTRA_PC_ID)
                }
            }
            return PendingIntent.getBroadcast(
                context,
                requestCode,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
    }
}
