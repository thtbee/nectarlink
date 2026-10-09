// SPDX-License-Identifier: GPL-3.0-or-later
#include "app_helpers.h"
#include "video_view.h"

#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QMimeData>
#include <QtCore/QPointer>
#include <QtCore/QUrl>
#include <QtCore/QtEnvironmentVariables>
#include <QtGui/QDrag>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QPixmap>
#include <QtGui/QPixmapCache>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlEngine>
#include <QtQuick/QQuickWindow>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <malloc.h>
#endif

namespace {
QIcon &appIcon()
{
    static QIcon icon;
    return icon;
}

QPointer<QQmlEngine> &qmlEngineRef()
{
    static QPointer<QQmlEngine> engine;
    return engine;
}
} // namespace

void register_qml_engine(QQmlEngine *engine)
{
    if (engine)
        qmlEngineRef() = engine;
}

void register_qml_app_engine(QQmlApplicationEngine &engine)
{
    qmlEngineRef() = &engine;
}

void prepare_qt()
{
    // Windows get an alpha channel so the Mica backdrop can show through, and
    // D3D swap chains are presented with DirectComposition, without which
    // transparent pixels render white (see spikes/s1-qt-rust, lesson 7).
    QQuickWindow::setDefaultAlphaBuffer(true);
    qputenv("QT_QPA_DISABLE_REDIRECTION_SURFACE", "1");
    qputenv("QSG_RENDER_LOOP", "basic");
    // Qt's V4 JS engine reserves 2 x QV4_JS_MAX_STACK_SIZE of committed RW
    // memory (8 MB by default). 512 KB per stack (1 MB total) is plenty for
    // UI bindings and saves 7 MB of private commit in the tray.
    qputenv("QV4_JS_MAX_STACK_SIZE", "524288");
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

void trim_memory_caches()
{
    clear_idle_video_frames();
    QPixmapCache::clear();
    if (QQmlEngine *engine = qmlEngineRef().data()) {
        engine->collectGarbage();
        engine->trimComponentCache();
        engine->collectGarbage();
    }
#ifdef _WIN32
    _heapmin();
    HANDLE heaps[64];
    const DWORD count = GetProcessHeaps(64, heaps);
    for (DWORD i = 0; i < count && i < 64; ++i) {
        HeapCompact(heaps[i], 0);
    }
    SetProcessWorkingSetSize(GetCurrentProcess(), static_cast<SIZE_T>(-1), static_cast<SIZE_T>(-1));
#endif
}

bool start_external_drag(rust::Str file_path, rust::Str text_fallback)
{
    const QString path = QString::fromUtf8(file_path.data(), static_cast<qsizetype>(file_path.size())).trimmed();
    const QString text = QString::fromUtf8(text_fallback.data(), static_cast<qsizetype>(text_fallback.size()));
    if (path.isEmpty() && text.isEmpty())
        return false;

    auto *mime = new QMimeData();
    QPixmap dragPixmap;
    if (!path.isEmpty() && QFileInfo::exists(path)) {
        mime->setUrls({QUrl::fromLocalFile(path)});
        QImage img(path);
        if (!img.isNull()) {
            mime->setImageData(img);
            dragPixmap = QPixmap::fromImage(img.scaled(96, 96, Qt::KeepAspectRatio, Qt::SmoothTransformation));
        }
    } else if (!text.isEmpty()) {
        mime->setText(text);
    } else {
        delete mime;
        return false;
    }

    QDrag drag(QGuiApplication::instance());
    drag.setMimeData(mime);
    if (!dragPixmap.isNull())
        drag.setPixmap(dragPixmap);
    return drag.exec(Qt::CopyAction) != Qt::IgnoreAction;
}


