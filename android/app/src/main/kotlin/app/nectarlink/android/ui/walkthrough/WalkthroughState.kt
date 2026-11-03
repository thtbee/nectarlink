// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.walkthrough

/**
 * The 5 steps of the first-run setup walkthrough.
 */
enum class WalkthroughStep(val index: Int, val title: String) {
    Welcome(0, "Welcome"),
    Permissions(1, "Permissions"),
    Pair(2, "Pair with your PC"),
    PowerLevel(3, "Choose your power level"),
    Done(4, "Setup complete"),
    ;

    val next: WalkthroughStep?
        get() = entries.getOrNull(index + 1)

    val previous: WalkthroughStep?
        get() = entries.getOrNull(index - 1)

    companion object {
        fun fromIndex(index: Int): WalkthroughStep =
            entries.firstOrNull { it.index == index.coerceIn(0, entries.lastIndex) } ?: Welcome
    }
}

/**
 * Persisted walkthrough flags stored in shared preferences.
 */
@androidx.compose.runtime.Immutable
data class WalkthroughPersistence(
    val completed: Boolean = false,
    val currentStepIndex: Int = 0,
    val whatsNewGuideDismissed: Boolean = false,
) {
    val currentStep: WalkthroughStep
        get() = WalkthroughStep.fromIndex(currentStepIndex)

    fun advanceStep(): WalkthroughPersistence {
        val nextStep = currentStep.next
        return if (nextStep != null) {
            copy(currentStepIndex = nextStep.index)
        } else {
            complete()
        }
    }

    fun previousStep(): WalkthroughPersistence {
        val prevStep = currentStep.previous ?: return this
        return copy(currentStepIndex = prevStep.index)
    }

    fun goToStep(step: WalkthroughStep): WalkthroughPersistence =
        copy(currentStepIndex = step.index)

    fun complete(): WalkthroughPersistence = copy(
        completed = true,
        currentStepIndex = 0,
        whatsNewGuideDismissed = true,
    )

    fun skip(): WalkthroughPersistence = copy(
        completed = true,
        currentStepIndex = 0,
        whatsNewGuideDismissed = true,
    )

    fun reopen(): WalkthroughPersistence = copy(
        completed = false,
        currentStepIndex = WalkthroughStep.Welcome.index,
    )

    fun dismissWhatsNewGuide(): WalkthroughPersistence = copy(
        whatsNewGuideDismissed = true,
    )
}

/**
 * Startup decision for whether to show the walkthrough or go straight to Home.
 */
@androidx.compose.runtime.Immutable
data class StartupWalkthroughResolution(
    val showWalkthrough: Boolean,
    val initialStep: WalkthroughStep,
    val markCompletedAutomatically: Boolean,
    val showWhatsNewGuideBanner: Boolean,
)

/**
 * Resolves walkthrough behavior once core state is ready:
 * - Fresh install (`!completed && step == 0 && !hasPairedDevices`) opens Walkthrough at Welcome.
 * - Mid-walkthrough (`!completed && step > 0`) resumes at `currentStep` even after pairing a PC in Step 2.
 * - Existing paired user upgrading (`!completed && step == 0 && hasPairedDevices`) skips forced walkthrough,
 *   marks `completed = true` automatically, and shows a dismissible Features guide card on Home.
 * - Completed user (`completed == true`) stays on Home.
 */
fun resolveStartupWalkthrough(
    persistence: WalkthroughPersistence,
    hasPairedDevices: Boolean,
    walkthroughAlreadyActive: Boolean = false,
): StartupWalkthroughResolution {
    if (persistence.completed) {
        return StartupWalkthroughResolution(
            showWalkthrough = false,
            initialStep = WalkthroughStep.Welcome,
            markCompletedAutomatically = false,
            showWhatsNewGuideBanner = !persistence.whatsNewGuideDismissed && hasPairedDevices,
        )
    }
    if (persistence.currentStepIndex > 0 || walkthroughAlreadyActive) {
        return StartupWalkthroughResolution(
            showWalkthrough = true,
            initialStep = persistence.currentStep,
            markCompletedAutomatically = false,
            showWhatsNewGuideBanner = false,
        )
    }
    if (hasPairedDevices) {
        return StartupWalkthroughResolution(
            showWalkthrough = false,
            initialStep = WalkthroughStep.Welcome,
            markCompletedAutomatically = true,
            showWhatsNewGuideBanner = !persistence.whatsNewGuideDismissed,
        )
    }
    return StartupWalkthroughResolution(
        showWalkthrough = true,
        initialStep = WalkthroughStep.Welcome,
        markCompletedAutomatically = false,
        showWhatsNewGuideBanner = false,
    )
}
