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
}
