// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.app.ActivityOptions
import android.os.Build
import android.os.Bundle

/** How NectarLink presses another app's notification button for a PC. */
internal object ActionStarts {
    /**
     * Lets the button open its app's screen, as tapping it on the phone would
     * (answering a call in WhatsApp does). Android 14+ asks the sender to say so.
     */
    fun options(): Bundle? {
        val mode = when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.BAKLAVA -> ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE -> {
                @Suppress("DEPRECATION")
                ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED
            }
            else -> return null
        }
        return ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(mode).toBundle()
    }
}
