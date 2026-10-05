// SPDX-License-Identifier: GPL-3.0-or-later
//! `Preferences`: the app's look and behavior, saved whenever it changes.

use std::{
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use crate::{
    core_host,
    settings::{ColorMode, Settings, Theme},
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
        type Preferences = super::PreferencesRust;
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
    }
}

impl qobject::Preferences {
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
