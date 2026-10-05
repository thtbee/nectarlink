// SPDX-License-Identifier: GPL-3.0-or-later
//! Small C++ helpers for Qt APIs that cxx-qt-lib doesn't wrap.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("window_helpers.h");

        /// Prepares windows for a DWM backdrop. Must be called before
        /// QGuiApplication and any QQuickWindow are created.
        fn enable_window_alpha();

        /// Keeps the process running with no windows (tray mode).
        fn set_quit_on_last_window_closed(quit: bool);
    }
}
