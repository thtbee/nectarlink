// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <rust/cxx.h>

#include <cstdint>

// Qt setup that must happen before QGuiApplication is created.
void prepare_qt();

// Adds one size of the app icon (straight-alpha RGBA, size x size).
void add_app_icon_image(int32_t size, rust::Slice<const uint8_t> rgba);
// Sets the icon built from the added images on all windows.
void apply_app_icon();

// Keeps the app running with no windows open (it lives in the tray).
void keep_running_without_windows();

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



