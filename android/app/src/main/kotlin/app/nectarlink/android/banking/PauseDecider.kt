// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

/**
 * When to pause for banking apps, and when to turn things back on, from the
 * app in front as it changes. Kept apart from Android so it can be tested.
 *
 * - A chosen banking app comes to the front: pause right away (most banking
 *   apps check as they open).
 * - Something else stays in front for [resumeAfterMs]: turn things back on.
 *   The wait lets a banking app hand over to another one and back (a UPI
 *   payment, say) without switching settings on and off in between.
 * - Screens Android puts over any app (permission prompts, the system UI)
 *   change nothing: they show over a banking app as often as over others.
 */
class PauseDecider(
    private val resumeAfterMs: Long = RESUME_AFTER_MS,
    private val neutral: Set<String> = NEUTRAL_PACKAGES,
) {
    enum class Action { PAUSE, RESUME }

    private var otherSinceMs: Long? = null

    /**
     * The app in front is [foreground] (null when unknown) at [nowMs];
     * [paused] says whether things are paused now. Returns what to do, if
     * anything.
     */
    fun decide(foreground: String?, apps: Set<String>, paused: Boolean, nowMs: Long): Action? {
        if (foreground == null || foreground in neutral) return null
        if (foreground in apps) {
            otherSinceMs = null
            return if (paused) null else Action.PAUSE
        }
        if (!paused) {
            otherSinceMs = null
            return null
        }
        val since = otherSinceMs ?: nowMs.also { otherSinceMs = it }
        if (nowMs - since < resumeAfterMs) return null
        otherSinceMs = null
        return Action.RESUME
    }

    companion object {
        const val RESUME_AFTER_MS = 3_000L

        /** Reported when the phone is locked or its screen is off, so a pause ends after [RESUME_AFTER_MS]. */
        const val LOCKED_PACKAGE = "com.android.keyguard"

        /** Shown over any app: the system UI and permission and install prompts. */
        val NEUTRAL_PACKAGES = setOf(
            "android",
            "com.android.systemui",
            "com.android.permissioncontroller",
            "com.google.android.permissioncontroller",
            "com.android.packageinstaller",
            "com.google.android.packageinstaller",
        )
    }
}
