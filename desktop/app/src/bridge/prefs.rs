// SPDX-License-Identifier: GPL-3.0-or-later
//! `Preferences`: the app's look and behavior, saved whenever it changes.

use std::pin::Pin;

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

        // Save after any change (connected after loading, so loading doesn't
        // rewrite the file).
        self.as_mut().on_theme_changed(|p| p.save()).release();
        self.as_mut().on_seed_changed(|p| p.save()).release();
        self.as_mut().on_color_mode_changed(|p| p.save()).release();
        self.as_mut().on_backdrop_changed(|p| p.save()).release();
        self.as_mut().on_close_to_tray_changed(|p| p.save()).release();
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
        };
        if let Err(e) = settings.save(&core_host::host().data_dir) {
            tracing::warn!(error = %e, "can't save preferences");
        }
    }
}
