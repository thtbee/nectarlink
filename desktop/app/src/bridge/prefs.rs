// SPDX-License-Identifier: GPL-3.0-or-later
//! `Preferences` and `DeckController`: the app's look, behavior and PC-owned Deck.

#![allow(clippy::too_many_arguments)]

use std::{
    path::PathBuf,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};

use crate::{
    core_host,
    settings::{ColorMode, RecordingFormat, Settings, Theme},
};

pub use super::deck::DeckControllerRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// "bloom" or "graphite".
        #[qproperty(QString, theme)]
        /// Bloom colors: "wallpaper", or a preset: "honey", "sage", "lavender"
        /// or "ocean".
        #[qproperty(QString, seed)]
        /// "system", "light" or "dark".
        #[qproperty(QString, color_mode)]
        /// Pure #000000 OLED black surfaces in dark mode.
        #[qproperty(bool, amoled)]
        /// Mica behind the window.
        #[qproperty(bool, backdrop)]
        /// Hive's comb breathes and pulses.
        #[qproperty(bool, comb_motion)]
        #[qproperty(bool, close_to_tray)]
        /// Send what's copied on this PC to connected phones.
        #[qproperty(bool, auto_clipboard)]
        /// Keep the last 50 synced clips in encrypted local history.
        #[qproperty(bool, clipboard_history)]
        /// Suggest context chips (Open, Open in Maps, Call, Track, Email) for copied text.
        #[qproperty(bool, suggest_clipboard_actions)]
        /// Copy one-time codes from phone notifications and SMS automatically.
        #[qproperty(bool, auto_copy_otp)]
        /// Pause PC media playback while a phone call is ringing or active.
        #[qproperty(bool, pause_media_on_call)]
        /// Quiet phone notification pop-ups on the PC while the phone is in Do Not Disturb.
        #[qproperty(bool, sync_dnd)]
        /// Paired phones in Explorer's "Send to" menu.
        #[qproperty(bool, send_to_menu)]
        /// Start when the user signs in.
        #[qproperty(bool, start_with_windows)]
        /// Check for updates on their own.
        #[qproperty(bool, auto_update)]
        /// Install a newer release by itself at a quiet moment.
        #[qproperty(bool, install_updates)]
        /// Tell when a phone's battery is low, or full.
        #[qproperty(bool, battery_alerts)]
        /// Folder where voice recordings from phones are saved.
        #[qproperty(QString, recordings_folder)]
        /// Whether `recordings_folder` is the default (`Documents\Nectarlink Recordings`).
        #[qproperty(bool, recordings_folder_is_default)]
        /// "m4a", "mp3", "wav" or "flac".
        #[qproperty(QString, recordings_format)]
        /// Timeline retention in days (`0` means keep up to 5,000 entries regardless of age).
        #[qproperty(i32, timeline_retention_days)]
        /// Global hotkey for "Take photo with phone" (e.g. `"Ctrl+Alt+C"`).
        #[qproperty(QString, continuity_photo_hotkey)]
        /// Global hotkey for "Scan document with phone" (e.g. `"Ctrl+Alt+D"`).
        #[qproperty(QString, continuity_scan_hotkey)]
        /// Global hotkey for the Command Palette (e.g. `"Ctrl+Alt+Space"`).
        #[qproperty(QString, command_palette_hotkey)]
        /// Global hotkey for the Shelf panel (e.g. `"Ctrl+Alt+S"`).
        #[qproperty(QString, shelf_hotkey)]
        /// Opt-in localhost HTTP/JSON + SSE automation API (`127.0.0.1`).
        #[qproperty(bool, local_api_enabled)]
        /// TCP port for the localhost automation API (default `53320`).
        #[qproperty(i32, local_api_port)]
        /// Path to `<data_dir>/api-token.json` containing the bearer token.
        #[qproperty(QString, local_api_token_path)]
        /// Copy received files (<= 10) to the Windows clipboard as CF_HDROP.
        #[qproperty(bool, clipboard_auto_copy_received_files)]
        /// Open a single file shared from the phone when it arrives.
        #[qproperty(bool, open_shared_files)]
        /// Send the clipboard to the phone when control moves to it.
        #[qproperty(bool, flow_follow_clipboard)]
        /// Floating Nectar Island pill at the top center of the screen.
        #[qproperty(bool, nectar_island)]
        /// Trigger Nectar Island for phone calls.
        #[qproperty(bool, nectar_island_calls)]
        /// Trigger Nectar Island for file transfers.
        #[qproperty(bool, nectar_island_transfers)]
        /// Trigger Nectar Island for media playback.
        #[qproperty(bool, nectar_island_media)]
        /// Trigger Nectar Island for phone notifications.
        #[qproperty(bool, nectar_island_notifications)]
        /// Stream PC system audio to the connected phone via WASAPI loopback.
        #[qproperty(bool, stream_pc_audio_to_phone)]
        /// Share PC user folders (Downloads, Documents, Desktop, Pictures) with phones.
        #[qproperty(bool, share_pc_storage)]
        /// Forward PC notifications to connected phones (opt-in privacy toggle).
        #[qproperty(bool, forward_pc_notifications_to_phone)]
        /// Stay connected over mobile data (LTE/5G) or relay when away from Wi-Fi.
        #[qproperty(bool, away_mode)]
        /// Alias for `start_with_windows` used by the walkthrough and settings bindings.
        #[qproperty(bool, start_at_login)]
        /// Whether the first-run setup walkthrough has been completed or skipped.
        #[qproperty(bool, walkthrough_completed)]
        /// Current step index (`0..=6`) in the first-run setup walkthrough.
        #[qproperty(i32, walkthrough_step)]
        /// Whether the gentle "What's new" card on Home has been dismissed.
        #[qproperty(bool, whats_new_dismissed)]
        /// Whether the card on Home inviting to try Hive has been answered.
        #[qproperty(bool, hive_invite_dismissed)]
        /// JSON array of configured `FolderSyncRule` items.
        #[qproperty(QString, folder_sync_rules_json)]
        /// JSON array of configured `AutomationRule` items.
        #[qproperty(QString, automation_rules_json)]
        /// Whether a saved main window `(x, y)` position exists.
        #[qproperty(bool, has_window_position)]
        /// Saved main window X position in screen pixels.
        #[qproperty(i32, window_x)]
        /// Saved main window Y position in screen pixels.
        #[qproperty(i32, window_y)]
        /// Saved main window normal width in logical pixels.
        #[qproperty(i32, window_width)]
        /// Saved main window normal height in logical pixels.
        #[qproperty(i32, window_height)]
        /// Whether the main window was maximized when last open.
        #[qproperty(bool, window_maximized)]
        type Preferences = super::PreferencesRust;

        /// Resets the setup walkthrough to step 0 and opens it again.
        #[qinvokable]
        fn restart_walkthrough(self: Pin<&mut Preferences>);
        /// Saves the main window's normal position, dimensions, and maximized state.
        #[qinvokable]
        fn save_window_geometry(
            self: Pin<&mut Preferences>,
            has_position: bool,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            maximized: bool,
        );
        /// OBS Studio's WebSocket port, for `obs:` Deck commands.
        #[qinvokable]
        fn obs_port(self: &Preferences) -> i32;
        /// Whether an OBS password is saved.
        #[qinvokable]
        fn obs_has_password(self: &Preferences) -> bool;
        /// Saves the OBS port and, when `replace_password` is set, the password (empty clears it).
        #[qinvokable]
        fn save_obs(self: Pin<&mut Preferences>, port: i32, password: &QString, replace_password: bool);
        /// Connects to OBS with the saved settings and shows the result as a message.
        #[qinvokable]
        fn test_obs(self: &Preferences);

        /// Sets the folder where voice recordings are saved (from a `file:` URL or path).
        #[qinvokable]
        fn choose_recordings_folder(self: Pin<&mut Preferences>, url: &QString);
        /// Resets the recordings folder to `Documents\Nectarlink Recordings`.
        #[qinvokable]
        fn reset_recordings_folder(self: Pin<&mut Preferences>);
        /// Adds a folder sync rule between a phone folder and a PC folder.
        #[qinvokable]
        fn add_folder_sync_rule(
            self: Pin<&mut Preferences>,
            device_id: &QString,
            phone_folder: &QString,
            pc_folder: &QString,
            two_way: bool,
        );
        /// Removes a folder sync rule by ID.
        #[qinvokable]
        fn remove_folder_sync_rule(self: Pin<&mut Preferences>, rule_id: &QString);
        /// Enables or disables a folder sync rule by ID.
        #[qinvokable]
        fn set_folder_sync_rule_enabled(self: Pin<&mut Preferences>, rule_id: &QString, enabled: bool);
        /// Runs all enabled folder sync rules immediately.
        #[qinvokable]
        fn sync_folders_now(self: Pin<&mut Preferences>);
        /// Adds a trigger-action automation rule.
        #[qinvokable]
        fn add_automation_rule(
            self: Pin<&mut Preferences>,
            trigger: &QString,
            action: &QString,
            param: &QString,
        );
        /// Removes an automation rule by ID.
        #[qinvokable]
        fn remove_automation_rule(self: Pin<&mut Preferences>, rule_id: &QString);
        /// Enables or disables an automation rule by ID.
        #[qinvokable]
        fn set_automation_rule_enabled(self: Pin<&mut Preferences>, rule_id: &QString, enabled: bool);

        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, pages_json)]
        #[qproperty(i32, volume)]
        #[qproperty(bool, muted)]
        /// `-1`: no mic, `0`: live, `1`: muted.
        #[qproperty(i32, mic_state)]
        #[qproperty(bool, playing)]
        type DeckController = super::DeckControllerRust;

        #[qinvokable]
        fn default_label_for(self: &DeckController, kind: &QString) -> QString;
        #[qinvokable]
        fn default_icon_for(self: &DeckController, kind: &QString) -> QString;
        #[qinvokable]
        fn default_color_for(self: &DeckController, kind: &QString) -> QString;
        #[qinvokable]
        fn add_page(self: Pin<&mut DeckController>, name: &QString) -> QString;
        #[qinvokable]
        fn rename_page(self: Pin<&mut DeckController>, page_id: &QString, name: &QString);
        #[qinvokable]
        fn remove_page(self: Pin<&mut DeckController>, page_id: &QString);
        #[qinvokable]
        fn reset_default(self: Pin<&mut DeckController>);
        #[qinvokable]
        fn move_tile(self: Pin<&mut DeckController>, page_id: &QString, from_index: i32, to_index: i32);
        #[qinvokable]
        fn remove_tile(self: Pin<&mut DeckController>, page_id: &QString, tile_id: &QString);
        #[qinvokable]
        fn save_tile(
            self: Pin<&mut DeckController>,
            page_id: &QString,
            tile_id: &QString,
            label: &QString,
            icon: &QString,
            color: &QString,
            kind: &QString,
            param: &QString,
            ctrl: bool,
            alt: bool,
            shift: bool,
            win_mod: bool,
        ) -> QString;
        #[qinvokable]
        fn test_tile(self: Pin<&mut DeckController>, tile_id: &QString);
        #[qinvokable]
        fn url_to_local_path(self: &DeckController, url: &QString) -> QString;
        #[qinvokable]
        fn set_page_active(self: &DeckController, active: bool);
    }

    impl cxx_qt::Threading for Preferences {}
    impl cxx_qt::Initialize for Preferences {}
    impl cxx_qt::Threading for DeckController {}
    impl cxx_qt::Initialize for DeckController {}
}

