// SPDX-License-Identifier: GPL-3.0-or-later
//! Nectarlink for Windows.
//!
//! One process: Rust runs the core (`nectarlink-core`) and OS integration,
//! Qt Quick draws the UI. Windows are created when opened and destroyed when
//! closed; the app keeps running in the tray (docs/PLAN.md §5).

// A GUI app: no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod clipboard;
mod core_host;
mod icons;
mod logging;
mod notifications;
mod palette;
mod qr;
mod settings;
mod state;
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
use nectarlink_core::Platform;

use crate::win::single_instance::{self, Instance};

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
}

/// Command-line options (also used by shortcuts and autostart).
#[derive(Debug, Default)]
struct Options {
    /// Where to keep data (default: %LOCALAPPDATA%\Nectarlink).
    data_dir: Option<PathBuf>,
    /// Start in the tray without opening the window (autostart).
    minimized: bool,
}

fn parse_options() -> Options {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => options.data_dir = args.next().map(PathBuf::from),
            "--minimized" => options.minimized = true,
            // Qt reads its own arguments; ignore everything else.
            _ => {}
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

    let instance = match single_instance::acquire(&data_dir) {
        Ok(Instance::Primary(primary)) => Some(&*Box::leak(Box::new(primary))),
        Ok(Instance::Secondary) => {
            tracing::info!("already running; asked it to show itself");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            tracing::warn!(error = %e, "single-instance check failed; continuing");
            None
        }
    };

    // Before any window: toasts and the taskbar both go by this ID.
    win::toast::set_process_id();
    if let Err(e) = core_host::start(data_dir, Arc::new(DesktopPlatform)) {
        tracing::error!(error = %e, "can't start the core runtime");
        return ExitCode::FAILURE;
    }
    watch_network();
    start_toasts();
    clipboard::start();

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
        bridge::native::ffi::add_app_icon_image(size, &win::icon::render(size as usize));
    }
    bridge::native::ffi::apply_app_icon();

    if let Some(instance) = instance {
        instance.watch(bridge::app::request_activation);
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
