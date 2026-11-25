// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri

/**
 * The Google Play build doesn't update itself: Play does. This keeps the
 * entry points the rest of the app calls, and they do nothing, except
 * [checkNow], which opens NectarLink's Play listing.
 */
@androidx.compose.runtime.Stable
class AppUpdater(context: Context) {
    private val context = context.applicationContext

    /** Whether this build updates itself: never, Play does. */
    val enabled: Boolean get() = false

    /** Unused: there's no feed to see here. */
    @Volatile var onFeedSeen: ((channel: String, version: String) -> Unit)? = null

    /** A paired device heard of a newer build: Play brings it. */
    fun onHint(channel: String, version: String) = Unit

    suspend fun updateIfDue() = Unit

    fun onAppResumed() = Unit

    /** The first run after an update: nothing to clean up, Play installed it. */
    fun onUpdated() = Unit

    /** "Update" asked for (the crash card): the Play listing, where updates come from. */
    fun checkNow() {
        val listing = Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=${context.packageName}"))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(listing)
        } catch (_: ActivityNotFoundException) {
            val web = Uri.parse("https://play.google.com/store/apps/details?id=${context.packageName}")
            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, web).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
    }
}
