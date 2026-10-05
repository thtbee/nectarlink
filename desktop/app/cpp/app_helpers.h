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
