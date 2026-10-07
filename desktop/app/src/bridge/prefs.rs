// SPDX-License-Identifier: GPL-3.0-or-later
//! `Preferences`: the app's look and behavior, saved whenever it changes.

use std::{
    path::PathBuf,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};

use crate::{
    core_host,
    settings::{ColorMode, RecordingFormat, Settings, Theme},
};

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
        /// Mica behind the window.
        #[qproperty(bool, backdrop)]
        #[qproperty(bool, close_to_tray)]
        /// Send what's copied on this PC to connected phones.
        #[qproperty(bool, auto_clipboard)]
        /// Paired phones in Explorer's "Send to" menu.
        #[qproperty(bool, send_to_menu)]
        /// Start when the user signs in.
        #[qproperty(bool, start_with_windows)]
        /// Check for updates on their own.
        #[qproperty(bool, auto_update)]
        /// Tell when a phone's battery is low, or full.
        #[qproperty(bool, battery_alerts)]
        /// Folder where voice recordings from phones are saved.
        #[qproperty(QString, recordings_folder)]
        /// Whether `recordings_folder` is the default (`Documents\Nectarlink Recordings`).
        #[qproperty(bool, recordings_folder_is_default)]
        /// "m4a", "mp3", "wav" or "flac".
        #[qproperty(QString, recordings_format)]
        type Preferences = super::PreferencesRust;

        /// Sets the folder where voice recordings are saved (from a `file:` URL or path).
        #[qinvokable]
        fn choose_recordings_folder(self: Pin<&mut Preferences>, url: &QString);
        /// Resets the recordings folder to `Documents\Nectarlink Recordings`.
        #[qinvokable]
        fn reset_recordings_folder(self: Pin<&mut Preferences>);
    }

    impl cxx_qt::Initialize for Preferences {}
}

#[derive(Default)]
pub struct PreferencesRust {
    theme: QString,
    seed: QString,
    color_mode: QString,
    backdrop: bool,
    close_to_tray: bool,
    auto_clipboard: bool,
    send_to_menu: bool,
    start_with_windows: bool,
    auto_update: bool,
    battery_alerts: bool,
    recordings_folder: QString,
    recordings_folder_is_default: bool,
    recordings_format: QString,
    /// What the user chose (`start_with_windows` shows the default until then).
    start_choice: Option<bool>,
    /// Custom recordings folder if chosen (`None` means default).
    custom_recordings_folder: Option<PathBuf>,
}

impl cxx_qt::Initialize for qobject::Preferences {
    fn initialize(mut self: Pin<&mut Self>) {
        let settings = Settings::load(&core_host::host().data_dir);
        self.as_mut().set_theme(QString::from(match settings.theme {
            Theme::Bloom => "bloom",
            Theme::Graphite => "graphite",
        }));
        self.as_mut().set_seed(QString::from(&settings.seed));
        self.as_mut().set_color_mode(QString::from(match settings.color_mode {
            ColorMode::System => "system",
            ColorMode::Light => "light",
            ColorMode::Dark => "dark",
        }));
        self.as_mut().set_backdrop(settings.backdrop);
        self.as_mut().set_close_to_tray(settings.close_to_tray);
        self.as_mut().set_auto_clipboard(settings.auto_clipboard);
        crate::clipboard::set_auto_send(settings.auto_clipboard);
        self.as_mut().set_send_to_menu(settings.send_to_menu);
        self.as_mut().set_auto_update(settings.auto_update);
        self.as_mut().set_battery_alerts(settings.battery_alerts);
        self.as_mut().set_start_with_windows(crate::startup::effective(settings.start_with_windows));
        self.as_mut().rust_mut().start_choice = settings.start_with_windows;
        let effective_folder = crate::recordings::effective_folder(settings.recordings_folder.as_deref());
        self.as_mut().set_recordings_folder(QString::from(&effective_folder.to_string_lossy().into_owned()));
        self.as_mut().set_recordings_folder_is_default(settings.recordings_folder.is_none());
        self.as_mut().rust_mut().custom_recordings_folder = settings.recordings_folder.clone();
        self.as_mut().set_recordings_format(QString::from(settings.recordings_format.as_str()));
        crate::recordings::init(&settings);

        // Save after any change (connected after loading, so loading doesn't
        // rewrite the file).
        self.as_mut().on_theme_changed(|p| p.save()).release();
        self.as_mut().on_seed_changed(|p| p.save()).release();
        self.as_mut().on_color_mode_changed(|p| p.save()).release();
        self.as_mut().on_backdrop_changed(|p| p.save()).release();
        self.as_mut().on_close_to_tray_changed(|p| p.save()).release();
        self.as_mut()
            .on_auto_clipboard_changed(|p| {
                crate::clipboard::set_auto_send(p.auto_clipboard);
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
            .on_battery_alerts_changed(|p| {
                crate::battery::set_enabled(p.battery_alerts);
                p.save();
            })
            .release();
        self.as_mut()
            .on_start_with_windows_changed(|mut p| {
                let on = p.start_with_windows;
                p.as_mut().rust_mut().start_choice = Some(on);
                crate::startup::apply(on);
                p.save();
            })
            .release();
        self.as_mut()
            .on_recordings_format_changed(|p| {
                let fmt = RecordingFormat::from_str_lossy(&String::from(&p.recordings_format));
                crate::recordings::set_format(fmt);
                p.save();
            })
            .release();
    }
}

impl qobject::Preferences {
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

    fn save(self: Pin<&mut Self>) {
        let p = self.rust();
        let settings = Settings {
            theme: if String::from(&p.theme) == "graphite" { Theme::Graphite } else { Theme::Bloom },
            seed: String::from(&p.seed),
            color_mode: match String::from(&p.color_mode).as_str() {
                "light" => ColorMode::Light,
                "dark" => ColorMode::Dark,
                _ => ColorMode::System,
            },
            backdrop: p.backdrop,
            close_to_tray: p.close_to_tray,
            auto_clipboard: p.auto_clipboard,
            send_to_menu: p.send_to_menu,
            auto_update: p.auto_update,
            battery_alerts: p.battery_alerts,
            start_with_windows: p.start_choice,
            recordings_folder: p.custom_recordings_folder.clone(),
            recordings_format: RecordingFormat::from_str_lossy(&String::from(&p.recordings_format)),
        };
        save_in_background(settings);
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
        }
        *written = seq;
    };
    if let Err(e) = std::thread::Builder::new().name("save-preferences".into()).spawn(save) {
        tracing::warn!(error = %e, "can't save preferences");
    }
}
