// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.format.DateUtils
import android.util.Log
import android.widget.RemoteViews
import androidx.core.content.edit
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.MainActivity

/**
 * A Bloom home-screen widget. None of them wake the phone on a timer
 * (`updatePeriodMillis="0"`): [Widgets.refreshAll] redraws them when what
 * they show changes.
 */
abstract class BloomWidget : AppWidgetProvider() {
    /** Widget [id] as it looks at the size [options] gives, for [state]. */
    abstract fun render(context: Context, id: Int, options: Bundle, state: CoreState?): RemoteViews

    override fun onDeleted(context: Context, ids: IntArray) {
        for (id in ids) Widgets.forgetPc(context, id)
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val core = (context.applicationContext as? NectarlinkApplication)?.core ?: return
        core.start()
        val state = core.state.value
        // While Core starts, the devices aren't loaded yet: keep what's drawn
        // rather than flash "No PC paired". Core redraws once it's ready.
        if (state.status == CoreStatus.Starting && state.devices.isEmpty()) return
        draw(context, manager, ids, state)
    }

    override fun onAppWidgetOptionsChanged(context: Context, manager: AppWidgetManager, id: Int, options: Bundle) {
        val state = (context.applicationContext as? NectarlinkApplication)?.core?.state?.value
        runCatching { manager.updateAppWidget(id, render(context, id, options, state)) }
            .onFailure { Log.w(TAG, "couldn't draw widget $id", it) }
    }

    fun draw(context: Context, manager: AppWidgetManager, ids: IntArray, state: CoreState?) {
        for (id in ids) {
            runCatching { manager.updateAppWidget(id, render(context, id, manager.getAppWidgetOptions(id), state)) }
                .onFailure { Log.w(TAG, "couldn't draw widget $id", it) }
        }
    }

    private companion object {
        const val TAG = "BloomWidget"
    }
}

/** What the widgets share: redrawing them, the PC they act on, and their taps. */
object Widgets {
    private fun all(): List<BloomWidget> = listOf(
        PcWidgetProvider(),
        NowPlayingWidgetProvider(),
        ActionsWidgetProvider(),
        FromPcWidgetProvider(),
    )

    /** Redraws every placed widget, from [state] or Core's current one. */
    fun refreshAll(context: Context, state: CoreState? = null) {
        val manager = runCatching { AppWidgetManager.getInstance(context) }.getOrNull() ?: return
        val current = state ?: (context.applicationContext as? NectarlinkApplication)?.core?.state?.value
        for (widget in all()) {
            val ids = runCatching { manager.getAppWidgetIds(ComponentName(context, widget.javaClass)) }.getOrNull()
            if (ids == null || ids.isEmpty()) continue
            widget.draw(context, manager, ids, current)
        }
    }

    /** The paired PCs, the ones a widget can act on. */
    fun pcs(state: CoreState?): List<Device> = state?.devices.orEmpty().filter { !it.isAndroid }

    /** The PC the widgets act on by default: the first connected one, else the first paired. */
    fun primaryPc(state: CoreState?): Device? {
        val pcs = pcs(state)
        return pcs.firstOrNull { it.online } ?: pcs.firstOrNull()
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** The PC widget [widgetId] shows: the one picked for it while it's still paired, else [primaryPc]. */
    fun pcFor(context: Context, widgetId: Int, state: CoreState?): Device? {
        val picked = prefs(context).getString(KEY_PC + widgetId, null)
        return pcs(state).firstOrNull { it.id == picked } ?: primaryPc(state)
    }

    fun choosePc(context: Context, widgetId: Int, pcId: String) {
        prefs(context).edit { putString(KEY_PC + widgetId, pcId) }
        refreshAll(context)
    }

    fun forgetPc(context: Context, widgetId: Int) {
        prefs(context).edit { remove(KEY_PC + widgetId) }
    }

    /** The PC's OS as a small logo, or a plain monitor for one that doesn't say. */
    fun osIcon(pc: Device): Int = when (pc.os) {
        "windows" -> R.drawable.ic_os_windows
        "macos" -> R.drawable.ic_os_macos
        "linux" -> R.drawable.ic_os_linux
        else -> R.drawable.ic_widget_pc
    }

    /** Opens the power menu for [pcId] over the home screen. */
    fun powerMenu(context: Context, widgetId: Int, pcId: String): PendingIntent =
        activity(context, widgetId * 4 + 1, WidgetMenuActivity.powerIntent(context, pcId))

    /** Opens the list of PCs to pick the one widget [widgetId] shows. */
    fun pcPicker(context: Context, widgetId: Int): PendingIntent =
        activity(context, widgetId * 4 + 2, WidgetMenuActivity.pickIntent(context, widgetId))

    fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        REQ_OPEN_APP,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    fun activity(context: Context, requestCode: Int, intent: Intent): PendingIntent = PendingIntent.getActivity(
        context,
        requestCode,
        intent,
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    /** A tap handled by [PcWidgetProvider.ActionReceiver], which no other app can send to. */
    fun action(context: Context, action: String, pcId: String?, extras: Bundle? = null): PendingIntent {
        val intent = Intent(context, PcWidgetProvider.ActionReceiver::class.java).setAction(action)
        if (!pcId.isNullOrEmpty()) intent.putExtra(PcWidgetProvider.EXTRA_PC_ID, pcId)
        if (extras != null) intent.putExtras(extras)
        return PendingIntent.getBroadcast(
            context,
            action.hashCode(),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    /** When something arrived, as a clock time today or a date before, which doesn't go stale on the home screen. */
    fun whenText(context: Context, secs: Long): String {
        val ms = secs * 1000L
        val flags = if (DateUtils.isToday(ms)) {
            DateUtils.FORMAT_SHOW_TIME
        } else {
            DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH
        }
        return DateUtils.formatDateTime(context, ms, flags)
    }

    /** The widget's height in dp as the launcher lays it out, from its options. */
    fun heightDp(options: Bundle): Int =
        maxOf(options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT), options.getInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT))

    private const val REQ_OPEN_APP = 100
    private const val PREFS = "widgets"
    private const val KEY_PC = "pc_"
}