#[derive(Default)]
pub struct PreferencesRust {
    theme: QString,
    seed: QString,
    color_mode: QString,
    amoled: bool,
    backdrop: bool,
    comb_motion: bool,
    close_to_tray: bool,
    auto_clipboard: bool,
    clipboard_history: bool,
    suggest_clipboard_actions: bool,
    auto_copy_otp: bool,
    pause_media_on_call: bool,
    sync_dnd: bool,
    send_to_menu: bool,
    start_with_windows: bool,
    auto_update: bool,
    install_updates: bool,
    battery_alerts: bool,
    recordings_folder: QString,
    recordings_folder_is_default: bool,
    recordings_format: QString,
    timeline_retention_days: i32,
    continuity_photo_hotkey: QString,
    continuity_scan_hotkey: QString,
    command_palette_hotkey: QString,
    shelf_hotkey: QString,
    local_api_enabled: bool,
    local_api_port: i32,
    local_api_token_path: QString,
    clipboard_auto_copy_received_files: bool,
    open_shared_files: bool,
    flow_follow_clipboard: bool,
    nectar_island: bool,
    nectar_island_calls: bool,
    nectar_island_transfers: bool,
    nectar_island_media: bool,
    nectar_island_notifications: bool,
    stream_pc_audio_to_phone: bool,
    share_pc_storage: bool,
    forward_pc_notifications_to_phone: bool,
    away_mode: bool,
    start_at_login: bool,
    walkthrough_completed: bool,
    walkthrough_step: i32,
    whats_new_dismissed: bool,
    hive_invite_dismissed: bool,
    folder_sync_rules_json: QString,
    automation_rules_json: QString,
    has_window_position: bool,
    window_x: i32,
    window_y: i32,
    window_width: i32,
    window_height: i32,
    window_maximized: bool,
    /// What the user chose (`start_with_windows` shows the default until then).
    start_choice: Option<bool>,
    /// Custom recordings folder if chosen (`None` means default).
    custom_recordings_folder: Option<PathBuf>,
}

