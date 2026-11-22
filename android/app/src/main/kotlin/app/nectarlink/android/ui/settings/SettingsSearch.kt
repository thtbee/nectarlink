// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import androidx.annotation.StringRes
import app.nectarlink.android.R

/** A setting search can find (#522): the section it's in and its title. */
internal data class SettingsSearchEntry(val section: String, @StringRes val title: Int)

/**
 * Every setting's title, by section. Built from the titles SettingsScreen
 * shows; a new row's title belongs here too.
 */
internal val SETTINGS_SEARCH_INDEX: List<SettingsSearchEntry> = listOf(
    SettingsSearchEntry("appearance", R.string.settings_appearance),
    SettingsSearchEntry("appearance", R.string.settings_theme),
    SettingsSearchEntry("appearance", R.string.theme_hive),
    SettingsSearchEntry("appearance", R.string.theme_bloom),
    SettingsSearchEntry("appearance", R.string.theme_graphite),
    SettingsSearchEntry("appearance", R.string.settings_mode),
    SettingsSearchEntry("appearance", R.string.settings_language),
    SettingsSearchEntry("appearance", R.string.settings_comb_motion),
    SettingsSearchEntry("appearance", R.string.settings_amoled),
    SettingsSearchEntry("appearance", R.string.settings_theme_artwork_color),
    SettingsSearchEntry("appearance", R.string.settings_dynamic_color),
    SettingsSearchEntry("appearance", R.string.settings_home_card_color),
    SettingsSearchEntry("devices", R.string.settings_devices),
    SettingsSearchEntry("devices", R.string.storage_pc_toggle),
    SettingsSearchEntry("devices", R.string.mic_pc_toggle),
    SettingsSearchEntry("devices", R.string.unlock_pc_toggle),
    SettingsSearchEntry("devices", R.string.start_without_asking_toggle),
    SettingsSearchEntry("devices", R.string.mirror_ask_toggle),
    SettingsSearchEntry("devices", R.string.clip_share_toggle),
    SettingsSearchEntry("devices", R.string.clip_accept_toggle),
    SettingsSearchEntry("devices", R.string.guide_away_title),
    SettingsSearchEntry("devices", R.string.relay_settings_title),
    SettingsSearchEntry("notifications", R.string.settings_notifications),
    SettingsSearchEntry("notifications", R.string.settings_quiet_while_using_pc),
    SettingsSearchEntry("notifications", R.string.notifications_title),
    SettingsSearchEntry("control", R.string.settings_control),
    SettingsSearchEntry("control", R.string.dnd_access_title),
    SettingsSearchEntry("control", R.string.elevated_title),
    SettingsSearchEntry("control", R.string.elevated_start_with),
    SettingsSearchEntry("control", R.string.elevated_method_wireless),
    SettingsSearchEntry("control", R.string.elevated_method_shizuku),
    SettingsSearchEntry("control", R.string.elevated_method_root),
    SettingsSearchEntry("control", R.string.write_settings_title),
    SettingsSearchEntry("control", R.string.settings_assist_on_demand),
    SettingsSearchEntry("control", R.string.control_title),
    SettingsSearchEntry("background", R.string.settings_background_title),
    SettingsSearchEntry("background", R.string.settings_run_in_background),
    SettingsSearchEntry("background", R.string.settings_hide_from_recents),
    SettingsSearchEntry("background", R.string.companion_wake_title),
    SettingsSearchEntry("background", R.string.companion_wake_associate),
    SettingsSearchEntry("background", R.string.background_title),
    SettingsSearchEntry("background", R.string.background_text),
    SettingsSearchEntry("background", R.string.battery_open_oem_settings),
    SettingsSearchEntry("calls_contacts", R.string.settings_calls_and_contacts),
    SettingsSearchEntry("calls_contacts", R.string.calls_title),
    SettingsSearchEntry("calls_contacts", R.string.contacts_title),
    SettingsSearchEntry("calls_contacts", R.string.sms_title),
    SettingsSearchEntry("photos", R.string.settings_photos),
    SettingsSearchEntry("photos", R.string.storage_all_files_title),
    SettingsSearchEntry("photos", R.string.storage_saf_title),
    SettingsSearchEntry("photos", R.string.storage_add_folder),
    SettingsSearchEntry("photos", R.string.photos_title),
    SettingsSearchEntry("clipboard", R.string.settings_clipboard_title),
    SettingsSearchEntry("clipboard", R.string.settings_background_clipboard_title),
    SettingsSearchEntry("clipboard", R.string.settings_clipboard_passwords),
    SettingsSearchEntry("clipboard", R.string.settings_clipboard_suggestions),
    SettingsSearchEntry("clipboard", R.string.settings_clipboard_screenshots),
    SettingsSearchEntry("remote", R.string.settings_remote),
    SettingsSearchEntry("remote", R.string.settings_touchpad_sensitivity),
    SettingsSearchEntry("remote", R.string.settings_volume_keys_control_pc),
    SettingsSearchEntry("remote", R.string.settings_pc_sound_buffer),
    SettingsSearchEntry("localsend", R.string.settings_localsend_title),
    SettingsSearchEntry("localsend", R.string.settings_localsend_subtitle),
    SettingsSearchEntry("data_storage", R.string.settings_data_storage_title),
    SettingsSearchEntry("data_storage", R.string.data_notifications_title),
    SettingsSearchEntry("data_storage", R.string.data_notifications_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_clipboard_title),
    SettingsSearchEntry("data_storage", R.string.data_clipboard_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_timeline_title),
    SettingsSearchEntry("data_storage", R.string.data_timeline_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_messages_title),
    SettingsSearchEntry("data_storage", R.string.data_messages_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_photos_title),
    SettingsSearchEntry("data_storage", R.string.data_photos_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_received_files_title),
    SettingsSearchEntry("data_storage", R.string.data_received_files_subtitle),
    SettingsSearchEntry("data_storage", R.string.data_clear_everything_button),
    SettingsSearchEntry("about", R.string.settings_about),
    SettingsSearchEntry("about", R.string.guide_title),
    SettingsSearchEntry("support", R.string.community_title),
    SettingsSearchEntry("support", R.string.community_discord_title),
    SettingsSearchEntry("support", R.string.community_x_title),
    SettingsSearchEntry("support", R.string.community_github_title),
    SettingsSearchEntry("support", R.string.support_title),
    SettingsSearchEntry("support", R.string.report_title),
)

