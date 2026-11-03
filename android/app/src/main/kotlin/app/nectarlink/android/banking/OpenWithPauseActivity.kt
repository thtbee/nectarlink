// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.os.Bundle
import android.util.Log
import androidx.core.graphics.drawable.toBitmap

/**
 * Behind an "Open with banking pause" home screen shortcut: pauses first,
 * then opens the banking app, for apps that check as they start, before
 * NectarLink could notice them in front.
 */
class OpenWithPauseActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pkg = intent.getStringExtra(EXTRA_PACKAGE)
        if (pkg != null && pkg in BankingPause.apps(this)) {
            if (BankingPause.isReady(this) && app.nectarlink.android.ui.Preferences.isBackgroundServiceEnabled(this)) {
                app.nectarlink.android.service.ConnectionService.start(applicationContext)
                BankingPause.pause(applicationContext)
            }
            packageManager.getLaunchIntentForPackage(pkg)?.let { launch ->
                runCatching { startActivity(launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
                    .onFailure { Log.w(TAG, "couldn't open the banking app", it) }
            }
        }
        finish()
    }

    companion object {
        private const val TAG = "BankingPause"
        private const val EXTRA_PACKAGE = "package"

        /** Whether this launcher can take a pinned shortcut. */
        fun canPin(context: Context): Boolean =
            context.getSystemService(ShortcutManager::class.java)?.isRequestPinShortcutSupported == true

        /** Asks the launcher to add an "Open with banking pause" shortcut for [pkg]. */
        fun pin(context: Context, pkg: String, label: String): Boolean = runCatching {
            val manager = context.getSystemService(ShortcutManager::class.java) ?: return false
            val intent = Intent(context, OpenWithPauseActivity::class.java)
                .setAction(Intent.ACTION_VIEW)
                .putExtra(EXTRA_PACKAGE, pkg)
            val bitmap = context.packageManager.getApplicationIcon(pkg).toBitmap(ICON_PX, ICON_PX)
            val shortcut = ShortcutInfo.Builder(context, "banking_$pkg")
                .setShortLabel(label)
                .setLongLabel(label)
                .setIcon(Icon.createWithBitmap(bitmap))
                .setIntent(intent)
                .build()
            manager.requestPinShortcut(shortcut, null)
        }.onFailure { Log.w(TAG, "couldn't add the shortcut", it) }.getOrDefault(false)

        private const val ICON_PX = 192
    }
}
