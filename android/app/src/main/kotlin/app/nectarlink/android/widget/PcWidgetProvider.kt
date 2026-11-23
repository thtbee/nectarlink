// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.View
import android.widget.RemoteViews
import android.widget.Toast
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import app.nectarlink.core.Link
import app.nectarlink.core.MediaAction

/**
 * Glance: a PC with its OS's logo, whether it's connected, its battery or
 * when it was last seen, and Ring and the power menu, or Wake while it's off.
 * With more than one PC, its name picks the PC. Keeps the class name of the
 * first widget so widgets already on home screens carry on.
 */
class PcWidgetProvider : BloomWidget() {

    override fun render(context: Context, id: Int, options: Bundle, state: CoreState?): RemoteViews {
        val views = RemoteViews(context.packageName, R.layout.widget_pc)
        views.setOnClickPendingIntent(R.id.widget_root, Widgets.openApp(context))
        val pc = Widgets.pcFor(context, id, state)
        if (pc == null) {
            views.setTextViewText(R.id.widget_pc_name, context.getString(R.string.widget_pc_no_pc))
            views.setTextViewText(R.id.widget_pc_subtitle, context.getString(R.string.widget_pc_tap_to_pair))
            views.setViewVisibility(R.id.widget_pc_status, View.GONE)
            views.setViewVisibility(R.id.widget_pc_status_off, View.GONE)
            views.setViewVisibility(R.id.widget_actions_row, View.GONE)
            views.setViewVisibility(R.id.widget_btn_wake, View.GONE)
            views.setViewVisibility(R.id.widget_pc_subtitle, View.VISIBLE)
            views.setViewVisibility(R.id.widget_fill, View.VISIBLE)
            views.setTextViewCompoundDrawablesRelative(R.id.widget_pc_name, 0, 0, 0, 0)
            views.setImageViewResource(R.id.widget_pc_icon, R.drawable.ic_widget_pc)
            return views
        }

        val name = pc.name.ifBlank { context.getString(R.string.your_pc) }
        views.setImageViewResource(R.id.widget_pc_icon, Widgets.osIcon(pc))
        val several = Widgets.pcs(state).size > 1
        views.setTextViewCompoundDrawablesRelative(R.id.widget_pc_name, 0, 0, if (several) R.drawable.ic_widget_chevron else 0, 0)
        if (several) {
            views.setOnClickPendingIntent(R.id.widget_pc_name, Widgets.pcPicker(context, id))
            views.setContentDescription(R.id.widget_pc_name, "$name. ${context.getString(R.string.widget_choose_pc)}")
        }

        views.setTextViewText(R.id.widget_pc_name, name)
        val waking = !pc.online && pc.wakeState == WakeState.Waking
        val pill = when {
            pc.online -> R.string.widget_pc_connected
            waking -> R.string.widget_pc_waking
            pc.link == Link.Connecting -> R.string.link_connecting
            else -> R.string.widget_pc_offline
        }
        views.setViewVisibility(R.id.widget_pc_status, if (pc.online) View.VISIBLE else View.GONE)
        views.setViewVisibility(R.id.widget_pc_status_off, if (pc.online) View.GONE else View.VISIBLE)
        views.setTextViewText(if (pc.online) R.id.widget_pc_status else R.id.widget_pc_status_off, context.getString(pill))
        views.setTextViewText(R.id.widget_pc_subtitle, subtitle(context, pc))

        val wake = !pc.online && pc.canWake && !waking
        // A short widget has no room for the "Seen" line and a whole Wake button.
        val short = wake && Widgets.heightDp(options) in 1 until SHORT_DP
        views.setViewVisibility(R.id.widget_pc_subtitle, if (short) View.GONE else View.VISIBLE)
        views.setViewVisibility(R.id.widget_actions_row, if (pc.online) View.VISIBLE else View.GONE)
        views.setViewVisibility(R.id.widget_fill, if (pc.online || short) View.GONE else View.VISIBLE)
        views.setViewVisibility(R.id.widget_btn_wake, if (wake) View.VISIBLE else View.GONE)
        if (pc.online) {
            views.setOnClickPendingIntent(R.id.widget_btn_ring, Widgets.action(context, ACTION_RING_PC, pc.id))
            views.setOnClickPendingIntent(R.id.widget_btn_power, Widgets.powerMenu(context, id, pc.id))
        }
        if (wake) views.setOnClickPendingIntent(R.id.widget_btn_wake, Widgets.action(context, ACTION_WAKE_PC, pc.id))
        return views
    }

