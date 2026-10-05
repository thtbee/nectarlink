// SPDX-License-Identifier: GPL-3.0-or-later
//! Builds the QML module `app.nectarlink` (QML files compiled ahead of time,
//! Rust QObjects, C++ helpers), compiles in the bundled fonts and links the
//! Windows libraries C++ uses.

use std::{
    fs,
    path::{Path, PathBuf},
};

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

const QML: &[&str] = &[
    "qml/App.qml",
    "qml/MainWindow.qml",
    "qml/components/Avatar.qml",
    "qml/components/Button.qml",
    "qml/components/CaptionButtons.qml",
    "qml/components/Card.qml",
    "qml/components/Chip.qml",
    "qml/components/CodeDigits.qml",
    "qml/components/Divider.qml",
    "qml/components/Icon.qml",
    "qml/components/IconButton.qml",
    "qml/components/ListRow.qml",
    "qml/components/LockChip.qml",
    "qml/components/NavItem.qml",
    "qml/components/NotificationItem.qml",
    "qml/components/QrCode.qml",
    "qml/components/Segmented.qml",
    "qml/components/Sheet.qml",
    "qml/components/Spinner.qml",
    "qml/components/StatusDot.qml",
    "qml/components/Toast.qml",
    "qml/components/Toggle.qml",
    "qml/components/TransferItem.qml",
    "qml/components/Txt.qml",
    "qml/pages/HomePage.qml",
    "qml/pages/PairingPanel.qml",
    "qml/pages/SettingsPage.qml",
    "qml/pages/WelcomePage.qml",
];

/// QML singletons.
const QML_SINGLETONS: &[&str] = &["qml/Tokens.qml", "qml/Theme.qml", "qml/Icons.qml"];

/// Bundled fonts (in `assets/fonts`), compiled in under `:/fonts/`.
const FONTS: &[&str] =
    &["Figtree.ttf", "InstrumentSerif-Regular.ttf", "SpaceMono-Regular.ttf", "SpaceMono-Bold.ttf"];

/// Writes a resource file listing the fonts (they live outside this crate,
/// so each needs an alias) and returns its path.
fn fonts_qrc() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
    let mut qrc = String::from("<RCC>\n  <qresource prefix=\"/fonts\">\n");
    for font in FONTS {
        let path = dir.join(font).canonicalize().unwrap_or_else(|e| panic!("missing font {font}: {e}"));
        println!("cargo::rerun-if-changed={}", path.display());
        // rcc takes forward slashes; strip the verbatim prefix canonicalize adds.
        let path = path.display().to_string().trim_start_matches(r"\\?\").replace('\\', "/");
        qrc.push_str(&format!("    <file alias=\"{font}\">{path}</file>\n"));
    }
    qrc.push_str("  </qresource>\n</RCC>\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("fonts.qrc");
    fs::write(&out, qrc).expect("write fonts.qrc");
    out
}

fn main() {
    let module = QmlModule::new("app.nectarlink")
        .qml_files(QML.iter().map(|f| QmlFile::from(*f)))
        .qml_files(QML_SINGLETONS.iter().map(|f| QmlFile::from(*f).singleton(true)));
    CxxQtBuilder::new_qml_module(module)
        .qt_module("Quick")
        .include_dir("cpp")
        .cpp_files(["cpp/app_helpers.cpp", "cpp/native_window.h", "cpp/native_window.cpp"])
        .qrc(fonts_qrc())
        .files([
            "src/bridge/native.rs",
            "src/bridge/app.rs",
            "src/bridge/devices.rs",
            "src/bridge/notifications.rs",
            "src/bridge/pairing.rs",
            "src/bridge/prefs.rs",
            "src/bridge/transfers.rs",
        ])
        .build();

    // The C++ window code calls DWM and user32 directly.
    println!("cargo:rustc-link-lib=dwmapi");
    println!("cargo:rustc-link-lib=user32");
}
