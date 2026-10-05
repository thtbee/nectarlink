// SPDX-License-Identifier: GPL-3.0-or-later
#include "app_helpers.h"

#include <QtCore/QDir>
#include <QtCore/QtEnvironmentVariables>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QPixmap>
#include <QtQuick/QQuickWindow>

namespace {
QIcon &appIcon()
{
    static QIcon icon;
    return icon;
}
} // namespace

void prepare_qt()
{
    // Windows get an alpha channel so the Mica backdrop can show through, and
    // D3D swap chains are presented with DirectComposition, without which
    // transparent pixels render white (see spikes/s1-qt-rust, lesson 7).
    QQuickWindow::setDefaultAlphaBuffer(true);
    qputenv("QT_QPA_DISABLE_REDIRECTION_SURFACE", "1");
}

void add_app_icon_image(int32_t size, rust::Slice<const uint8_t> rgba)
{
    if (size <= 0 || rgba.size() != static_cast<size_t>(size) * size * 4)
        return;
    // Copy: the image must not reference Rust memory after this call.
    const QImage image =
        QImage(rgba.data(), size, size, size * 4, QImage::Format_RGBA8888).copy();
    appIcon().addPixmap(QPixmap::fromImage(image));
}

void apply_app_icon()
{
    QGuiApplication::setWindowIcon(appIcon());
}

void keep_running_without_windows()
{
    QGuiApplication::setQuitOnLastWindowClosed(false);
}

int32_t load_bundled_fonts()
{
    int32_t loaded = 0;
    const QDir dir(QStringLiteral(":/fonts"));
    for (const QString &name : dir.entryList({QStringLiteral("*.ttf")}, QDir::Files)) {
        if (QFontDatabase::addApplicationFont(dir.filePath(name)) >= 0)
            ++loaded;
    }
    return loaded;
}