    /** The battery while connected (how it connects without one), or when it was last seen. */
    private fun subtitle(context: Context, pc: Device): String = when (val link = pc.link) {
        is Link.Online -> {
            val battery = pc.battery
            when {
                battery != null && battery.charging ->
                    context.getString(R.string.widget_battery_charging, battery.level.toInt())
                battery != null -> context.getString(R.string.widget_battery, battery.level.toInt())
                link.relayed -> context.getString(R.string.link_away)
                link.usb -> context.getString(R.string.link_usb)
                else -> context.getString(R.string.widget_link_lan)
            }
        }
        Link.Connecting -> ""
        is Link.Offline -> when (val seen = link.lastSeen) {
            null -> context.getString(R.string.link_never)
            // A clock time, not "2 minutes ago": the widget only redraws when something changes.
            else -> context.getString(R.string.widget_seen, Widgets.whenText(context, seen))
        }
    }

    /**
     * Not exported, so other apps can't send it taps to ring or wake a
     * paired PC, or play media on it. Every widget's buttons come here.
     */
    class ActionReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val action = intent.action ?: return
            val app = context.applicationContext as? NectarlinkApplication ?: return
            app.core.start()
            if (action == ACTION_COPY_CLIP) {
                val clip = intent.getStringExtra(EXTRA_CLIP_ID) ?: return
                app.core.copyClipboardHistory(clip)
                // Android 13 and later show their own confirmation.
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                    Toast.makeText(context, R.string.clipboard_history_copied, Toast.LENGTH_SHORT).show()
                }
                return
            }
            val state = app.core.state.value
            val pcId = intent.getStringExtra(EXTRA_PC_ID)?.takeIf { it.isNotBlank() }
                ?: Widgets.primaryPc(state)?.id
                ?: return
            val pc = state.devices.firstOrNull { it.id == pcId }
            val pcName = pc?.name?.takeIf { it.isNotBlank() } ?: context.getString(R.string.your_pc)

            when (action) {
                ACTION_RING_PC -> {
                    if (pc?.online != true) return
                    app.core.ring(pcId, true)
                    toast(context, context.getString(R.string.widget_pc_ringing_toast, pcName))
                }
                ACTION_WAKE_PC -> {
                    app.core.wake(pcId)
                    toast(context, context.getString(R.string.widget_pc_waking_toast, pcName))
                }
                ACTION_MEDIA_PLAY_PAUSE, ACTION_MEDIA_NEXT, ACTION_MEDIA_PREVIOUS -> {
                    val media = app.core.pcMedia
                    val summary = media.summaries.value[pcId] ?: return
                    val command = when (action) {
                        ACTION_MEDIA_NEXT -> MediaAction.NEXT
                        ACTION_MEDIA_PREVIOUS -> MediaAction.PREVIOUS
                        else -> if (summary.playing) MediaAction.PAUSE else MediaAction.PLAY
                    }
                    media.command(pcId, summary.playerId, command)
                }
            }
        }

        private fun toast(context: Context, text: String) = Toast.makeText(context, text, Toast.LENGTH_SHORT).show()
    }

    companion object {
        /** Below this height (dp) the offline widget drops its "Seen" line for Wake. */
        private const val SHORT_DP = 160
        const val ACTION_RING_PC = "app.nectarlink.widget.ACTION_RING_PC"
        const val ACTION_WAKE_PC = "app.nectarlink.widget.ACTION_WAKE_PC"
        const val ACTION_MEDIA_PLAY_PAUSE = "app.nectarlink.widget.ACTION_MEDIA_PLAY_PAUSE"
        const val ACTION_MEDIA_NEXT = "app.nectarlink.widget.ACTION_MEDIA_NEXT"
        const val ACTION_MEDIA_PREVIOUS = "app.nectarlink.widget.ACTION_MEDIA_PREVIOUS"
        const val ACTION_COPY_CLIP = "app.nectarlink.widget.ACTION_COPY_CLIP"
        const val EXTRA_PC_ID = "app.nectarlink.widget.EXTRA_PC_ID"
        const val EXTRA_CLIP_ID = "app.nectarlink.widget.EXTRA_CLIP_ID"
    }
}
