// SPDX-License-Identifier: GPL-3.0-or-later
#include "window_helpers.h"

#include <QtCore/QtEnvironmentVariables>
#include <QtQuick/QQuickWindow>

void enable_window_alpha()
{
    // Gives Qt Quick windows an alpha channel so the Windows 11 Mica
    // backdrop can show through transparent areas.
    QQuickWindow::setDefaultAlphaBuffer(true);
    // Qt's D3D11/D3D12 swap chains only blend with what is behind the window
    // when presented through DirectComposition, which Qt uses when the window
    // has no redirection surface. Without this, transparent pixels are white.
    qputenv("QT_QPA_DISABLE_REDIRECTION_SURFACE", "1");
}
