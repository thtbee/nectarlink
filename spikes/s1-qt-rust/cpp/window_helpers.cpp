// SPDX-License-Identifier: GPL-3.0-or-later
#include "window_helpers.h"

#include <QtQuick/QQuickWindow>

void enable_window_alpha()
{
    // Gives Qt Quick windows an alpha channel so the Windows 11 Mica
    // backdrop can show through transparent areas.
    QQuickWindow::setDefaultAlphaBuffer(true);
}
