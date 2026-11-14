// SPDX-License-Identifier: GPL-3.0-or-later
//! NectarLink for Windows.
//!
//! One process: Rust runs the core (`nectarlink-core`) and OS integration,
//! Qt Quick draws the UI. Windows are created when opened and destroyed when
//! closed; the app keeps running in the tray (docs/PLAN.md §5).

// A GUI app: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod automations;
mod battery;
mod bridge;
mod calls;
mod clipboard;
mod command_palette;
mod continuity_camera;
mod core_host;
mod deck;
mod doctor;
mod feed;
mod folder_sync;
mod hotspot;
mod i18n;
mod icons;
mod input_share;
mod launch;
mod links;
mod local_api;
mod logging;
mod mark;
mod media;
mod media_file;
mod messages;
mod mirror;
mod notification_store;
mod notifications;
mod obs;
mod palette;
mod pc_audio_stream;
mod pc_mirror;
mod pc_notifications;
mod pc_storage;
mod phone_install;
mod phone_mic;
mod phone_sound;
mod photo_backup;
mod photos;
mod play_on_phone;
mod qr;
mod recordings;
mod remote;
mod send_to;
mod settings;
mod setup;
mod startup;
mod state;
mod storage;
mod transfers;
mod update_relay;
mod updater;
mod version;
mod webcam;
#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
#[path = "linux/mod.rs"]
mod win;
#[cfg(target_os = "macos")]
#[path = "mac/mod.rs"]
mod win;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(not(target_os = "linux"))]
use cxx_qt_lib::QGuiApplication;
use cxx_qt_lib::{QQmlApplicationEngine, QString, QUrl};
use nectarlink_core::{
    MediaAction, MediaError, MirrorInput, MirrorStart, Platform, PowerAction, StorageEntry, StorageError,
    StorageReadFile, StorageWriteDone,
};

use crate::{
    launch::Request,
    win::single_instance::{self, Instance},
};

/// How many fonts `build.rs` compiles in.
const BUNDLED_FONTS: i32 = 4;

/// Rings the PC when a phone asks ("find my PC").
#[derive(Debug)]
struct DesktopPlatform;

