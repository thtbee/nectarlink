// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.app.Activity
import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.util.Log
import androidx.core.content.edit
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.mic.MicService
import app.nectarlink.android.webcam.WebcamService
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Start without asking (#132): a PC the user chose turns on this phone's
 * camera or microphone as soon as it asks, with no tap on the phone.
 *
 * Android starts the camera and microphone only for an app on screen. The
 * Elevated helper may open an app's screen, so it opens [PcStartActivity],
 * which starts the request that just came in and closes. Without Elevated,
 * or if that fails, the request's notification is there as always.
 */
object PcAutoStart {
    enum class Kind { WEBCAM, MIC }

    internal data class Pending(val kind: Kind, val pcId: String, val at: Long)

    private const val TAG = "PcAutoStart"
    private const val PREFS = "pc_auto_start"
    private const val KEY = "pcs"

    /** How long a request waits for [PcStartActivity] to open. */
    private const val PENDING_MS = 15_000L

    /** The one shell command the helper runs for this (allowed by name in InputServer). */
    const val OPEN_COMMAND = "am start -n ${app.nectarlink.android.BuildConfig.APPLICATION_ID}/app.nectarlink.android.service.PcStartActivity"

    @Volatile private var pending: Pending? = null
    @Volatile private var loaded = false
    private val _pcs = MutableStateFlow(emptySet<String>())

    /** The PCs that may start the camera and microphone without asking. */
    fun pcs(context: Context): StateFlow<Set<String>> {
        load(context)
        return _pcs.asStateFlow()
    }

    fun isOn(context: Context, pcId: String): Boolean = pcId in pcs(context).value

    @Synchronized
    fun set(context: Context, pcId: String, on: Boolean) {
        load(context)
        val next = if (on) _pcs.value + pcId else _pcs.value - pcId
        prefs(context).edit { putStringSet(KEY, next) }
        _pcs.value = next
    }

    @Synchronized
    private fun load(context: Context) {
        if (loaded) return
        _pcs.value = prefs(context).getStringSet(KEY, emptySet()).orEmpty().toSet()
        loaded = true
    }

    /**
     * Starts [kind] for [pcId] without asking, if the user chose that for it
     * and Elevated is running. False otherwise: the caller asks as usual.
     */
    fun tryStart(context: Context, kind: Kind, pcId: String): Boolean {
        if (!isOn(context, pcId) || !Elevated.running) return false
        offer(Pending(kind, pcId, SystemClock.elapsedRealtime()))
        return Elevated.runCommand(OPEN_COMMAND).also { if (!it) pending = null }
    }

    internal fun offer(request: Pending) {
        pending = request
    }

    /** The request that just came in, once; null when there's none or it's gone stale. */
    internal fun take(now: Long = SystemClock.elapsedRealtime()): Pending? {
        val request = pending ?: return null
        pending = null
        return request.takeIf { now - it.at in 0..PENDING_MS }
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    internal fun logStartFailed(error: Throwable) = Log.w(TAG, "can't start without asking", error)
}

/**
 * Opened by the Elevated helper for [PcAutoStart]: starts the camera or
 * microphone a chosen PC just asked for, now that the app is on screen, and
 * closes. Exported (the helper opens it as the shell), so it acts only on a
 * request from the last few seconds that the user allowed for that PC; opened
 * any other way it closes and does nothing.
 */
class PcStartActivity : Activity() {
    private var started = false

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        // A phone on a stand is often locked; the camera starts there too.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) setShowWhenLocked(true)
    }

    override fun onResume() {
        super.onResume()
        // On screen now, as Android requires for the camera and microphone.
        if (!started) {
            started = true
            runCatching { start() }.onFailure(PcAutoStart::logStartFailed)
        }
        finish()
    }

    private fun start() {
        val request = PcAutoStart.take() ?: return
        if (!PcAutoStart.isOn(this, request.pcId)) return
        val core = (application as NectarlinkApplication).core
        val state = core.state.value
        when (request.kind) {
            PcAutoStart.Kind.WEBCAM -> {
                val webcam = state.webcamRequest?.takeIf { it.pcId == request.pcId } ?: return
                if (!WebcamService.hasPermission(this)) return
                WebcamService.start(this, webcam.pcId, webcam.height, webcam.fps, webcam.camera)
                core.clearWebcamRequest(webcam.pcId)
            }
            PcAutoStart.Kind.MIC -> {
                if (state.micRequestFrom != request.pcId || !MicService.hasPermission(this)) return
                core.clearMicRequest(request.pcId)
                MicService.start(this, request.pcId)
            }
        }
    }
}
