// SPDX-License-Identifier: GPL-3.0-or-later
//! Builds the QML module `app.nectarlink` (QML files compiled ahead of time,
//! Rust QObjects, C++ helpers) and links the Windows libraries C++ uses.

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
    "qml/components/QrCode.qml",
    "qml/components/Segmented.qml",
    "qml/components/Sheet.qml",
    "qml/components/Spinner.qml",
    "qml/components/StatusDot.qml",
    "qml/components/Toast.qml",
    "qml/components/Toggle.qml",
    "qml/components/Txt.qml",
    "qml/pages/HomePage.qml",
    "qml/pages/PairingPanel.qml",
    "qml/pages/SettingsPage.qml",
    "qml/pages/WelcomePage.qml",
];

/// QML singletons.
const QML_SINGLETONS: &[&str] = &["qml/Tokens.qml", "qml/Theme.qml", "qml/Icons.qml"];

fn main() {
    let module = QmlModule::new("app.nectarlink")
        .qml_files(QML.iter().map(|f| QmlFile::from(*f)))
        .qml_files(QML_SINGLETONS.iter().map(|f| QmlFile::from(*f).singleton(true)));
    CxxQtBuilder::new_qml_module(module)
        .qt_module("Quick")
        .include_dir("cpp")
        .cpp_files(["cpp/app_helpers.cpp", "cpp/native_window.h", "cpp/native_window.cpp"])
        .files([
            "src/bridge/native.rs",
            "src/bridge/app.rs",
            "src/bridge/devices.rs",
            "src/bridge/pairing.rs",
            "src/bridge/prefs.rs",
        ])
        .build();

    // The C++ window code calls DWM and user32 directly.
    println!("cargo:rustc-link-lib=dwmapi");
    println!("cargo:rustc-link-lib=user32");
}