impl Platform for DesktopPlatform {
    fn start_ringing(&self) {
        win::sound::start_ringing();
    }
    fn stop_ringing(&self) {
        win::sound::stop_ringing();
    }
    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        win::clipboard::write(text)
    }
    fn set_clipboard_rich(&self, text: &str, html: Option<&str>, rtf: Option<&str>) -> Result<(), String> {
        win::clipboard::write_rich(text, html, rtf)
    }
    fn set_clipboard_private(&self, text: &str) -> Result<(), String> {
        win::clipboard::write_private(text)
    }
    fn clear_clipboard_private(&self, text: &str) {
        win::clipboard::clear_private(text);
    }
    fn set_clipboard_image(&self, mime: &str, bytes: &[u8]) -> Result<(), String> {
        win::clipboard::write_image(mime, bytes)
    }
    fn power(&self, action: PowerAction) -> Result<(), String> {
        links::power(action)
    }
    fn open_link(&self, _from: &nectarlink_core::DeviceId, url: &str) -> Result<(), String> {
        links::open_here(url)
    }
    fn media_command(
        &self,
        player: &str,
        action: MediaAction,
        position: Option<u64>,
    ) -> Result<(), MediaError> {
        win::media_sessions::command(player, action, position)
    }
    fn mirror_requested(
        &self,
        peer: &nectarlink_core::DeviceId,
        options: &MirrorStart,
    ) -> Result<(), String> {
        if options.session != nectarlink_core::MIRROR_SCREEN {
            return Err("unsupported mirror session on PC".into());
        }
        pc_mirror::start(*peer, options.clone());
        Ok(())
    }
    fn mirror_stop_requested(&self, peer: &nectarlink_core::DeviceId, session: u32) {
        if session == nectarlink_core::MIRROR_SCREEN {
            pc_mirror::stop_for(peer);
        }
    }
    fn mirror_keyframe_requested(&self, peer: &nectarlink_core::DeviceId, session: u32) {
        if session == nectarlink_core::MIRROR_SCREEN {
            pc_mirror::request_keyframe(peer);
        }
    }
    fn mirror_input(&self, peer: &nectarlink_core::DeviceId, session: u32, input: MirrorInput) {
        pc_mirror::handle_input(peer, session, input);
    }
    fn mirror_sink(
        &self,
        peer: &nectarlink_core::DeviceId,
        session: u32,
    ) -> Option<std::sync::Arc<dyn nectarlink_core::MirrorSink>> {
        mirror::sink(peer, session)
    }
    fn mic_sink(
        &self,
        peer: &nectarlink_core::DeviceId,
    ) -> Option<std::sync::Arc<dyn nectarlink_core::MicSink>> {
        phone_mic::sink(peer)
    }
    fn webcam_sink(
        &self,
        peer: &nectarlink_core::DeviceId,
    ) -> Option<std::sync::Arc<dyn nectarlink_core::WebcamSink>> {
        webcam::sink(peer)
    }
    fn remote_input(
        &self,
        peer: &nectarlink_core::DeviceId,
        input: nectarlink_core::RemoteInput,
    ) -> Result<(), String> {
        remote::handle_input(peer, input);
        Ok(())
    }
    fn deck_press(&self, peer: &nectarlink_core::DeviceId, tile: &str) -> Result<(), String> {
        deck::handle_press(peer, tile)
    }
    fn set_pc_audio(
        &self,
        peer: &nectarlink_core::DeviceId,
        volume: Option<u8>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        deck::handle_set_audio(peer, volume, muted)
    }
    fn set_pc_audio_output(&self, _peer: &nectarlink_core::DeviceId, id: &str) -> Result<(), String> {
        deck::handle_set_output(id)
    }
    fn storage_list(&self, path: &str) -> Result<Vec<StorageEntry>, StorageError> {
        pc_storage::storage_list(path)
    }
    fn storage_open_read(&self, path: &str) -> Result<StorageReadFile, StorageError> {
        pc_storage::storage_open_read(path)
    }
    fn storage_write(
        &self,
        path: &str,
        staged: &std::path::Path,
        modified: Option<i64>,
    ) -> Result<StorageWriteDone, StorageError> {
        pc_storage::storage_write(path, staged, modified)
    }
    fn storage_mkdir(&self, path: &str) -> Result<(), StorageError> {
        pc_storage::storage_mkdir(path)
    }
    fn storage_rename(&self, path: &str, new_name: &str) -> Result<(), StorageError> {
        pc_storage::storage_rename(path, new_name)
    }
    fn storage_delete(&self, path: &str, confirmed: bool) -> Result<(), StorageError> {
        pc_storage::storage_delete(path, confirmed)
    }
}

/// Command-line options (also used by shortcuts and autostart).
#[derive(Debug, Default)]
struct Options {
    /// Where to keep data (default: %LOCALAPPDATA%\Nectarlink).
    data_dir: Option<PathBuf>,
    /// Start in the tray without opening the window (autostart).
    minimized: bool,
    /// Started by the updater's relauncher, as the new version.
    updated: bool,
    /// `--send-to <device> <files…>`: Explorer's "Send to" menu.
    send_to: Option<(String, Vec<PathBuf>)>,
    /// `--share <targets…>`: Windows Share sheet or CLI share invocation.
    share: Option<win::share_target::SharedPayload>,
    /// `--task <action>`: Windows taskbar Jump List task.
    task: Option<String>,
    /// Ask the running app to quit (the installer, before an update).
    quit: bool,
    /// Quit the running app and undo what it set up in Windows (the
    /// uninstaller). Data is kept.
    uninstall: bool,
    /// Elevated helper: register `nectarlink_vcam.dll` in `HKLM` and exit.
    register_vcam: bool,
    /// Elevated helper: unregister `nectarlink_vcam.dll` from `HKLM` and exit.
    unregister_vcam: bool,
    /// Elevated helper (Windows): register the sparse package and exit (#84).
    register_package: bool,
    /// Out-of-process Windows 11 Widget Board COM server (`--widget-provider`).
    widget_provider: bool,
    /// The update task's job, as SYSTEM (`--update-service`, see `updater`).
    update_service: bool,
    /// The installer's window (`--setup --installer <setup.exe>`, see `setup`).
    setup: Option<PathBuf>,
    /// The uninstaller's window (`--remove [--installed <folder>]`).
    remove: bool,
    installed: Option<PathBuf>,
    /// `--ui-check` (needs `--data-dir`): open the main window, quit after
    /// [`UI_CHECK_SECONDS`], and fail if QML reported a warning or an error.
    /// Qt's messages go to stderr. For CI (`scripts/ui-check.ps1`).
    ui_check: bool,
    /// `--version` / `-V`: print the version and exit.
    version: bool,
}