impl cxx_qt::Initialize for qobject::Preferences {
    fn initialize(mut self: Pin<&mut Self>) {
        let settings = Settings::load(&core_host::host().data_dir);
        settings.apply_runtime();
        self.as_mut().set_theme(QString::from(match settings.theme {
            Theme::Hive => "hive",
            Theme::Bloom => "bloom",
            Theme::Glass => "glass",
            Theme::Graphite => "graphite",
        }));
        self.as_mut().set_seed(QString::from(&settings.seed));
        self.as_mut().set_color_mode(QString::from(match settings.color_mode {
            ColorMode::System => "system",
            ColorMode::Light => "light",
            ColorMode::Dark => "dark",
        }));
        self.as_mut().set_amoled(settings.amoled);
        self.as_mut().set_backdrop(settings.backdrop);
        self.as_mut().set_comb_motion(settings.comb_motion);
        self.as_mut().set_close_to_tray(settings.close_to_tray);
        self.as_mut().set_auto_clipboard(settings.auto_clipboard);
        crate::clipboard::set_auto_send(settings.auto_clipboard);
        self.as_mut().set_clipboard_history(settings.clipboard_history);
        crate::clipboard::set_history_enabled(settings.clipboard_history);
        self.as_mut().set_suggest_clipboard_actions(settings.suggest_clipboard_actions);
        crate::clipboard::set_suggest_actions(settings.suggest_clipboard_actions);
        self.as_mut().set_auto_copy_otp(settings.auto_copy_otp);
        crate::notifications::set_auto_copy_otp(settings.auto_copy_otp);
        self.as_mut().set_pause_media_on_call(settings.pause_media_on_call);
        crate::calls::set_pause_media_on_call(settings.pause_media_on_call);
        self.as_mut().set_sync_dnd(settings.sync_dnd);
        crate::notifications::set_sync_dnd(settings.sync_dnd);
        self.as_mut().set_send_to_menu(settings.send_to_menu);
        self.as_mut().set_auto_update(settings.auto_update);
        self.as_mut().set_install_updates(settings.install_updates);
        self.as_mut().set_battery_alerts(settings.battery_alerts);
        let start_effective = crate::startup::effective(settings.start_with_windows);
        self.as_mut().set_start_with_windows(start_effective);
        self.as_mut().set_start_at_login(start_effective);
        self.as_mut().rust_mut().start_choice = settings.start_with_windows;
        let effective_folder = crate::recordings::effective_folder(settings.recordings_folder.as_deref());
        self.as_mut().set_recordings_folder(QString::from(&effective_folder.to_string_lossy().into_owned()));
        self.as_mut().set_recordings_folder_is_default(settings.recordings_folder.is_none());
        self.as_mut().rust_mut().custom_recordings_folder = settings.recordings_folder.clone();
        self.as_mut().set_recordings_format(QString::from(settings.recordings_format.as_str()));
        self.as_mut().set_timeline_retention_days(settings.timeline_retention_days as i32);
        self.as_mut().set_continuity_photo_hotkey(QString::from(&settings.continuity_photo_hotkey));
        self.as_mut().set_continuity_scan_hotkey(QString::from(&settings.continuity_scan_hotkey));
        self.as_mut().set_command_palette_hotkey(QString::from(&settings.command_palette_hotkey));
        self.as_mut().set_shelf_hotkey(QString::from(&settings.shelf_hotkey));
        self.as_mut().set_local_api_enabled(settings.local_api_enabled);
        self.as_mut().set_local_api_port(i32::from(settings.local_api_port));
        let token_path = crate::local_api::token_file_path();
        self.as_mut().set_local_api_token_path(QString::from(&token_path.to_string_lossy().into_owned()));
        self.as_mut().set_clipboard_auto_copy_received_files(settings.clipboard_auto_copy_received_files);
        self.as_mut().set_open_shared_files(settings.open_shared_files);
        self.as_mut().set_flow_follow_clipboard(settings.flow_follow_clipboard);
        self.as_mut().set_nectar_island(settings.nectar_island);
        self.as_mut().set_nectar_island_calls(settings.nectar_island_calls);
        self.as_mut().set_nectar_island_transfers(settings.nectar_island_transfers);
        self.as_mut().set_nectar_island_media(settings.nectar_island_media);
        self.as_mut().set_nectar_island_notifications(settings.nectar_island_notifications);
        self.as_mut().set_stream_pc_audio_to_phone(settings.stream_pc_audio_to_phone);
        self.as_mut().set_share_pc_storage(settings.share_pc_storage);
        self.as_mut().set_forward_pc_notifications_to_phone(settings.forward_pc_notifications_to_phone);
        self.as_mut().set_away_mode(settings.away_mode);
        self.as_mut().set_walkthrough_completed(settings.walkthrough_completed);
        self.as_mut().set_walkthrough_step(i32::from(settings.walkthrough_step.min(6)));
        self.as_mut().set_whats_new_dismissed(settings.whats_new_dismissed);
        self.as_mut().set_hive_invite_dismissed(settings.hive_invite_dismissed);
        let folder_rules_json =
            serde_json::to_string(&settings.folder_sync_rules).unwrap_or_else(|_| "[]".into());
        self.as_mut().set_folder_sync_rules_json(QString::from(&folder_rules_json));
        let automation_rules_json =
            serde_json::to_string(&settings.automation_rules).unwrap_or_else(|_| "[]".into());
        self.as_mut().set_automation_rules_json(QString::from(&automation_rules_json));
        self.as_mut().set_has_window_position(settings.window_x.is_some() && settings.window_y.is_some());
        self.as_mut().set_window_x(settings.window_x.unwrap_or(0));
        self.as_mut().set_window_y(settings.window_y.unwrap_or(0));
        self.as_mut().set_window_width(settings.window_width as i32);
        self.as_mut().set_window_height(settings.window_height as i32);
        self.as_mut().set_window_maximized(settings.window_maximized);
        crate::win::tray::update_hotkeys(
            &settings.continuity_photo_hotkey,
            &settings.continuity_scan_hotkey,
            &settings.command_palette_hotkey,
            &settings.shelf_hotkey,
        );
        crate::recordings::init(&settings);

        // Save after any change (connected after loading, so loading doesn't
        // rewrite the file).
        self.as_mut().on_theme_changed(|p| p.save()).release();
        self.as_mut().on_seed_changed(|p| p.save()).release();
        self.as_mut().on_color_mode_changed(|p| p.save()).release();
        self.as_mut().on_amoled_changed(|p| p.save()).release();
        self.as_mut().on_backdrop_changed(|p| p.save()).release();
        self.as_mut().on_comb_motion_changed(|p| p.save()).release();
        self.as_mut().on_close_to_tray_changed(|p| p.save()).release();
        self.as_mut()
            .on_auto_clipboard_changed(|p| {
                crate::clipboard::set_auto_send(p.auto_clipboard);
                p.save();
            })
            .release();
        self.as_mut()
            .on_clipboard_history_changed(|p| {
                crate::clipboard::set_history_enabled(p.clipboard_history);
                p.save();
            })
            .release();
        self.as_mut()
            .on_suggest_clipboard_actions_changed(|p| {
                crate::clipboard::set_suggest_actions(p.suggest_clipboard_actions);
                p.save();
            })
            .release();
        self.as_mut()
            .on_auto_copy_otp_changed(|p| {
                crate::notifications::set_auto_copy_otp(p.auto_copy_otp);
                p.save();
            })
            .release();
        self.as_mut()
            .on_pause_media_on_call_changed(|p| {
                crate::calls::set_pause_media_on_call(p.pause_media_on_call);
                p.save();
            })
            .release();
        self.as_mut()
            .on_sync_dnd_changed(|p| {
                crate::notifications::set_sync_dnd(p.sync_dnd);
                p.save();
            })
            .release();
        self.as_mut()
            .on_send_to_menu_changed(|p| {
                crate::send_to::set_enabled(p.send_to_menu);
                p.save();
            })
            .release();
        self.as_mut()
            .on_auto_update_changed(|p| {
                crate::updater::set_auto(p.auto_update);
                p.save();
            })
            .release();
        self.as_mut()
            .on_install_updates_changed(|p| {
                crate::updater::set_auto_install(p.install_updates);
                p.save();
            })
            .release();
        self.as_mut()
            .on_battery_alerts_changed(|p| {
                crate::battery::set_enabled(p.battery_alerts);
                p.save();
            })
            .release();
        self.as_mut()
            .on_start_with_windows_changed(|mut p| {
                let on = p.start_with_windows;
                if *p.start_at_login() != on {
                    p.as_mut().set_start_at_login(on);
                }
                p.as_mut().rust_mut().start_choice = Some(on);
                crate::startup::apply(on);
                p.save();
            })
            .release();
        self.as_mut()
            .on_start_at_login_changed(|mut p| {
                let on = p.start_at_login;
                if *p.start_with_windows() != on {
                    p.as_mut().set_start_with_windows(on);
                }
            })
            .release();
        self.as_mut()
            .on_recordings_format_changed(|p| {
                let fmt = RecordingFormat::from_str_lossy(&String::from(&p.recordings_format));
                crate::recordings::set_format(fmt);
                p.save();
            })
            .release();
        self.as_mut()
            .on_timeline_retention_days_changed(|p| {
                let days = p.timeline_retention_days.max(0) as u32;
                if let Some(node) = core_host::node() {
                    let _ = node.set_timeline_retention(nectarlink_core::TimelineRetention {
                        max_days: days,
                        max_entries: nectarlink_core::DEFAULT_TIMELINE_MAX_ENTRIES,
                    });
                }
                p.save();
            })
            .release();
        self.as_mut()
            .on_continuity_photo_hotkey_changed(|p| {
                crate::win::tray::update_hotkeys(
                    &String::from(&p.continuity_photo_hotkey),
                    &String::from(&p.continuity_scan_hotkey),
                    &String::from(&p.command_palette_hotkey),
                    &String::from(&p.shelf_hotkey),
                );
                p.save();
            })
            .release();
        self.as_mut()
            .on_continuity_scan_hotkey_changed(|p| {
                crate::win::tray::update_hotkeys(
                    &String::from(&p.continuity_photo_hotkey),
                    &String::from(&p.continuity_scan_hotkey),
                    &String::from(&p.command_palette_hotkey),
                    &String::from(&p.shelf_hotkey),
                );
                p.save();
            })
            .release();
        self.as_mut()
            .on_command_palette_hotkey_changed(|p| {
                crate::win::tray::update_hotkeys(
                    &String::from(&p.continuity_photo_hotkey),
                    &String::from(&p.continuity_scan_hotkey),
                    &String::from(&p.command_palette_hotkey),
                    &String::from(&p.shelf_hotkey),
                );
                p.save();
            })
            .release();
        self.as_mut()
            .on_shelf_hotkey_changed(|p| {
                crate::win::tray::update_hotkeys(
                    &String::from(&p.continuity_photo_hotkey),
                    &String::from(&p.continuity_scan_hotkey),
                    &String::from(&p.command_palette_hotkey),
                    &String::from(&p.shelf_hotkey),
                );
                p.save();
            })
            .release();
        self.as_mut()
            .on_local_api_enabled_changed(|mut p| {
                let token_path = crate::local_api::token_file_path();
                p.as_mut()
                    .set_local_api_token_path(QString::from(&token_path.to_string_lossy().into_owned()));
                let settings = p.current_settings();
                let _ = settings.save(&core_host::host().data_dir);
                crate::local_api::sync_from_settings();
                save_in_background(settings);
            })
            .release();
        self.as_mut()
            .on_local_api_port_changed(|mut p| {
                let token_path = crate::local_api::token_file_path();
                p.as_mut()
                    .set_local_api_token_path(QString::from(&token_path.to_string_lossy().into_owned()));
                let settings = p.current_settings();
                let _ = settings.save(&core_host::host().data_dir);
                crate::local_api::sync_from_settings();
                save_in_background(settings);
            })
            .release();
        self.as_mut()
            .on_clipboard_auto_copy_received_files_changed(|p| {
                crate::settings::set_clipboard_auto_copy_received_files(p.clipboard_auto_copy_received_files);
                p.save();
            })
            .release();
        self.as_mut()
            .on_open_shared_files_changed(|p| {
                crate::settings::set_open_shared_files(p.open_shared_files);
                p.save();
            })
            .release();
        self.as_mut()
            .on_flow_follow_clipboard_changed(|p| {
                crate::settings::set_flow_follow_clipboard(p.flow_follow_clipboard);
                p.save();
            })
            .release();
        self.as_mut()
            .on_nectar_island_changed(|p| {
                crate::settings::set_nectar_island(p.nectar_island);
                p.save();
            })
            .release();
        self.as_mut().on_nectar_island_calls_changed(|p| p.save()).release();
        self.as_mut().on_nectar_island_transfers_changed(|p| p.save()).release();
        self.as_mut().on_nectar_island_media_changed(|p| p.save()).release();
        self.as_mut().on_nectar_island_notifications_changed(|p| p.save()).release();
        self.as_mut()
            .on_stream_pc_audio_to_phone_changed(|p| {
                crate::settings::set_stream_pc_audio_to_phone(p.stream_pc_audio_to_phone);
                p.save();
            })
            .release();
        self.as_mut()
            .on_share_pc_storage_changed(|p| {
                crate::settings::set_share_pc_storage(p.share_pc_storage);
                p.save();
            })
            .release();
        self.as_mut()
            .on_forward_pc_notifications_to_phone_changed(|p| {
                crate::settings::set_forward_pc_notifications_to_phone(p.forward_pc_notifications_to_phone);
                p.save();
            })
            .release();
        self.as_mut()
            .on_away_mode_changed(|p| {
                crate::settings::set_away_mode(p.away_mode);
                p.save();
            })
            .release();
        self.as_mut().on_walkthrough_completed_changed(|p| p.save()).release();
        self.as_mut()
            .on_walkthrough_step_changed(|mut p| {
                let clamped = p.walkthrough_step.clamp(0, 6);
                if p.walkthrough_step != clamped {
                    p.as_mut().set_walkthrough_step(clamped);
                    return;
                }
                p.save();
            })
            .release();
        self.as_mut().on_whats_new_dismissed_changed(|p| p.save()).release();
        self.as_mut().on_hive_invite_dismissed_changed(|p| p.save()).release();

        super::subscribe(self.qt_thread(), crate::state::Changes::PREFS, Self::refresh_runtime_flags);
    }
}

