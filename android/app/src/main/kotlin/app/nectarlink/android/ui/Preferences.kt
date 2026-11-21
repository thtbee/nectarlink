// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import androidx.core.content.edit
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.Tokens
import app.nectarlink.android.ui.walkthrough.WalkthroughPersistence
import app.nectarlink.android.ui.walkthrough.WalkthroughStep
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The app's look, saved in shared preferences and observable. */
@androidx.compose.runtime.Stable
class Preferences(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("appearance", Context.MODE_PRIVATE)
    private val _appearance = MutableStateFlow(load())
    val appearance: StateFlow<Appearance> = _appearance.asStateFlow()

    private val _touchpadSensitivity = MutableStateFlow(prefs.getFloat(TOUCHPAD_SENSITIVITY, 1.0f).coerceIn(0.4f, 3.0f))
    val touchpadSensitivity: StateFlow<Float> = _touchpadSensitivity.asStateFlow()

    private val _suggestClipboardActions = MutableStateFlow(prefs.getBoolean(SUGGEST_CLIPBOARD_ACTIONS, true))
    val suggestClipboardActions: StateFlow<Boolean> = _suggestClipboardActions.asStateFlow()

    private val _volumeKeysControlPc = MutableStateFlow(prefs.getBoolean(VOLUME_KEYS_CONTROL_PC, true))
    val volumeKeysControlPc: StateFlow<Boolean> = _volumeKeysControlPc.asStateFlow()

    private val _pcSoundBufferMs = MutableStateFlow(pcSoundBufferMs(context))
    val pcSoundBufferMs: StateFlow<Int> = _pcSoundBufferMs.asStateFlow()

    private val _quietWhileUsingPc = MutableStateFlow(prefs.getBoolean(QUIET_WHILE_USING_PC, false))
    val quietWhileUsingPc: StateFlow<Boolean> = _quietWhileUsingPc.asStateFlow()

    private val _hideFromRecents = MutableStateFlow(prefs.getBoolean(HIDE_FROM_RECENTS, false))
    val hideFromRecents: StateFlow<Boolean> = _hideFromRecents.asStateFlow()

    private val _backgroundServiceEnabled = MutableStateFlow(prefs.getBoolean(BACKGROUND_SERVICE_ENABLED, true))
    val backgroundServiceEnabled: StateFlow<Boolean> = _backgroundServiceEnabled.asStateFlow()

    private val preferenceChangeListener =
        android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == BACKGROUND_SERVICE_ENABLED) {
                _backgroundServiceEnabled.value = prefs.getBoolean(BACKGROUND_SERVICE_ENABLED, true)
            }
        }

    init {
        prefs.registerOnSharedPreferenceChangeListener(preferenceChangeListener)
    }

    /**
     * The invitation to try Hive shows until it's answered, and again after
     * an update (a reinstall clears this anyway): it remembers the version
     * that answered it.
     */
    private val _hiveInviteShown = MutableStateFlow(
        prefs.getLong(HIVE_INVITE_ANSWERED, -1L) != app.nectarlink.android.BuildConfig.VERSION_CODE.toLong(),
    )
    val hiveInviteShown: StateFlow<Boolean> = _hiveInviteShown.asStateFlow()

    fun answerHiveInvite() {
        prefs.edit { putLong(HIVE_INVITE_ANSWERED, app.nectarlink.android.BuildConfig.VERSION_CODE.toLong()) }
        _hiveInviteShown.value = false
    }

    private val _walkthrough = MutableStateFlow(loadWalkthrough())
    val walkthrough: StateFlow<WalkthroughPersistence> = _walkthrough.asStateFlow()

    private fun load(): Appearance {
        val defaults = Appearance()
        return sanitizeAppearance(
            theme = prefs.getString(THEME, defaults.theme),
            mode = prefs.getString(MODE, defaults.mode),
            seed = prefs.getString(SEED, defaults.seed),
            dynamicColor = prefs.getBoolean(DYNAMIC, defaults.dynamicColor),
            amoled = prefs.getBoolean(AMOLED, defaults.amoled),
            combMotion = prefs.getBoolean(COMB_MOTION, defaults.combMotion),
        )
    }

    private fun loadWalkthrough(): WalkthroughPersistence {
        val completed = prefs.getBoolean(WALKTHROUGH_COMPLETED, false)
        val rawStep = prefs.getInt(WALKTHROUGH_STEP, 0)
        val clampedStep = rawStep.coerceIn(0, WalkthroughStep.entries.lastIndex)
        val dismissed = prefs.getBoolean(WHATS_NEW_GUIDE_DISMISSED, false)
        return WalkthroughPersistence(
            completed = completed,
            currentStepIndex = clampedStep,
            whatsNewGuideDismissed = dismissed,
        )
    }

    fun update(change: (Appearance) -> Appearance) {
        val next = sanitizeAppearance(change(_appearance.value))
        prefs.edit {
            putString(THEME, next.theme)
            putString(MODE, next.mode)
            putString(SEED, next.seed)
            putBoolean(DYNAMIC, next.dynamicColor)
            putBoolean(AMOLED, next.amoled)
            putBoolean(COMB_MOTION, next.combMotion)
        }
        _appearance.value = next
    }

    fun updateTouchpadSensitivity(sensitivity: Float) {
        val clamped = sensitivity.coerceIn(0.4f, 3.0f)
        prefs.edit { putFloat(TOUCHPAD_SENSITIVITY, clamped) }
        _touchpadSensitivity.value = clamped
    }

    fun updateSuggestClipboardActions(enabled: Boolean) {
        prefs.edit { putBoolean(SUGGEST_CLIPBOARD_ACTIONS, enabled) }
        _suggestClipboardActions.value = enabled
        if (!enabled) {
            app.nectarlink.android.links.LinkNotifications.dismissClipSuggestion(appContext)
        }
    }

    fun updateVolumeKeysControlPc(enabled: Boolean) {
        prefs.edit { putBoolean(VOLUME_KEYS_CONTROL_PC, enabled) }
        _volumeKeysControlPc.value = enabled
        (appContext as app.nectarlink.android.NectarlinkApplication).core.pcMedia.volumeKeys(enabled)
    }

    fun updatePcSoundBufferMs(ms: Int) {
        val value = ms.coerceIn(MIN_SOUND_BUFFER_MS, MAX_SOUND_BUFFER_MS)
        prefs.edit { putInt(PC_SOUND_BUFFER_MS, value) }
        _pcSoundBufferMs.value = value
        (appContext as app.nectarlink.android.NectarlinkApplication).core.setPcSoundBufferMs(value)
    }

    fun updateQuietWhileUsingPc(enabled: Boolean) {
        prefs.edit { putBoolean(QUIET_WHILE_USING_PC, enabled) }
        _quietWhileUsingPc.value = enabled
        app.nectarlink.android.notifications.NotificationListener.refreshHostHints()
    }

    fun updateHideFromRecents(hide: Boolean) {
        prefs.edit { putBoolean(HIDE_FROM_RECENTS, hide) }
        _hideFromRecents.value = hide
        applyHideFromRecents(appContext, hide)
    }

    fun updateBackgroundServiceEnabled(enabled: Boolean) {
        prefs.edit { putBoolean(BACKGROUND_SERVICE_ENABLED, enabled) }
        _backgroundServiceEnabled.value = enabled
    }

    fun updateWalkthrough(change: (WalkthroughPersistence) -> WalkthroughPersistence) {
        val current = _walkthrough.value
        val next = change(current).let {
            it.copy(currentStepIndex = it.currentStepIndex.coerceIn(0, WalkthroughStep.entries.lastIndex))
        }
        if (next == current) return
        prefs.edit {
            putBoolean(WALKTHROUGH_COMPLETED, next.completed)
            putInt(WALKTHROUGH_STEP, next.currentStepIndex)
            putBoolean(WHATS_NEW_GUIDE_DISMISSED, next.whatsNewGuideDismissed)
        }
        _walkthrough.value = next
    }

    private val appContext = context.applicationContext

    companion object {
        private const val THEME = "theme"
        private const val MODE = "mode"
        private const val SEED = "seed"
        private const val DYNAMIC = "dynamic_color"
        private const val AMOLED = "amoled_black"
        private const val COMB_MOTION = "comb_motion"
        private const val HIVE_INVITE_ANSWERED = "hive_invite_answered_version"
        private const val TOUCHPAD_SENSITIVITY = "touchpad_sensitivity"
        private const val SUGGEST_CLIPBOARD_ACTIONS = "suggest_clipboard_actions"
        private const val VOLUME_KEYS_CONTROL_PC = "volume_keys_control_pc"
        private const val QUIET_WHILE_USING_PC = "quiet_while_using_pc"
        private const val PC_SOUND_BUFFER_MS = "pc_sound_buffer_ms"
        private const val MIN_SOUND_BUFFER_MS = 40
        private const val MAX_SOUND_BUFFER_MS = 500
        private const val HIDE_FROM_RECENTS = "hide_from_recents"
        internal const val BACKGROUND_SERVICE_ENABLED = "background_service_enabled"
        private const val AWAY_MODE = "away_mode"
        private const val WALKTHROUGH_COMPLETED = "walkthrough_completed"
        private const val WALKTHROUGH_STEP = "walkthrough_step"
        private const val WHATS_NEW_GUIDE_DISMISSED = "whats_new_guide_dismissed"

        val VALID_THEMES = setOf("hive", "bloom", "graphite", "glass")
        val VALID_MODES = setOf("system", "light", "dark")

        fun sanitizeAppearance(
            theme: String?,
            mode: String?,
            seed: String?,
            dynamicColor: Boolean = true,
            amoled: Boolean = false,
            combMotion: Boolean = true,
        ): Appearance {
            val defaults = Appearance()
            val safeTheme = theme?.takeIf { it in VALID_THEMES } ?: defaults.theme
            val safeMode = mode?.takeIf { it in VALID_MODES } ?: defaults.mode
            val safeSeed = seed?.takeIf { it == "artwork" || it in Tokens.bloomSeeds } ?: defaults.seed
            return Appearance(
                theme = safeTheme,
                mode = safeMode,
                seed = safeSeed,
                dynamicColor = dynamicColor,
                amoled = amoled,
                combMotion = combMotion,
            )
        }

        fun sanitizeAppearance(appearance: Appearance): Appearance =
            sanitizeAppearance(
                theme = appearance.theme,
                mode = appearance.mode,
                seed = appearance.seed,
                dynamicColor = appearance.dynamicColor,
                amoled = appearance.amoled,
                combMotion = appearance.combMotion,
            )

        fun isSuggestClipboardActionsEnabled(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(SUGGEST_CLIPBOARD_ACTIONS, true)

        /** How much of the PC's sound the phone holds before playing it, in ms (#321). */
        fun pcSoundBufferMs(context: Context): Int =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getInt(PC_SOUND_BUFFER_MS, MIN_SOUND_BUFFER_MS)
                .coerceIn(MIN_SOUND_BUFFER_MS, MAX_SOUND_BUFFER_MS)

        fun isVolumeKeysControlPcEnabled(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(VOLUME_KEYS_CONTROL_PC, true)

        /**
         * Whether the phone silences its own notification sounds and vibration
         * while a paired PC that shows them is in use. Off by default: the
         * phone sounds as it always does (#384, #407).
         */
        fun isQuietWhileUsingPc(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(QUIET_WHILE_USING_PC, false)

        /**
         * Keeps NectarLink's tasks out of the recent apps screen, or puts them
         * back (#409): where clearing recent apps also stops what's running,
         * NectarLink then can't be closed by accident.
         */
        fun applyHideFromRecents(context: Context, hide: Boolean) {
            runCatching {
                context.getSystemService(android.app.ActivityManager::class.java)?.appTasks?.forEach {
                    it.setExcludeFromRecents(hide)
                }
            }
        }

        /** Whether NectarLink keeps out of the recent apps screen (#409). */
        fun isHideFromRecents(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(HIDE_FROM_RECENTS, false)

        fun isAwayModeEnabled(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(AWAY_MODE, false)

        fun setAwayModeEnabled(context: Context, enabled: Boolean) {
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .edit { putBoolean(AWAY_MODE, enabled) }
        }

        fun isBackgroundServiceEnabled(context: Context): Boolean =
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .getBoolean(BACKGROUND_SERVICE_ENABLED, true)

        fun setBackgroundServiceEnabled(context: Context, enabled: Boolean) {
            context.applicationContext
                .getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .edit { putBoolean(BACKGROUND_SERVICE_ENABLED, enabled) }
        }
    }
}

