// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android

import android.app.Application
import app.nectarlink.android.core.Core
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** Starts the core with the process and keeps it for the app's lifetime. */
class NectarlinkApplication : Application() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val core: Core by lazy { Core(this, scope) }

    override fun onCreate() {
        super.onCreate()
        core.start()
    }
}
