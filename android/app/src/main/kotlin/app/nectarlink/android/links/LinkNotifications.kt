// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.links

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.core.net.toUri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.core.ClipKind
import app.nectarlink.core.ClipSuggestion
import app.nectarlink.core.mapsWebUrl
import app.nectarlink.core.trackingSearchUrl

/**
 * Links and smart clipboard actions from a PC: Android doesn't let an app in
 * the background open screens, so links and background clip suggestions wait
 * in a notification on the same channel.
 */
object LinkNotifications {
    private const val CHANNEL = "links"
    private const val TAG = "link"
    private const val CLIP_TAG = "clip_suggestion"
    private const val CLIP_NOTIFICATION_ID = 0x434C4950 // "CLIP"

    fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_links), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** Shows a link, video with timestamp, or map location from a PC; false if notifications are off. */
    fun show(context: Context, pcName: String, url: String): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        val handoff = app.nectarlink.core.extractHandoffLink(url)
        val viewIntent = if (url.startsWith("geo:", ignoreCase = true)) {
            val geoIntent = Intent(Intent.ACTION_VIEW, url.toUri()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (geoIntent.resolveActivity(context.packageManager) != null) {
                geoIntent
            } else {
                val webFallback = app.nectarlink.core.geoToMapsHttps(url) ?: url
                Intent(Intent.ACTION_VIEW, webFallback.toUri()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
        } else {
            Intent(Intent.ACTION_VIEW, url.toUri()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        // When Nectarlink is in the foreground, Android allows opening the target right away.
        runCatching { context.startActivity(viewIntent) }

        val open = PendingIntent.getActivity(
            context,
            url.hashCode(),
            viewIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val from = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val title = when (handoff?.kind) {
            app.nectarlink.core.HandoffKind.MAP_LOCATION -> context.getString(R.string.handoff_map_from, from)
            app.nectarlink.core.HandoffKind.VIDEO_LINK -> context.getString(R.string.handoff_video_from, from)
            else -> context.getString(R.string.link_from, from)
        }
        val shown = handoff?.label ?: url.substringAfter("://").removeSuffix("/")
        val actionLabel = when (handoff?.kind) {
            app.nectarlink.core.HandoffKind.MAP_LOCATION -> context.getString(R.string.clip_action_maps)
            app.nectarlink.core.HandoffKind.VIDEO_LINK -> context.getString(R.string.handoff_action_continue_video)
            else -> context.getString(R.string.clip_action_open)
        }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(shown)
            .setStyle(NotificationCompat.BigTextStyle().bigText(shown))
            .setContentIntent(open)
            .addAction(0, actionLabel, open)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(TAG, url.hashCode(), notification) }.isSuccess
    }

    /** Short action chip label for a [ClipKind]. */
    fun actionLabel(context: Context, kind: ClipKind): String = when (kind) {
        ClipKind.WEB_LINK -> context.getString(R.string.clip_action_open)
        ClipKind.STREET_ADDRESS -> context.getString(R.string.clip_action_maps)
        ClipKind.PHONE_NUMBER -> context.getString(R.string.clip_action_call)
        ClipKind.TRACKING_NUMBER -> context.getString(R.string.clip_action_track)
        ClipKind.EMAIL -> context.getString(R.string.clip_action_email)
    }

    /** Builds the Android [Intent] for a [ClipSuggestion]. */
    fun intentForSuggestion(context: Context, suggestion: ClipSuggestion): Intent = when (suggestion.kind) {
        ClipKind.WEB_LINK -> Intent(Intent.ACTION_VIEW, suggestion.target.toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        ClipKind.STREET_ADDRESS -> {
            val geoIntent = Intent(
                Intent.ACTION_VIEW,
                Uri.parse("geo:0,0?q=${Uri.encode(suggestion.target)}"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (geoIntent.resolveActivity(context.packageManager) != null) {
                geoIntent
            } else {
                Intent(Intent.ACTION_VIEW, mapsWebUrl(suggestion.target).toUri())
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
        }
        ClipKind.PHONE_NUMBER -> Intent(
            Intent.ACTION_DIAL,
            Uri.fromParts("tel", suggestion.target, null),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        ClipKind.TRACKING_NUMBER -> Intent(
            Intent.ACTION_VIEW,
            trackingSearchUrl(suggestion.target).toUri(),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        ClipKind.EMAIL -> Intent(
            Intent.ACTION_SENDTO,
            Uri.fromParts("mailto", suggestion.target, null),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /** Runs a [ClipSuggestion] action on this phone, with web/view fallback if needed. */
    fun runSuggestion(context: Context, suggestion: ClipSuggestion) {
        dismissClipSuggestion(context)
        val primary = intentForSuggestion(context, suggestion)
        if (runCatching { context.startActivity(primary) }.isSuccess) return
        val fallback = when (suggestion.kind) {
            ClipKind.STREET_ADDRESS -> Intent(Intent.ACTION_VIEW, mapsWebUrl(suggestion.target).toUri())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            ClipKind.EMAIL -> Intent(Intent.ACTION_VIEW, Uri.fromParts("mailto", suggestion.target, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            else -> null
        }
        if (fallback != null) {
            runCatching { context.startActivity(fallback) }
        }
    }

    /** Shows a silent notification with the suggested action chip for a received clip. */
    fun showClipSuggestion(context: Context, pcName: String, suggestion: ClipSuggestion): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        val intent = intentForSuggestion(context, suggestion)
        val pending = PendingIntent.getActivity(
            context,
            CLIP_NOTIFICATION_ID,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val label = actionLabel(context, suggestion.kind)
        val from = pcName.ifEmpty { context.getString(R.string.your_pc) }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.clip_received_from, from))
            .setContentText(label)
            .setContentIntent(pending)
            .addAction(0, label, pending)
            .setSilent(true)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()
        return runCatching {
            NotificationManagerCompat.from(context).notify(CLIP_TAG, CLIP_NOTIFICATION_ID, notification)
        }.isSuccess
    }

    fun dismissClipSuggestion(context: Context) {
        runCatching { NotificationManagerCompat.from(context).cancel(CLIP_TAG, CLIP_NOTIFICATION_ID) }
    }
}

