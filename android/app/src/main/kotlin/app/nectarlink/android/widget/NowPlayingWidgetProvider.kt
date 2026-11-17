// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Shader
import android.os.Bundle
import android.view.View
import android.widget.RemoteViews
import androidx.core.graphics.createBitmap
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState

/** Now playing: what plays on the PC, with Previous, Play or Pause, and Next. */
class NowPlayingWidgetProvider : BloomWidget() {

    override fun render(context: Context, id: Int, options: Bundle, state: CoreState?): RemoteViews {
        val views = RemoteViews(context.packageName, R.layout.widget_now_playing)
        views.setOnClickPendingIntent(R.id.np_root, Widgets.openApp(context))
        val pc = Widgets.pcFor(context, id, state)
        val pcName = pc?.name?.ifBlank { null } ?: context.getString(R.string.your_pc)
        val several = Widgets.pcs(state).size > 1
        views.setTextViewCompoundDrawablesRelative(R.id.np_source, 0, 0, if (several) R.drawable.ic_widget_chevron else 0, 0)
        if (several) {
            views.setOnClickPendingIntent(R.id.np_source, Widgets.pcPicker(context, id))
            views.setContentDescription(R.id.np_source, context.getString(R.string.widget_choose_pc))
        }
        val media = if (pc?.online == true) {
            (context.applicationContext as? NectarlinkApplication)?.core?.pcMedia?.summaries?.value?.get(pc.id)
        } else {
            null
        }

        if (pc == null || media == null) {
            views.setTextViewText(R.id.np_source, if (pc == null) "" else context.getString(R.string.widget_media_on_pc, pcName))
            views.setTextViewText(
                R.id.np_title,
                context.getString(if (pc == null) R.string.widget_pc_no_pc else R.string.widget_media_nothing),
            )
            views.setTextViewText(
                R.id.np_subtitle,
                if (pc == null) context.getString(R.string.widget_pc_tap_to_pair) else context.getString(R.string.widget_media_nothing_hint, pcName),
            )
            views.setViewVisibility(R.id.np_controls, View.GONE)
            views.setViewVisibility(R.id.np_art_image, View.GONE)
            return views
        }

        views.setTextViewText(R.id.np_source, context.getString(R.string.widget_media_on_pc, pcName))
        views.setTextViewText(R.id.np_title, media.title)
        views.setTextViewText(R.id.np_subtitle, media.subtitle)
        views.setViewVisibility(R.id.np_subtitle, if (media.subtitle.isBlank()) View.GONE else View.VISIBLE)
        val art = media.artwork?.let { rounded(it) }
        if (art != null) {
            views.setImageViewBitmap(R.id.np_art_image, art)
            views.setViewVisibility(R.id.np_art_image, View.VISIBLE)
        } else {
            views.setViewVisibility(R.id.np_art_image, View.GONE)
        }

        views.setViewVisibility(R.id.np_controls, View.VISIBLE)
        views.setImageViewResource(R.id.np_play, if (media.playing) R.drawable.ic_pause else R.drawable.ic_play)
        views.setContentDescription(
            R.id.np_play,
            context.getString(if (media.playing) R.string.media_pause else R.string.media_play),
        )
        views.setOnClickPendingIntent(R.id.np_play, Widgets.action(context, PcWidgetProvider.ACTION_MEDIA_PLAY_PAUSE, pc.id))
        button(views, R.id.np_previous, media.canPrevious, Widgets.action(context, PcWidgetProvider.ACTION_MEDIA_PREVIOUS, pc.id))
        button(views, R.id.np_next, media.canNext, Widgets.action(context, PcWidgetProvider.ACTION_MEDIA_NEXT, pc.id))
        return views
    }

    private fun button(views: RemoteViews, id: Int, enabled: Boolean, tap: android.app.PendingIntent) {
        views.setFloat(id, "setAlpha", if (enabled) 1f else 0.38f)
        views.setOnClickPendingIntent(id, if (enabled) tap else null)
    }

    /**
     * The cover scaled down and cut to the art well's corners. Widgets travel
     * to the launcher as parcels, so a small bitmap keeps them under its limit.
     */
    private fun rounded(source: Bitmap): Bitmap? = runCatching {
        val size = ART_PX
        val scale = maxOf(size.toFloat() / source.width, size.toFloat() / source.height)
        val out = createBitmap(size, size)
        val shader = BitmapShader(source, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP).apply {
            setLocalMatrix(
                android.graphics.Matrix().apply {
                    setScale(scale, scale)
                    postTranslate((size - source.width * scale) / 2f, (size - source.height * scale) / 2f)
                },
            )
        }
        val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG).apply { this.shader = shader }
        // The well's 18dp corner, in the bitmap's own pixels.
        val radius = 18f * size / ART_DP
        Canvas(out).drawRoundRect(RectF(0f, 0f, size.toFloat(), size.toFloat()), radius, radius, paint)
        out
    }.getOrNull()

    private companion object {
        /** The art well's size in the layout. */
        const val ART_DP = 96f
        const val ART_PX = 240
    }
}
