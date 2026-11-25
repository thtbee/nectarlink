// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.content.Context
import android.content.Intent
import android.net.Uri

/**
 * The F-Droid build doesn't update itself: F-Droid does. This keeps the
 * entry points the rest of the app calls, and they do nothing, except
 * [checkNow], which opens NectarLink's F-Droid page.
 */
@androidx.compose.runtime.Stable
class AppUpdater(context: Context) {
    private val context = context.applicationContext

    /** Whether this build updates itself: never, F-Droid does. */
    val enabled: Boolean get() = false

    /** Unused: there's no feed to see here. */
    @Volatile var onFeedSeen: ((channel: String, version: String) -> Unit)? = null

    /** A paired device heard of a newer build: F-Droid brings it. */
    fun onHint(channel: String, version: String) = Unit

    suspend fun updateIfDue() = Unit

    fun onAppResumed() = Unit

    /** The first run after an update: nothing to clean up, F-Droid installed it. */
    fun onUpdated() = Unit

    /** "Update" asked for (the crash card): the F-Droid page, where updates come from. */
    fun checkNow() {
        val page = Uri.parse("https://f-droid.org/packages/${context.packageName}/")
        runCatching {
            context.startActivity(
                Intent(Intent.ACTION_VIEW, page).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }
}
