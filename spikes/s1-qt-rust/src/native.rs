// SPDX-License-Identifier: GPL-3.0-or-later
//! Small C++ helpers for Qt APIs that cxx-qt-lib doesn't wrap.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("window_helpers.h");

        /// Must be called before any QQuickWindow is created.
        fn enable_window_alpha();
    }
}