/// How long `--ui-check` keeps the window open: long enough for the window
/// to load, be restored and settle.
const UI_CHECK_SECONDS: u64 = 8;

fn parse_options() -> Options {
    parse_options_from(std::env::args_os().skip(1))
}

fn parse_options_from<I: IntoIterator<Item = std::ffi::OsString>>(iter: I) -> Options {
    let mut options = Options::default();
    let mut paths = Vec::new();
    let mut share_targets = Vec::new();
    let mut in_share = false;
    let mut args = iter.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                in_share = false;
                options.data_dir = args.next().map(PathBuf::from);
            }
            Some("--minimized") => {
                in_share = false;
                options.minimized = true;
            }
            Some("--quit") => {
                in_share = false;
                options.quit = true;
            }
            Some("--uninstall") => {
                in_share = false;
                options.uninstall = true;
            }
            Some("--ui-check") => {
                in_share = false;
                options.ui_check = true;
            }
            Some("--version" | "-V") => {
                in_share = false;
                options.version = true;
            }
            Some(setup::SETUP_ARG) => {
                in_share = false;
                options.setup = Some(PathBuf::new());
            }
            Some(setup::INSTALLER_ARG) => {
                in_share = false;
                options.setup = args.next().map(PathBuf::from);
            }
            Some(setup::REMOVE_ARG) => {
                in_share = false;
                options.remove = true;
            }
            Some(setup::INSTALLED_ARG) => {
                in_share = false;
                options.installed = args.next().map(PathBuf::from);
            }
            Some(updater::SERVICE_ARG) => {
                in_share = false;
                options.update_service = true;
            }
            Some(updater::UPDATED_ARG) => {
                in_share = false;
                options.updated = true;
            }
            Some(win::vcam::REGISTER_ARG) => {
                in_share = false;
                options.register_vcam = true;
            }
            Some(win::vcam::UNREGISTER_ARG) => {
                in_share = false;
                options.unregister_vcam = true;
            }
            Some("--register-package") => {
                in_share = false;
                options.register_package = true;
            }
            Some(win::widget::ARG) => {
                in_share = false;
                options.widget_provider = true;
            }
            Some(send_to::ARG) => {
                in_share = false;
                let device = args.next().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                options.send_to = Some((device, Vec::new()));
            }
            Some(win::jumplist::ARG) => {
                in_share = false;
                options.task =
                    args.next().map(|a| a.to_string_lossy().into_owned()).filter(|s| !s.is_empty());
            }
            Some(win::share_target::ARG) => {
                in_share = true;
            }
            _ if in_share && !arg.to_string_lossy().starts_with('-') => {
                share_targets.push(arg.to_string_lossy().into_owned());
            }
            // Explorer adds the chosen files after the shortcut's arguments.
            _ if !arg.to_string_lossy().starts_with('-') => paths.push(PathBuf::from(arg)),
            // Qt reads its own options; ignore everything else.
            _ => {
                in_share = false;
            }
        }
    }
    if let Some((_, files)) = &mut options.send_to {
        *files = paths;
        // Sending doesn't need the window.
        options.minimized = true;
    } else if share_targets.is_empty() && !paths.is_empty() {
        // Desktop entry `Exec=nectarlink-desktop %U` passes URLs or dropped files
        // as positional arguments without `--share`.
        share_targets = paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
    }
    if matches!(options.task.as_deref(), Some("send-clipboard" | "find-phone" | "take-photo")) {
        options.minimized = true;
    }
    if !share_targets.is_empty() {
        let payload = win::share_target::parse_targets(&share_targets);
        if !payload.is_empty() {
            options.share = Some(payload);
            options.minimized = true;
        }
    }
    options
}

fn default_data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("Nectarlink")
}

/// Network changes come in bursts; tell the core once things settle.
fn watch_network() {
    static PENDING: AtomicBool = AtomicBool::new(false);
    win::net::watch(|| {
        if PENDING.swap(true, Ordering::AcqRel) {
            return;
        }
        core_host::spawn(async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            PENDING.store(false, Ordering::Release);
            core_host::refresh_wake().await;
            if let Some(node) = core_host::node() {
                tracing::debug!("network changed");
                node.network_changed().await;
            }
        });
    });
}

