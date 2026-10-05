// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

// Prepares Qt Quick windows for a DWM system backdrop (Mica). Must be called
// before QGuiApplication and any QQuickWindow are created.
void enable_window_alpha();

// Keeps the process running with no windows (tray mode).
void set_quit_on_last_window_closed(bool quit);
