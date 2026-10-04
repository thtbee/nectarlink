// SPDX-License-Identifier: GPL-3.0-or-later
//! Spike S1: can Qt Quick + Rust deliver the Nectarlink design within the
//! performance budgets? See README.md for what is measured and the results.

mod bridge;
mod mica;
mod native;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

fn main() {
    let _ = bridge::STARTED.set(std::time::Instant::now());

    let mica = std::env::args().any(|a| a == "--mica");
    if mica {
        native::ffi::enable_window_alpha();
    }
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(app) = app.as_mut() {
        app.set_application_name(&QString::from("Nectarlink S1"));
    }
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/app/nectarlink/spike/qml/Main.qml"));
    }
    if mica {
        mica::apply_to_process_windows();
    }
    if let Some(app) = app.as_mut() {
        std::process::exit(app.exec());
    }
}
