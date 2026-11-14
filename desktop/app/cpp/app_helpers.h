// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <rust/cxx.h>

#include <cstdint>
#include <memory>

class QGuiApplication;

// Creates the Qt application instance (`QApplication` on Linux so
// `QSystemTrayIcon` and `QMenu` work; `QGuiApplication` elsewhere).
std::unique_ptr<QGuiApplication> create_qt_app();

// Qt setup that must happen before QGuiApplication is created.
void prepare_qt(rust::Str log_dir);
void prepare_qt_without_log();
void prepare_qt_for_ui_check();
void set_ui_scale(int32_t percent);
int32_t qml_problem_count();

// Adds one size of the app icon (straight-alpha RGBA, size x size).
void add_app_icon_image(int32_t size, rust::Slice<const uint8_t> rgba);
// Sets the icon built from the added images on all windows.
void apply_app_icon();

// Keeps the app running with no windows open (it lives in the tray).
void keep_running_without_windows();

// Asks the session manager not to restart the app at the next login (Linux).
void stay_out_of_session_restore();

// Registers every font compiled in under :/fonts/ and returns how many
// loaded. Call after QGuiApplication is created.
int32_t load_bundled_fonts();

class QQmlEngine;
class QQmlApplicationEngine;

// Clears Qt pixmap/QML caches, compacts heaps, and asks Windows to trim unused
// pages from the process working set after windows are destroyed (tray mode).
void trim_memory_caches();

// Records the active QML engine so trim_memory_caches can run GC and clear
// component caches when windows are unloaded.
void register_qml_engine(QQmlEngine *engine);
void register_qml_app_engine(QQmlApplicationEngine &engine);

// Starts an OS-level drag-and-drop operation for a local file or text fallback
// so items dragged out of the Shelf can be dropped into Explorer or any app.
bool start_external_drag(rust::Str file_path, rust::Str text_fallback);

// Installs the NectarLink and Qt translations for `language` (a locale name
// such as "zh_CN", or "" for the system's), replacing those installed before.
// `dir` is the folder holding the executable, where windeployqt and the
// packaging script stage the .qm files. Call after QGuiApplication is
// created and before the QML engine loads, so that every qsTr() resolves on
// the first pass; called again later, it retranslates the open windows.
// Returns true when NectarLink's own translation was found and installed.
bool install_translator(rust::Str dir, rust::Str language);
rust::Vec<rust::String> system_ui_languages();

// `text` in the installed language (Qt's translate), for the strings the app
// builds in Rust; `text` itself when there is no translation.
rust::String translate(rust::Str context, rust::Str text);

// Decodes image bytes (PNG, JPEG, WebP, BMP, etc.) into width, height and ARGB32 bytes.
rust::Vec<uint8_t> decode_image(rust::Slice<const uint8_t> bytes);

// Native macOS clipboard support: changeCount, privacy detection, and direct reading/writing.
int64_t mac_clipboard_change_count();
rust::String mac_clipboard_read_text(bool &is_private);
bool mac_clipboard_write_text(rust::Str text, bool is_private);

int64_t mac_local_offset_ms(int64_t at_ms);
uint64_t mac_free_space(rust::Str path, bool &ok);

// Linux QSystemTrayIcon + QMenu, screen geometry, activation token, and QClipboard.
void linux_tray_setup(
    rust::Str icon_path,
    rust::Str tooltip,
    rust::Str open_label,
    rust::Str command_palette_label,
    rust::Str shelf_label,
    rust::Str take_photo_label,
    rust::Str scan_document_label,
    rust::Str find_phone_label,
    rust::Str open_link_label,
    rust::Str pause_notifications_label,
    rust::Str quit_label);
void linux_tray_set_tooltip(rust::Str tooltip);
void linux_tray_set_pause_label(rust::Str label);
void linux_tray_set_find_phone_enabled(bool enabled);
void linux_tray_teardown();
rust::String linux_tray_wait_action(uint32_t timeout_ms);
bool linux_tray_is_available();

bool linux_work_area(int32_t &x, int32_t &y, int32_t &w, int32_t &h);
bool linux_screen_geometry(int32_t &x, int32_t &y, int32_t &w, int32_t &h);
void linux_set_activation_token(rust::Str token);

uint64_t linux_clipboard_sequence();
uint64_t linux_clipboard_wait_change(uint64_t last_seq, uint32_t timeout_ms);
void linux_clipboard_notify_external();
bool linux_clipboard_read(
    bool check_privacy,
    bool &is_private,
    bool &has_files,
    rust::String &text,
    rust::String &html,
    rust::String &rtf,
    rust::Vec<uint8_t> &png_bytes);
rust::Vec<rust::String> linux_clipboard_read_files();
bool linux_clipboard_write_text(rust::Str text, rust::Str html, rust::Str rtf, bool is_private);
bool linux_clipboard_write_image_png(rust::Slice<const uint8_t> png);
bool linux_clipboard_write_files(rust::Slice<const rust::String> paths);
