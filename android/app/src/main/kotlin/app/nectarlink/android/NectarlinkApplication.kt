// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android

import android.app.Application
import android.content.Context
import android.content.SharedPreferences
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.core.Core
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.update.AppUpdater
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** Starts the core with the process and keeps it for the app's lifetime. */
class NectarlinkApplication : Application(), DefaultLifecycleObserver {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val core: Core by lazy { Core(this, scope) }

    /** Updates from GitHub releases (release builds only). */
    val updater: AppUpdater by lazy { AppUpdater(this) }

    private var pendingStopJob: Job? = null

    private val prefListener =
        SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == Preferences.BACKGROUND_SERVICE_ENABLED) {
                if (Preferences.isBackgroundServiceEnabled(this)) {
                    pendingStopJob?.cancel()
                    pendingStopJob = null
                } else {
                    if (!isInForeground()) {
                        scheduleStop()
                    }
                }
            }
        }

    override fun onCreate() {
        super<Application>.onCreate()
        // Before the core starts: its network code reads the phone's DNS servers.
        app.nectarlink.android.core.AndroidContext.install(this)
        // Before Elevated: it waits while paused for a banking app.
        BankingPause.init(this)
        BankingPause.restoreIfStranded(this)
        // The media volume, if the app stopped while the phone's sound played on the PC.
        app.nectarlink.android.mirror.PhoneQuiet.end(this)
        Elevated.init(this)
        // What the phone offers PCs follows Elevated starting and stopping.
        Elevated.onChange = { core.refreshNotificationAccess() }
        Elevated.onClipboardChanged = { text, html -> core.onElevatedClipboardChanged(text, html) }
        Elevated.onSensitiveCopied = { text -> core.onElevatedSensitiveCopied(text) }
        Elevated.onInputBlocked = { core.onElevatedInputBlocked() }

        // A build the phone sees in a feed is passed on to its PCs.
        updater.onFeedSeen = { channel, version -> core.announceUpdate(channel, version) }

        ProcessLifecycleOwner.get().lifecycle.addObserver(this)
        getSharedPreferences("appearance", Context.MODE_PRIVATE)
            .registerOnSharedPreferenceChangeListener(prefListener)

        if (Preferences.isBackgroundServiceEnabled(this)) {
            android.view.Choreographer.getInstance().postFrameCallback {
                scope.launch { core.start() }
            }
            scope.launch {
                delay(120)
                core.start()
            }
        }
    }

    override fun onStart(owner: LifecycleOwner) {
        pendingStopJob?.cancel()
        pendingStopJob = null
        core.start()
    }

    override fun onStop(owner: LifecycleOwner) {
        if (!Preferences.isBackgroundServiceEnabled(this)) {
            scheduleStop()
        }
    }

    fun onBackgroundServiceDisabled() {
        if (!isInForeground()) {
            scheduleStop()
        }
    }

    private fun isInForeground(): Boolean =
        ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

    private fun isStopEligible(): Boolean =
        !isInForeground() && !Preferences.isBackgroundServiceEnabled(this)

    private fun scheduleStop() {
        pendingStopJob?.cancel()
        pendingStopJob = scope.launch(Dispatchers.IO) {
            delay(STOP_DELAY_MS)
            if (!isStopEligible()) return@launch

            while (core.busy.value) {
                core.busy.first { !it }
                delay(STOP_IDLE_DEBOUNCE_MS)
                if (!isStopEligible()) return@launch
            }

            if (isStopEligible()) {
                core.stop()
            }
        }
    }

    companion object {
        const val STOP_DELAY_MS = 5_000L
        const val STOP_IDLE_DEBOUNCE_MS = 2_000L
    }
}