impl qobject::Preferences {
    fn refresh_runtime_flags(mut self: Pin<&mut Self>) {
        let on = crate::pc_audio_stream::is_enabled();
        if *self.stream_pc_audio_to_phone() != on {
            self.as_mut().set_stream_pc_audio_to_phone(on);
        }
        let disk = Settings::load(&core_host::host().data_dir);
        let disk_theme = QString::from(match disk.theme {
            Theme::Hive => "hive",
            Theme::Bloom => "bloom",
            Theme::Glass => "glass",
            Theme::Graphite => "graphite",
        });
        let disk_mode = QString::from(match disk.color_mode {
            ColorMode::System => "system",
            ColorMode::Light => "light",
            ColorMode::Dark => "dark",
        });
        let disk_step = i32::from(disk.walkthrough_step.min(6));
        if *self.theme() != disk_theme {
            self.as_mut().set_theme(disk_theme);
        }
        if *self.color_mode() != disk_mode {
            self.as_mut().set_color_mode(disk_mode);
        }
        if *self.walkthrough_step() != disk_step {
            self.as_mut().set_walkthrough_step(disk_step);
        }
        if *self.walkthrough_completed() != disk.walkthrough_completed {
            self.as_mut().set_walkthrough_completed(disk.walkthrough_completed);
        }
        if *self.whats_new_dismissed() != disk.whats_new_dismissed {
            self.as_mut().set_whats_new_dismissed(disk.whats_new_dismissed);
        }
        let _ = disk.save(&core_host::host().data_dir);
    }