/// Phone notifications as Windows toasts, sent by "NectarLink" with the
/// app's icon (written as a PNG, which Windows needs as a file).
fn start_toasts() {
    let icon = core_host::host().data_dir.join("cache").join("nectarlink.png");
    win::toast::start(icon, notifications::on_toast);
}

/// Removes what the app set up in Windows: the "Send to" entries and the
/// notification sender registration.
fn uninstall() {
    tracing::info!("uninstalling");
    if let Err(e) = send_to::remove_all() {
        tracing::warn!(error = %e, "can't remove the Send to entries");
    }
    storage::unregister_all();
    let _ = win::vcam::unregister_hklm();
    win::toast::unregister();
    win::jumplist::unregister();
    startup::remove();
}

fn install_panic_logging() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default(info);
    }));
}

/// The installer's or uninstaller's window: no core, no data folder, no log
/// file (removal deletes that folder).
fn run_setup_window(mode: setup::Mode) -> ExitCode {
    if let setup::Mode::Remove { installed } = &mode {
        match setup::hand_over_removal(installed) {
            Ok(true) => return ExitCode::SUCCESS,
            Ok(false) => {}
            Err(e) => {
                eprintln!("nectarlink: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    bridge::setup::set_mode(mode);
    bridge::native::ffi::prepare_qt_without_log();
    #[cfg(target_os = "linux")]
    let mut app = bridge::native::ffi::create_qt_app();
    #[cfg(not(target_os = "linux"))]
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("NectarLink Setup"));
        app.as_mut().set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    // Before any QML loads, so every string is right on the first pass.
    i18n::install("");
    bridge::native::ffi::load_bundled_fonts();
    for size in [16, 20, 24, 32, 40, 48, 64, 256] {
        bridge::native::ffi::add_app_icon_image(size, &mark::render(size as usize));
    }
    bridge::native::ffi::apply_app_icon();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/app/nectarlink/qml/setup/SetupWindow.qml"));
    }
    let code = app.as_mut().map_or(1, |app| app.exec());
    drop(engine);
    drop(app);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn main() -> ExitCode {
    let options = parse_options();
    if options.version {
        println!("nectarlink-desktop {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if let Some(installer) = options.setup.clone().filter(|p| !p.as_os_str().is_empty()) {
        return run_setup_window(setup::Mode::Install { installer });
    }
    if options.remove {
        let installed = options
            .installed
            .clone()
            .or_else(|| std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)));
        let Some(installed) = installed else { return ExitCode::FAILURE };
        return run_setup_window(setup::Mode::Remove { installed });
    }
    if options.update_service {
        return updater::run_service();
    }
    if options.register_vcam {
        return match win::vcam::register_hklm() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nectarlink: register-vcam failed: {e}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(windows)]
    if options.register_package {
        return win::share_target::register_elevated();
    }
    if options.unregister_vcam {
        return match win::vcam::unregister_hklm() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nectarlink: unregister-vcam failed: {e}");
                ExitCode::FAILURE
            }
        };
    }
    // The check starts the whole app, so never on the user's own data.
    if options.ui_check && options.data_dir.is_none() {
        eprintln!("nectarlink: --ui-check needs --data-dir");
        return ExitCode::FAILURE;
    }
    let data_dir = options.data_dir.clone().unwrap_or_else(default_data_dir);
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("nectarlink: can't create {}: {e}", data_dir.display());
        return ExitCode::FAILURE;
    }
    logging::init(&data_dir);
    install_panic_logging();
    // Home and the other phone pages only load with a paired phone, and a
    // page that fails to load must fail the check.
    if options.ui_check
        && let Err(e) = nectarlink_core::add_ui_check_phone(&data_dir)
    {
        eprintln!("nectarlink: --ui-check: can't add the pretend phone: {e}");
        return ExitCode::FAILURE;
    }
    if options.widget_provider {
        return win::widget::run_provider(&data_dir);
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), minimized = options.minimized, "starting");

    if options.uninstall {
        uninstall();
    }
    let share_payload = options.share.clone().or_else(|| win::share_target::take_activated_share(&data_dir));
    let request = if options.quit || options.uninstall {
        Request::Quit
    } else if let Some((device, paths)) = options.send_to.clone() {
        Request::Send { device, paths }
    } else if let Some(shared) = share_payload {
        Request::Share { files: shared.files, url: shared.url, text: shared.text }
    } else if let Some(action) = options.task.clone() {
        Request::Task { action }
    } else {
        Request::Show
    };
    let instance = match single_instance::acquire(&data_dir) {
        // Nothing running to quit.
        Ok(Instance::Primary(_)) if request == Request::Quit => return ExitCode::SUCCESS,
        Ok(Instance::Primary(primary)) => Some(&*Box::leak(Box::new(primary))),
        Ok(Instance::Secondary) => {
            // Without the request, the running instance still shows itself.
            if let Err(e) = launch::queue(&data_dir, &request) {
                tracing::warn!(error = %e, "can't pass the request on");
            }
            single_instance::wake(&data_dir);
            tracing::info!("already running; passed the request on");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            tracing::warn!(error = %e, "single-instance check failed; continuing");
            None
        }
    };

    // Before any window: toasts and the taskbar both go by this ID.
    win::toast::set_process_id();
    // Logging out or shutting down sends SIGTERM: quit as Quit does, so
    // phone storage is unmounted instead of left behind.
    #[cfg(target_os = "linux")]
    win::quit_on_signals(bridge::app::request_quit);
    win::jumplist::register(options.data_dir.as_deref());
    // Before the core looks for adb: one downloaded for phones on a cable is used too.
    phone_install::init(&data_dir);
    if let Err(e) = core_host::start(data_dir.clone(), Arc::new(DesktopPlatform)) {
        tracing::error!(error = %e, "can't start the core runtime");
        return ExitCode::FAILURE;
    }
    // The node starts in the background, so wait for it: the beacon carries
    // its device ID.
    core_host::spawn(async {
        if let Some(node) = core_host::wait_for_node().await {
            win::ble_beacon::start(node.device_id());
        }
    });
    core_host::host().hub.update(|s| {
        notification_store::load(&data_dir, s);
        state::Changes::APPS | state::Changes::HISTORY
    });
    notification_store::start(data_dir.clone());
    std::thread::spawn(notifications::prune_images);
    watch_network();
    start_toasts();
    // The Share sheet and notification buttons, after an update (#84).
    #[cfg(windows)]
    win::share_target::ensure_sparse_package(data_dir.clone());
    clipboard::start();
    win::smtc::start(media::on_flyout);
    win::media_sessions::start(media::local_changed);
    deck::init(&data_dir);
    let settings = settings::Settings::load(&data_dir);
    settings.apply_runtime();
    clipboard::set_auto_send(settings.auto_clipboard);
    clipboard::set_history_enabled(settings.clipboard_history);
    clipboard::set_suggest_actions(settings.suggest_clipboard_actions);
    notifications::set_auto_copy_otp(settings.auto_copy_otp);
    notifications::set_sync_dnd(settings.sync_dnd);
    notifications::set_hold_when_locked(settings.hold_when_locked);
    notifications::start_lock_watch();
    calls::set_pause_media_on_call(settings.pause_media_on_call);
    pc_mirror::set_mute_while_watched(settings.mute_while_watched);
    recordings::init(&settings);
    send_to::set_enabled(settings.send_to_menu);
    battery::set_enabled(settings.battery_alerts);
    storage::start();
    folder_sync::start_poller();
    photo_backup::start();
    webcam::init(&settings);
    phone_mic::init(&settings);
    local_api::init(&settings);
    win::widget::write_state_snapshot(&data_dir);
    win::tray::update_hotkeys(
        &settings.continuity_photo_hotkey,
        &settings.continuity_scan_hotkey,
        &settings.command_palette_hotkey,
        &settings.shelf_hotkey,
    );
    let updated_from = updater::note_running_version(&data_dir);
    if updated_from.is_some() {
        let mut saved = settings::Settings::load(&data_dir);
        if saved.ask_for_star_after_update()
            && let Err(e) = saved.save(&data_dir)
        {
            tracing::warn!(error = %e, "can't save settings");
        }
    }
    // A test instance (own data folder) leaves the user's menu and sign-in
    // alone.
    if options.data_dir.is_none() {
        send_to::start();
        startup::start(settings.start_with_windows);
        updater::set_auto(settings.auto_update);
        updater::set_auto_install(settings.install_updates);
        // Kept from the first run: going by the version alone, a nightly install
        // would leave Nightly as soon as it took a release.
        let channel = version::Channel::parse(&settings.update_channel).unwrap_or_else(|| {
            let own = updater::channel();
            let mut saved = settings::Settings::load(&data_dir);
            saved.update_channel = own.as_str().into();
            if let Err(e) = saved.save(&data_dir) {
                tracing::warn!(error = %e, "can't save settings");
            }
            own
        });
        updater::set_channel(channel);
        updater::start();
        match (updated_from, options.updated) {
            (Some(previous), _) => {
                // A new version invites to try Hive again, once.
                let mut saved = settings::Settings::load(&data_dir);
                if std::mem::take(&mut saved.hive_invite_dismissed)
                    && let Err(e) = saved.save(&data_dir)
                {
                    tracing::warn!(error = %e, "can't save settings");
                }
                updater::announce_update(previous, !options.minimized);
            }
            // Reopened by the relauncher, still on this version.
            (None, true) => updater::announce_update_failed(!options.minimized),
            (None, false) => {}
        }
    }
    // Requests left while no instance was running, then this launch's own.
    for waiting in launch::drain(&data_dir) {
        if matches!(
            waiting,
            Request::Send { .. } | Request::Share { .. } | Request::TaskNotify { .. } | Request::Task { .. }
        ) {
            send_to::handle(waiting);
        }
    }
    if matches!(request, Request::Send { .. } | Request::Share { .. } | Request::Task { .. }) {
        send_to::handle(request);
    }

    if options.ui_check {
        bridge::native::ffi::prepare_qt_for_ui_check();
    } else {
        bridge::native::ffi::prepare_qt(&logging::logs_dir(&data_dir).to_string_lossy());
    }
    bridge::native::ffi::set_ui_scale(i32::from(settings::Settings::load(&data_dir).ui_scale));
    #[cfg(target_os = "linux")]
    let mut app = bridge::native::ffi::create_qt_app();
    #[cfg(not(target_os = "linux"))]
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("Nectarlink"));
        app.as_mut().set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    // Before any QML loads, so every string is right on the first pass.
    i18n::install(&settings::Settings::load(&data_dir).language);
    bridge::native::ffi::keep_running_without_windows();
    // Text falls back to system fonts if these are missing, so it's not fatal.
    let fonts = bridge::native::ffi::load_bundled_fonts();
    if fonts < BUNDLED_FONTS {
        tracing::warn!(loaded = fonts, expected = BUNDLED_FONTS, "some bundled fonts didn't load");
    }
    for size in [16, 20, 24, 32, 40, 48, 64, 256] {
        bridge::native::ffi::add_app_icon_image(size, &mark::render(size as usize));
    }
    bridge::native::ffi::apply_app_icon();
    bridge::native::ffi::stay_out_of_session_restore();

    if let Some(instance) = instance {
        let data_dir = data_dir.clone();
        instance.watch(move || {
            let requests = launch::drain(&data_dir);
            if requests.is_empty() {
                bridge::app::request_activation();
            }
            requests.into_iter().for_each(send_to::handle);
        });
    }

    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        bridge::native::ffi::register_qml_app_engine(engine.as_mut());
        engine.load(&QUrl::from("qrc:/qt/qml/app/nectarlink/qml/App.qml"));
    }
    if options.ui_check {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(UI_CHECK_SECONDS));
            bridge::app::request_quit();
        });
    }
    let code = if bridge::app::is_quit_requested() { 0 } else { app.as_mut().map_or(1, |app| app.exec()) };

    tracing::info!(code, "shutting down");
    // QML first (it holds references into Qt), then the core.
    drop(engine);
    mirror::stop_decoders();
    pc_audio_stream::set_enabled(false);
    pc_mirror::stop_all();
    photos::clear_shared_files();
    local_api::stop();
    win::ble_beacon::stop();
    webcam::shutdown();
    storage::shutdown();
    win::net::stop();
    core_host::shutdown();
    drop(app);
    if options.ui_check {
        let problems = bridge::native::ffi::qml_problem_count();
        if problems > 0 {
            eprintln!("nectarlink: --ui-check: {problems} QML warning(s) or error(s), see above");
            return ExitCode::FAILURE;
        }
        eprintln!("nectarlink: --ui-check: no QML warnings");
    }
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    /// Qt gives its windows the icon of this exact name, or else a generic one.
    #[cfg(windows)]
    #[allow(unsafe_code)]
    #[test]
    fn the_exe_has_the_icon_qt_looks_for() {
        use windows::{
            Win32::{
                System::LibraryLoader::GetModuleHandleW,
                UI::WindowsAndMessaging::{IMAGE_ICON, LR_DEFAULTSIZE, LoadImageW},
            },
            core::w,
        };
        // SAFETY: the name is a valid wide string, and this only reads our own resources.
        let icon = unsafe {
            let module = GetModuleHandleW(None).unwrap();
            LoadImageW(Some(module.into()), w!("IDI_ICON1"), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE)
        };
        assert!(icon.is_ok_and(|icon| !icon.is_invalid()));
    }

    use std::ffi::OsString;

    use super::*;

    #[test]
    fn parse_options_recognizes_share_flag() {
        let opts = parse_options_from(
            ["--data-dir", r"C:\Temp\nectarlink-test", "--share", r"C:\photo.jpg", "https://example.com"]
                .into_iter()
                .map(OsString::from),
        );
        assert_eq!(opts.data_dir, Some(PathBuf::from(r"C:\Temp\nectarlink-test")));
        assert!(opts.minimized);
        assert_eq!(
            opts.share,
            Some(win::share_target::SharedPayload {
                files: vec![PathBuf::from(r"C:\photo.jpg")],
                url: Some("https://example.com".into()),
                text: None,
            })
        );
    }

    #[test]
    fn parse_options_recognizes_ui_check_flag() {
        let opts = parse_options_from(
            ["--ui-check", "--data-dir", r"C:\Temp\nectarlink-test"].into_iter().map(OsString::from),
        );
        assert!(opts.ui_check);
        assert!(!opts.minimized);
        assert_eq!(opts.data_dir, Some(PathBuf::from(r"C:\Temp\nectarlink-test")));
        assert!(!parse_options_from(["--minimized"].into_iter().map(OsString::from)).ui_check);
    }

    #[test]
    fn parse_options_recognizes_task_flag() {
        let find_opts = parse_options_from(
            ["--data-dir", r"C:\Temp\nectarlink-test", "--task", "find-phone"]
                .into_iter()
                .map(OsString::from),
        );
        assert_eq!(find_opts.data_dir, Some(PathBuf::from(r"C:\Temp\nectarlink-test")));
        assert_eq!(find_opts.task.as_deref(), Some("find-phone"));
        assert!(find_opts.minimized);

        let timeline_opts = parse_options_from(
            ["--data-dir", r"C:\Temp\nectarlink-test", "--task", "open-timeline"]
                .into_iter()
                .map(OsString::from),
        );
        assert_eq!(timeline_opts.task.as_deref(), Some("open-timeline"));
        assert!(!timeline_opts.minimized);
    }

    #[test]
    fn parse_options_recognizes_widget_provider_flag() {
        let opts = parse_options_from(
            ["--data-dir", r"C:\Temp\nectarlink-test", "--widget-provider"].into_iter().map(OsString::from),
        );
        assert_eq!(opts.data_dir, Some(PathBuf::from(r"C:\Temp\nectarlink-test")));
        assert!(opts.widget_provider);
    }

    #[test]
    fn parse_options_recognizes_version_and_positional_desktop_uris() {
        let ver = parse_options_from(["--version"].into_iter().map(OsString::from));
        assert!(ver.version);

        let web_opts = parse_options_from(["https://example.com"].into_iter().map(OsString::from));
        assert!(web_opts.minimized);
        assert_eq!(
            web_opts.share,
            Some(win::share_target::SharedPayload {
                files: Vec::new(),
                url: Some("https://example.com".into()),
                text: None,
            })
        );

        #[cfg(target_os = "linux")]
        {
            let uri_opts = parse_options_from(["tel:+15551234567"].into_iter().map(OsString::from));
            assert!(uri_opts.minimized);
            assert_eq!(
                uri_opts.share,
                Some(win::share_target::SharedPayload {
                    files: Vec::new(),
                    url: Some("tel:+15551234567".into()),
                    text: None,
                })
            );
        }
    }
}
