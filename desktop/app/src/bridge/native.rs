// SPDX-License-Identifier: GPL-3.0-or-later
//! Qt calls that cxx-qt-lib doesn't wrap (see `cpp/app_helpers.cpp`).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("app_helpers.h");
        include!("video_view.h");

        /// The latest picture of a video stream (32-bit BGRX rows), for
        /// `VideoView`s showing it. Any thread.
        fn video_frame(stream: &str, width: u32, height: u32, bgrx: &[u8]);
        /// The stream ended: its views go blank. Any thread.
        fn video_clear(stream: &str);

        /// Must run before `QGuiApplication` is created.
        fn prepare_qt();
        fn add_app_icon_image(size: i32, rgba: &[u8]);
        fn apply_app_icon();
        fn keep_running_without_windows();
        /// Registers the fonts compiled in under `:/fonts/`; returns how
        /// many loaded.
        fn load_bundled_fonts() -> i32;
        /// Clears Qt pixmap caches and trims the process working set.
        fn trim_memory_caches();
    }
}