    pub fn restart_walkthrough(mut self: Pin<&mut Self>) {
        self.as_mut().set_walkthrough_step(0);
        self.as_mut().set_walkthrough_completed(false);
        self.save();
    }

    pub fn save_window_geometry(
        mut self: Pin<&mut Self>,
        has_position: bool,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        maximized: bool,
    ) {
        let clamped_w = (width.max(crate::settings::MIN_WINDOW_WIDTH as i32) as u32).min(7680);
        let clamped_h = (height.max(crate::settings::MIN_WINDOW_HEIGHT as i32) as u32).min(4320);
        let (opt_x, opt_y) = if has_position {
            (Some(x), Some(y))
        } else {
            (None, None)
        };
        crate::settings::set_window_geometry(opt_x, opt_y, clamped_w, clamped_h, maximized);
        if *self.has_window_position() != has_position {
            self.as_mut().set_has_window_position(has_position);
        }
        if has_position {
            if *self.window_x() != x {
                self.as_mut().set_window_x(x);
            }
            if *self.window_y() != y {
                self.as_mut().set_window_y(y);
            }
        }
        if *self.window_width() != clamped_w as i32 {
            self.as_mut().set_window_width(clamped_w as i32);
        }
        if *self.window_height() != clamped_h as i32 {
            self.as_mut().set_window_height(clamped_h as i32);
        }
        if *self.window_maximized() != maximized {
            self.as_mut().set_window_maximized(maximized);
        }
        let settings = self.current_settings();
        let _ = settings.save(&core_host::host().data_dir);
    }

