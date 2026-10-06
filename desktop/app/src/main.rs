// SPDX-License-Identifier: GPL-3.0-or-later
//! Nectarlink for Windows.
//!
//! One process: Rust runs the core (`nectarlink-core`) and OS integration,
//! Qt Quick draws the UI. Windows are created when opened and destroyed when
//! closed; the app keeps running in the tray (docs/PLAN.md §5).

// A GUI app: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod battery;
mod bridge;
mod calls;
mod clipboard;
mod core_host;
mod doctor;
mod icons;
mod launch;
mod links;
mod logging;
mod mark;
mod media;
mod notification_store;
mod notifications;
mod palette;
mod photos;
mod qr;
mod send_to;
mod settings;
mod startup;
mod state;
mod transfers;
mod updater;
mod win;

use std::{
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use nectarlink_core::{MediaAction, MediaError, Platform, PowerAction};

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
}

/// Command-line options (also used by shortcuts and autostart).
#[derive(Debug, Default)]
struct Options {
    /// Where to keep data (default: %LOCALAPPDATA%\Nectarlink).
    data_dir: Option<PathBuf>,
    /// Start in the tray without opening the window (autostart).
    minimized: bool,
    /// `--send-to <device> <files…>`: Explorer's "Send to" menu.
    send_to: Option<(String, Vec<PathBuf>)>,
    /// Ask the running app to quit (the installer, before an update).
    quit: bool,
    /// Quit the running app and undo what it set up in Windows (the
    /// uninstaller). Data is kept.
    uninstall: bool,
}

fn parse_options() -> Options {
    let mut options = Options::default();
    let mut paths = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => options.data_dir = args.next().map(PathBuf::from),
            Some("--minimized") => options.minimized = true,
            Some("--quit") => options.quit = true,
            Some("--uninstall") => options.uninstall = true,
            Some(send_to::ARG) => {
                let device = args.next().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                options.send_to = Some((device, Vec::new()));
            }
            // Explorer adds the chosen files after the shortcut's arguments.
            _ if !arg.to_string_lossy().starts_with('-') => paths.push(PathBuf::from(arg)),
            // Qt reads its own options; ignore everything else.
            _ => {}
        }
    }
    if let Some((_, files)) = &mut options.send_to {
        *files = paths;
        // Sending doesn't need the window.
        options.minimized = true;
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
            if let Some(node) = core_host::node() {
                tracing::debug!("network changed");
                node.network_changed().await;
            }
        });
    });
}

/// Phone notifications as Windows toasts, sent by "Nectarlink" with the
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
    win::toast::unregister();
    startup::remove();
}

fn install_panic_logging() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default(info);
    }));
}

fn main() -> ExitCode {
    let options = parse_options();
    let data_dir = options.data_dir.clone().unwrap_or_else(default_data_dir);
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("nectarlink: can't create {}: {e}", data_dir.display());
        return ExitCode::FAILURE;
    }
    logging::init(&data_dir);
    install_panic_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), minimized = options.minimized, "starting");

    if options.uninstall {
        uninstall();
    }
    let request = match options.send_to.clone() {
        _ if options.quit || options.uninstall => Request::Quit,
        Some((device, paths)) => Request::Send { device, paths },
        None => Request::Show,
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
    if let Err(e) = core_host::start(data_dir.clone(), Arc::new(DesktopPlatform)) {
        tracing::error!(error = %e, "can't start the core runtime");
        return ExitCode::FAILURE;
    }
    core_host::host().hub.update(|s| {
        notification_store::load(&data_dir, s);
        state::Changes::APPS | state::Changes::HISTORY
    });
    notification_store::start(data_dir.clone());
    std::thread::spawn(notifications::prune_images);
    watch_network();
    start_toasts();
    clipboard::start();
    win::smtc::start(media::on_flyout);
    win::media_sessions::start(media::local_changed);
    let settings = settings::Settings::load(&data_dir);
    send_to::set_enabled(settings.send_to_menu);
    battery::set_enabled(settings.battery_alerts);
    // A test instance (own data folder) leaves the user's menu and sign-in
    // alone.
    if options.data_dir.is_none() {
        send_to::start();
        startup::start(settings.start_with_windows);
        updater::set_auto(settings.auto_update);
        updater::start();
    }
    // Requests left while no instance was running, then this launch's own.
    for waiting in launch::drain(&data_dir) {
        if matches!(waiting, Request::Send { .. }) {
            send_to::handle(waiting);
        }
    }
    if matches!(request, Request::Send { .. }) {
        send_to::handle(request);
    }

    bridge::native::ffi::prepare_qt();
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("Nectarlink"));
        app.as_mut().set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
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
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/app/nectarlink/qml/App.qml"));
    }
    let code = app.as_mut().map_or(1, |app| app.exec());

    tracing::info!(code, "shutting down");
    // QML first (it holds references into Qt), then the core.
    drop(engine);
    win::net::stop();
    core_host::shutdown();
    drop(app);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
