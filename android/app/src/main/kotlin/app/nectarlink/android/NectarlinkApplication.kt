// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android

import android.app.Application
import app.nectarlink.android.core.Core
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.update.AppUpdater
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** Starts the core with the process and keeps it for the app's lifetime. */
class NectarlinkApplication : Application() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val core: Core by lazy { Core(this, scope) }

    /** Updates from GitHub releases (release builds only). */
    val updater: AppUpdater by lazy { AppUpdater(this) }

    override fun onCreate() {
        super.onCreate()
        Elevated.init(this)
        // What the phone offers PCs follows Elevated starting and stopping.
        Elevated.onChange = { core.refreshNotificationAccess() }
        core.start()
    }
}