    pub fn obs_port(&self) -> i32 {
        i32::from(crate::settings::obs_settings().0)
    }

    pub fn obs_has_password(&self) -> bool {
        crate::settings::obs_settings().1.is_some()
    }

    pub fn save_obs(self: Pin<&mut Self>, port: i32, password: &QString, replace_password: bool) {
        let (current_port, current_password) = crate::settings::obs_settings();
        let port = u16::try_from(port).ok().filter(|&p| p >= 1024).unwrap_or(current_port);
        let protected = if replace_password {
            let password = String::from(password);
            if password.is_empty() { None } else { crate::settings::protect_obs_password(&password) }
        } else {
            current_password
        };
        crate::settings::set_obs_settings(port, protected);
        self.save();
    }

    pub fn test_obs(&self) {
        let spawned = std::thread::Builder::new().name("obs-test".into()).spawn(|| {
            let message = match crate::deck::send_obs_command("GetVersion") {
                Ok(()) => "Connected to OBS.".to_owned(),
                Err(e) => e,
            };
            crate::bridge::app::show_message(message);
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "can't test the OBS connection");
        }
    }

    pub fn choose_recordings_folder(mut self: Pin<&mut Self>, url: &QString) {
        let raw = QUrl::from(url)
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from(url));
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let path = PathBuf::from(trimmed);
        self.as_mut().rust_mut().custom_recordings_folder = Some(path.clone());
        self.as_mut().set_recordings_folder(QString::from(&path.to_string_lossy().into_owned()));
        self.as_mut().set_recordings_folder_is_default(false);
        crate::recordings::set_folder(Some(path));
        self.save();
    }

    pub fn reset_recordings_folder(mut self: Pin<&mut Self>) {
        let default = crate::recordings::default_folder();
        self.as_mut().rust_mut().custom_recordings_folder = None;
        self.as_mut().set_recordings_folder(QString::from(&default.to_string_lossy().into_owned()));
        self.as_mut().set_recordings_folder_is_default(true);
        crate::recordings::set_folder(None);
        self.save();
    }

    pub fn add_folder_sync_rule(
        mut self: Pin<&mut Self>,
        device_id: &QString,
        phone_folder: &QString,
        pc_folder: &QString,
        two_way: bool,
    ) {
        let raw_pc = QUrl::from(pc_folder)
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from(pc_folder));
        let pc_trimmed = raw_pc.trim();
        let phone_trimmed = String::from(phone_folder).trim().to_owned();
        if pc_trimmed.is_empty() || phone_trimmed.is_empty() {
            return;
        }
        let mut rules = crate::settings::folder_sync_rules();
        let next_id = format!("sync-{}", rules.len() + 1);
        rules.push(crate::folder_sync::FolderSyncRule {
            id: next_id,
            phone_id: String::from(device_id).trim().to_owned(),
            phone_folder: phone_trimmed,
            pc_folder: pc_trimmed.to_owned(),
            mode: if two_way { "two_way".into() } else { "one_way_phone_to_pc".into() },
            enabled: true,
        });
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_folder_sync_rules(rules);
        self.as_mut().set_folder_sync_rules_json(QString::from(&json));
        self.save();
    }

    pub fn remove_folder_sync_rule(mut self: Pin<&mut Self>, rule_id: &QString) {
        let id = String::from(rule_id);
        let mut rules = crate::settings::folder_sync_rules();
        rules.retain(|r| r.id != id);
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_folder_sync_rules(rules);
        self.as_mut().set_folder_sync_rules_json(QString::from(&json));
        self.save();
    }

    pub fn set_folder_sync_rule_enabled(mut self: Pin<&mut Self>, rule_id: &QString, enabled: bool) {
        let id = String::from(rule_id);
        let mut rules = crate::settings::folder_sync_rules();
        if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = enabled;
        }
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_folder_sync_rules(rules);
        self.as_mut().set_folder_sync_rules_json(QString::from(&json));
        self.save();
    }

    pub fn sync_folders_now(self: Pin<&mut Self>) {
        let _ = self;
        crate::folder_sync::trigger_sync_now();
    }

    pub fn add_automation_rule(
        mut self: Pin<&mut Self>,
        trigger: &QString,
        action: &QString,
        param: &QString,
    ) {
        let trig = String::from(trigger).trim().to_owned();
        let act = String::from(action).trim().to_owned();
        if trig.is_empty() || act.is_empty() {
            return;
        }
        let mut rules = crate::settings::automation_rules();
        let next_id = format!("auto-{}", rules.len() + 1);
        rules.push(crate::automations::AutomationRule {
            id: next_id,
            name: format!("{trig} → {act}"),
            enabled: true,
            trigger: trig,
            action_kind: act,
            action_arg: String::from(param).trim().to_owned(),
        });
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_automation_rules(rules);
        self.as_mut().set_automation_rules_json(QString::from(&json));
        self.save();
    }

    pub fn remove_automation_rule(mut self: Pin<&mut Self>, rule_id: &QString) {
        let id = String::from(rule_id);
        let mut rules = crate::settings::automation_rules();
        rules.retain(|r| r.id != id);
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_automation_rules(rules);
        self.as_mut().set_automation_rules_json(QString::from(&json));
        self.save();
    }

    pub fn set_automation_rule_enabled(mut self: Pin<&mut Self>, rule_id: &QString, enabled: bool) {
        let id = String::from(rule_id);
        let mut rules = crate::settings::automation_rules();
        if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = enabled;
        }
        let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        crate::settings::set_automation_rules(rules);
        self.as_mut().set_automation_rules_json(QString::from(&json));
        self.save();
    }

    fn current_settings(&self) -> Settings {
        let p = self.rust();
        let (win_x, win_y, win_w, win_h, win_max) = crate::settings::window_geometry();
        Settings {
            theme: match String::from(&p.theme).as_str() {
                "hive" => Theme::Hive,
                "graphite" => Theme::Graphite,
                "glass" | "blurry" => Theme::Glass,
                _ => Theme::Bloom,
            },
            seed: String::from(&p.seed),
            color_mode: match String::from(&p.color_mode).as_str() {
                "light" => ColorMode::Light,
                "dark" => ColorMode::Dark,
                _ => ColorMode::System,
            },
            amoled: p.amoled,
            backdrop: p.backdrop,
            comb_motion: p.comb_motion,
            close_to_tray: p.close_to_tray,
            auto_clipboard: p.auto_clipboard,
            clipboard_history: p.clipboard_history,
            suggest_clipboard_actions: p.suggest_clipboard_actions,
            auto_copy_otp: p.auto_copy_otp,
            pause_media_on_call: p.pause_media_on_call,
            sync_dnd: p.sync_dnd,
            send_to_menu: p.send_to_menu,
            auto_update: p.auto_update,
            install_updates: p.install_updates,
            battery_alerts: p.battery_alerts,
            start_with_windows: p.start_choice,
            recordings_folder: p.custom_recordings_folder.clone(),
            recordings_format: RecordingFormat::from_str_lossy(&String::from(&p.recordings_format)),
            webcam_phone: crate::webcam::selected_phone().map(|d| d.to_string()),
            webcam_height: crate::webcam::height(),
            webcam_mirror: crate::webcam::mirror(),
            webcam_camera: crate::webcam::camera().to_owned(),
            timeline_retention_days: p.timeline_retention_days.max(0) as u32,
            continuity_photo_hotkey: String::from(&p.continuity_photo_hotkey),
            continuity_scan_hotkey: String::from(&p.continuity_scan_hotkey),
            command_palette_hotkey: String::from(&p.command_palette_hotkey),
            shelf_hotkey: String::from(&p.shelf_hotkey),
            local_api_enabled: p.local_api_enabled,
            local_api_port: u16::try_from(p.local_api_port)
                .ok()
                .filter(|&port| port > 0)
                .unwrap_or(crate::settings::DEFAULT_LOCAL_API_PORT),
            clipboard_auto_copy_received_files: p.clipboard_auto_copy_received_files,
            open_shared_files: p.open_shared_files,
            flow_follow_clipboard: p.flow_follow_clipboard,
            folder_sync_rules: crate::settings::folder_sync_rules(),
            nectar_island: p.nectar_island,
            nectar_island_calls: p.nectar_island_calls,
            nectar_island_transfers: p.nectar_island_transfers,
            nectar_island_media: p.nectar_island_media,
            nectar_island_notifications: p.nectar_island_notifications,
            automation_rules: crate::settings::automation_rules(),
            stream_pc_audio_to_phone: p.stream_pc_audio_to_phone,
            share_pc_storage: p.share_pc_storage,
            forward_pc_notifications_to_phone: p.forward_pc_notifications_to_phone,
            away_mode: p.away_mode,
            walkthrough_completed: p.walkthrough_completed,
            walkthrough_step: p.walkthrough_step.clamp(0, 6) as u8,
            whats_new_dismissed: p.whats_new_dismissed,
            hive_invite_dismissed: p.hive_invite_dismissed,
            obs_port: crate::settings::obs_settings().0,
            obs_password: crate::settings::obs_settings().1,
            window_x: win_x,
            window_y: win_y,
            window_width: win_w,
            window_height: win_h,
            window_maximized: win_max,
        }
    }

    fn save(self: Pin<&mut Self>) {
        save_in_background(self.current_settings());
    }
}

/// Saves off the UI thread (the write is flushed to disk, which can take a
/// moment), newest settings last even when changes come in quick succession.
fn save_in_background(settings: Settings) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static WRITTEN: Mutex<u64> = Mutex::new(0);
    let seq = NEXT.fetch_add(1, Ordering::SeqCst) + 1;
    let save = move || {
        let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
        if *written > seq {
            return; // A newer save already ran.
        }
        if let Err(e) = settings.save(&core_host::host().data_dir) {
            tracing::warn!(error = %e, "can't save preferences");
        } else {
            crate::local_api::sync_from_settings();
        }
        *written = seq;
    };
    if let Err(e) = std::thread::Builder::new().name("save-preferences".into()).spawn(save) {
        tracing::warn!(error = %e, "can't save preferences");
    }
}
