// SPDX-License-Identifier: GPL-3.0-or-later
use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("app.nectarlink.spike").qml_files([
        QmlFile::from("qml/App.qml"),
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Theme.qml").singleton(true),
        QmlFile::from("qml/Icon.qml"),
        QmlFile::from("qml/QuickAction.qml"),
        QmlFile::from("qml/NotificationCard.qml"),
        QmlFile::from("qml/DeckKey.qml"),
        QmlFile::from("qml/StatsPanel.qml"),
        QmlFile::from("qml/MirrorPanel.qml"),
    ]))
    .qt_module("Quick")
    .include_dir("cpp")
    .cpp_files([
        "cpp/window_helpers.cpp",
        "cpp/frame_notifier.cpp",
        "cpp/memory_tools.h",
        "cpp/video_surface.h",
        "cpp/video_surface.cpp",
    ])
    .files(["src/bridge.rs", "src/native.rs", "src/video.rs"])
    .build();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR")
    {
        println!("cargo:rustc-link-arg=-fuse-ld=bfd");
        println!("cargo:rustc-link-lib=Qt6Quick");
        let sysroot_lib =
            std::path::Path::new(&manifest_dir).join("../../.qt6-sysroot/usr/lib/x86_64-linux-gnu");
        if let Ok(canon) = sysroot_lib.canonicalize() {
            println!("cargo:rustc-link-search=native={}", canon.display());
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", canon.display());
        }
    }
}
