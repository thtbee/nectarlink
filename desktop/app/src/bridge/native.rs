// SPDX-License-Identifier: GPL-3.0-or-later
//! Qt calls that cxx-qt-lib doesn't wrap (see `cpp/app_helpers.cpp`).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("app_helpers.h");

        /// Must run before `QGuiApplication` is created.
        fn prepare_qt();
        fn add_app_icon_image(size: i32, rgba: &[u8]);
        fn apply_app_icon();
        fn keep_running_without_windows();
    }
}