/** Which category chip shows [section], and its place in that category's list. */
internal fun settingsSectionPlace(section: String): Pair<String, Int> = when (section) {
    "appearance" -> "appearance" to 1
    "devices" -> "devices" to 1
    "notifications" -> "permissions" to 1
    "control" -> "permissions" to 2
    "background" -> "permissions" to 3
    "calls_contacts" -> "permissions" to 4
    "photos" -> "permissions" to 5
    "clipboard" -> "features" to 1
    "remote" -> "features" to 2
    "localsend" -> "features" to 3
    "data_storage" -> "features" to 4
    "about" -> "features" to 5
    "support" -> "features" to 6
    else -> "all" to 0
}

/**
 * The entries whose title, or section's title, has every word of [query]
 * (any case), each title once, at most [limit].
 */
internal fun searchSettings(
    query: String,
    entries: List<SettingsSearchEntry>,
    sectionTitle: (String) -> String,
    label: (Int) -> String,
    limit: Int = 8,
): List<SettingsSearchEntry> {
    val words = query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (words.isEmpty()) return emptyList()
    val seen = HashSet<String>()
    return entries.filter { entry ->
        val title = label(entry.title)
        val text = (title + " " + sectionTitle(entry.section)).lowercase()
        words.all { it in text } && seen.add(title)
    }.take(limit)
}
