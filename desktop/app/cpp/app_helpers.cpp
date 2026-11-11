// SPDX-License-Identifier: GPL-3.0-or-later
#include "app_helpers.h"
#include "video_view.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QLibraryInfo>
#include <QtCore/QLocale>
#include <QtCore/QMimeData>
#include <QtCore/QPointer>
#include <QtCore/QStorageInfo>
#include <QtCore/QStringList>
#include <QtCore/QTimeZone>
#include <QtCore/QTranslator>
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

#ifdef __linux__
#include <QtCore/QBuffer>
#include <QtCore/QMetaObject>
#include <QtCore/QThread>
#include <QtGui/QAction>
#include <QtGui/QClipboard>
#include <QtGui/QScreen>
#include <QtWidgets/QApplication>
#include <QtWidgets/QMenu>
#include <QtWidgets/QSystemTrayIcon>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <fstream>
#include <mutex>
#include <string>
#include <vector>
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

// Every QML engine registered, for retranslating them all when the
// language changes (gone ones drop out on their own).
QList<QPointer<QQmlEngine>> &allEngines()
{
    static QList<QPointer<QQmlEngine>> engines;
    return engines;
}

void remember(QQmlEngine *engine)
{
    auto &engines = allEngines();
    engines.removeIf([](const QPointer<QQmlEngine> &e) { return e.isNull(); });
    if (!engines.contains(engine))
        engines.append(engine);
}
} // namespace

void register_qml_engine(QQmlEngine *engine)
{
    if (engine) {
        qmlEngineRef() = engine;
        remember(engine);
    }
}

void register_qml_app_engine(QQmlApplicationEngine &engine)
{
    qmlEngineRef() = &engine;
    remember(&engine);
}

#include <QtCore/QDateTime>
#include <QtCore/QFile>
#include <QtCore/QTextStream>

#include <atomic>
#include <cstdio>

namespace {
std::atomic<int> &qmlProblems()
{
    static std::atomic<int> count { 0 };
    return count;
}

// A warning or error from QML: a component that didn't load, a binding or
// a script that failed. Also NativeWindow's, which CI should fail on too.
bool isQmlProblem(QtMsgType type, const QMessageLogContext &ctx, const QString &msg)
{
    if (type != QtWarningMsg && type != QtCriticalMsg && type != QtFatalMsg)
        return false;
    return (ctx.file && QByteArray(ctx.file).startsWith("qrc:/")) || msg.contains(QStringLiteral("qrc:/"))
        || msg.startsWith(QStringLiteral("NativeWindow:"));
}

// `--ui-check`: messages go to stderr, not the log file, and QML problems
// are counted.
void uiCheckMessageHandler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg)
{
    if (type == QtDebugMsg)
        return;
    if (isQmlProblem(type, ctx, msg))
        qmlProblems().fetch_add(1);
    std::fprintf(stderr, "[%d] %s (%s:%d)\n", int(type), qUtf8Printable(msg), ctx.file ? ctx.file : "", ctx.line);
    std::fflush(stderr);
}

// The app's logs folder (`<data>/logs`), so a test instance with its own
// data folder doesn't write into the installed app's.
QString &qmlLogDir()
{
    static QString dir;
    return dir;
}

void qtLogMessageHandler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg)
{
    if (type == QtDebugMsg)
        return;
    const QString &dirPath = qmlLogDir();
    if (dirPath.isEmpty())
        return;
    QDir().mkpath(dirPath);
    QFile file(dirPath + QStringLiteral("/qml.log"));
    if (file.open(QIODevice::WriteOnly | QIODevice::Append | QIODevice::Text)) {
        QTextStream ts(&file);
        ts << QDateTime::currentDateTimeUtc().toString(Qt::ISODate) << " [" << type << "] "
           << msg << " (" << (ctx.file ? ctx.file : "") << ":" << ctx.line << ")\n";
    }
}
} // namespace

void prepare_qt(rust::Str log_dir)
{
    qmlLogDir() = QString::fromUtf8(log_dir.data(), static_cast<qsizetype>(log_dir.size()));
    qInstallMessageHandler(qtLogMessageHandler);
    prepare_qt_without_log();
}

void set_ui_scale(int32_t percent)
{
    if (percent > 100 && percent <= 200 && !qEnvironmentVariableIsSet("QT_SCALE_FACTOR"))
        qputenv("QT_SCALE_FACTOR", QByteArray::number(percent / 100.0));
}

void prepare_qt_for_ui_check()
{
    qInstallMessageHandler(uiCheckMessageHandler);
    prepare_qt_without_log();
}

int32_t qml_problem_count()
{
    return qmlProblems().load();
}

void prepare_qt_without_log()
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

namespace {
// The translators installed now, replaced when the language changes.
QPointer<QTranslator> &qtTranslator()
{
    static QPointer<QTranslator> t;
    return t;
}
QPointer<QTranslator> &appTranslator()
{
    static QPointer<QTranslator> t;
    return t;
}

void drop(QPointer<QTranslator> &translator)
{
    if (translator) {
        QCoreApplication::removeTranslator(translator);
        delete translator.data();
    }
    translator = nullptr;
}
} // namespace

rust::Vec<rust::String> system_ui_languages()
{
    rust::Vec<rust::String> out;
    for (const QString &tag : QLocale::system().uiLanguages())
        out.push_back(rust::String(tag.toStdString()));
    return out;
}

bool install_translator(rust::Str dir, rust::Str language)
{
    const QString folder = QString::fromUtf8(dir.data(), static_cast<qsizetype>(dir.size()));
    const QString chosen = QString::fromUtf8(language.data(), static_cast<qsizetype>(language.size()));
    const QLocale locale = chosen.isEmpty() ? QLocale::system() : QLocale(chosen);
    QLocale::setDefault(locale);
    drop(qtTranslator());
    drop(appTranslator());

    // Qt's own strings: file dialogs, context menus, the standard buttons.
    // windeployqt stages a merged `qt_<locale>.qm` in `translations/` beside the
    // executable, while a build from source finds `qtbase_<locale>.qm` in Qt's
    // own folder, so try both names in both places.
    const QStringList qtDirs{
        folder + QStringLiteral("/translations"),
        QLibraryInfo::path(QLibraryInfo::TranslationsPath),
    };
    for (const QString &qtDir : qtDirs) {
        for (const char *name : {"qtbase", "qt"}) {
            auto *qt = new QTranslator(QCoreApplication::instance());
            if (qt->load(locale, QString::fromLatin1(name), QStringLiteral("_"), qtDir)) {
                QCoreApplication::installTranslator(qt);
                qtTranslator() = qt;
                break;
            }
            delete qt;
        }
        if (qtTranslator())
            break;
    }

    // NectarLink's own strings, staged beside the executable. A locale with no
    // translation (`nectarlink_es_ES.qm` missing), or English, stays English.
    bool installed = false;
    auto *app = new QTranslator(QCoreApplication::instance());
    if (app->load(locale, QStringLiteral("nectarlink"), QStringLiteral("_"), folder)) {
        installed = QCoreApplication::installTranslator(app);
        appTranslator() = app;
    } else {
        delete app;
    }

    // Windows already open show the new language at once.
    for (const QPointer<QQmlEngine> &engine : allEngines()) {
        if (engine)
            engine->retranslate();
    }
    return installed;
}

rust::String translate(rust::Str context, rust::Str text)
{
    const std::string c(context.data(), context.size());
    const std::string t(text.data(), text.size());
    return rust::String(QCoreApplication::translate(c.c_str(), t.c_str()).toStdString());
}

rust::Vec<uint8_t> decode_image(rust::Slice<const uint8_t> bytes)
{
    rust::Vec<uint8_t> result;
    if (bytes.empty())
        return result;
    QImage img;
    if (!img.loadFromData(bytes.data(), static_cast<int>(bytes.size())))
        return result;
    const QImage argb = img.convertToFormat(QImage::Format_ARGB32);
    const uint32_t width = static_cast<uint32_t>(argb.width());
    const uint32_t height = static_cast<uint32_t>(argb.height());
    const size_t byteCount = static_cast<size_t>(argb.sizeInBytes());
    result.reserve(8 + byteCount);
    for (int i = 0; i < 4; ++i)
        result.push_back(static_cast<uint8_t>((width >> (i * 8)) & 0xFF));
    for (int i = 0; i < 4; ++i)
        result.push_back(static_cast<uint8_t>((height >> (i * 8)) & 0xFF));
    const uint8_t *bits = argb.constBits();
    for (size_t i = 0; i < byteCount; ++i)
        result.push_back(bits[i]);
    return result;
}

#ifdef __APPLE__
#include <objc/runtime.h>
#include <objc/message.h>

extern "C" void *objc_autoreleasePoolPush(void);
extern "C" void objc_autoreleasePoolPop(void *pool);

namespace {
struct AutoReleasePool {
    void *pool;
    AutoReleasePool() : pool(objc_autoreleasePoolPush()) {}
    ~AutoReleasePool() { objc_autoreleasePoolPop(pool); }
};
} // namespace

int64_t mac_clipboard_change_count()
{
    AutoReleasePool pool;
    id pboardClass = reinterpret_cast<id>(objc_getClass("NSPasteboard"));
    if (!pboardClass)
        return 0;
    id pboard = reinterpret_cast<id (*)(id, SEL)>(objc_msgSend)(pboardClass, sel_registerName("generalPasteboard"));
    if (!pboard)
        return 0;
    return reinterpret_cast<int64_t (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("changeCount"));
}

rust::String mac_clipboard_read_text(bool &is_private)
{
    AutoReleasePool pool;
    is_private = false;
    id pboardClass = reinterpret_cast<id>(objc_getClass("NSPasteboard"));
    if (!pboardClass)
        return rust::String();
    id pboard = reinterpret_cast<id (*)(id, SEL)>(objc_msgSend)(pboardClass, sel_registerName("generalPasteboard"));
    if (!pboard)
        return rust::String();

    id strClass = reinterpret_cast<id>(objc_getClass("NSString"));
    if (!strClass)
        return rust::String();

    id concealed = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), "org.nspasteboard.ConcealedType");
    id transient = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), "org.nspasteboard.TransientType");
    id autoGen = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), "org.nspasteboard.AutoGeneratedType");
    id plainTextType = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), "public.utf8-plain-text");

    for (int attempt = 0; attempt < 2; ++attempt) {
        int64_t countBefore = reinterpret_cast<int64_t (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("changeCount"));

        id types = reinterpret_cast<id (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("types"));
        if (types) {
            if ((concealed && reinterpret_cast<bool (*)(id, SEL, id)>(objc_msgSend)(types, sel_registerName("containsObject:"), concealed))
                || (transient && reinterpret_cast<bool (*)(id, SEL, id)>(objc_msgSend)(types, sel_registerName("containsObject:"), transient))
                || (autoGen && reinterpret_cast<bool (*)(id, SEL, id)>(objc_msgSend)(types, sel_registerName("containsObject:"), autoGen))) {
                int64_t countAfter = reinterpret_cast<int64_t (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("changeCount"));
                if (countBefore == countAfter) {
                    is_private = true;
                    return rust::String();
                }
                continue;
            }
        }

        id str = reinterpret_cast<id (*)(id, SEL, id)>(objc_msgSend)(pboard, sel_registerName("stringForType:"), plainTextType);
        int64_t countAfter = reinterpret_cast<int64_t (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("changeCount"));
        if (countBefore != countAfter) {
            continue;
        }

        if (str) {
            const char *utf8 = reinterpret_cast<const char * (*)(id, SEL)>(objc_msgSend)(str, sel_registerName("UTF8String"));
            if (utf8)
                return rust::String(utf8);
        }
        return rust::String();
    }

    is_private = false;
    return rust::String();
}

bool mac_clipboard_write_text(rust::Str text, bool is_private)
{
    AutoReleasePool pool;
    id pboardClass = reinterpret_cast<id>(objc_getClass("NSPasteboard"));
    if (!pboardClass)
        return false;
    id pboard = reinterpret_cast<id (*)(id, SEL)>(objc_msgSend)(pboardClass, sel_registerName("generalPasteboard"));
    if (!pboard)
        return false;

    reinterpret_cast<void (*)(id, SEL)>(objc_msgSend)(pboard, sel_registerName("clearContents"));

    id strClass = reinterpret_cast<id>(objc_getClass("NSString"));
    if (!strClass)
        return false;

    const std::string text_str(text.data(), text.size());
    id nsStr = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), text_str.c_str());
    if (!nsStr)
        return false;

    id plainTextType = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
        strClass, sel_registerName("stringWithUTF8String:"), "public.utf8-plain-text");

    id arrClass = reinterpret_cast<id>(objc_getClass("NSArray"));
    if (!arrClass)
        return false;

    if (is_private) {
        id concealed = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
            strClass, sel_registerName("stringWithUTF8String:"), "org.nspasteboard.ConcealedType");
        id transient = reinterpret_cast<id (*)(id, SEL, const char *)>(objc_msgSend)(
            strClass, sel_registerName("stringWithUTF8String:"), "org.nspasteboard.TransientType");
        const id items[] = { plainTextType, concealed, transient };
        id types = reinterpret_cast<id (*)(id, SEL, const id *, unsigned long)>(objc_msgSend)(
            arrClass, sel_registerName("arrayWithObjects:count:"), items, 3);
        reinterpret_cast<void (*)(id, SEL, id, id)>(objc_msgSend)(
            pboard, sel_registerName("declareTypes:owner:"), types, nullptr);
        bool ok = reinterpret_cast<bool (*)(id, SEL, id, id)>(objc_msgSend)(
            pboard, sel_registerName("setString:forType:"), nsStr, plainTextType);
        if (!ok)
            return false;
        id dataClass = reinterpret_cast<id>(objc_getClass("NSData"));
        if (dataClass) {
            id emptyData = reinterpret_cast<id (*)(id, SEL)>(objc_msgSend)(dataClass, sel_registerName("data"));
            reinterpret_cast<bool (*)(id, SEL, id, id)>(objc_msgSend)(
                pboard, sel_registerName("setData:forType:"), emptyData, concealed);
            reinterpret_cast<bool (*)(id, SEL, id, id)>(objc_msgSend)(
                pboard, sel_registerName("setData:forType:"), emptyData, transient);
        }
        return true;
    } else {
        const id items[] = { plainTextType };
        id types = reinterpret_cast<id (*)(id, SEL, const id *, unsigned long)>(objc_msgSend)(
            arrClass, sel_registerName("arrayWithObjects:count:"), items, 1);
        reinterpret_cast<void (*)(id, SEL, id, id)>(objc_msgSend)(
            pboard, sel_registerName("declareTypes:owner:"), types, nullptr);
        return reinterpret_cast<bool (*)(id, SEL, id, id)>(objc_msgSend)(
            pboard, sel_registerName("setString:forType:"), nsStr, plainTextType);
    }
}
#else
int64_t mac_clipboard_change_count()
{
    return 0;
}

rust::String mac_clipboard_read_text(bool &is_private)
{
    is_private = false;
    return rust::String();
}

bool mac_clipboard_write_text(rust::Str /*text*/, bool /*is_private*/)
{
    return false;
}
#endif

int64_t mac_local_offset_ms(int64_t at_ms)
{
    const QDateTime dt = QDateTime::fromMSecsSinceEpoch(at_ms, QTimeZone::systemTimeZone());
    return static_cast<int64_t>(dt.offsetFromUtc()) * 1000;
}

uint64_t mac_free_space(rust::Str path, bool &ok)
{
    ok = false;
    const std::string pathStr(path.data(), path.size());
    QString qpath = QString::fromStdString(pathStr);
    QFileInfo info(qpath);
    while (!info.exists() && !qpath.isEmpty()) {
        const QString parent = info.dir().absolutePath();
        if (parent == qpath)
            break;
        qpath = parent;
        info.setFile(qpath);
    }
    const QStorageInfo storage(qpath);
    if (!storage.isValid() || !storage.isReady())
        return 0;
    const qint64 available = storage.bytesAvailable();
    if (available < 0)
        return 0;
    ok = true;
    return static_cast<uint64_t>(available);
}

#ifdef __linux__
namespace {
std::atomic<bool> &qtEventLoopRunning()
{
    static std::atomic<bool> running { false };
    return running;
}

struct LinuxClipboardWatch {
    std::mutex mutex;
    std::condition_variable cv;
    std::atomic<uint64_t> seq { 1 };
};

LinuxClipboardWatch &linuxClipboardWatch()
{
    static LinuxClipboardWatch *watch = new LinuxClipboardWatch();
    return *watch;
}

struct LinuxTrayState {
    QPointer<QSystemTrayIcon> icon;
    QPointer<QMenu> menu;
    QPointer<QAction> takePhotoAction;
    QPointer<QAction> scanDocumentAction;
    QPointer<QAction> findPhoneAction;
    QPointer<QAction> openLinkAction;
    QPointer<QAction> pauseAction;
    bool phoneActionsEnabled = false;
    std::mutex mutex;
    std::condition_variable cv;
    std::deque<std::string> actions;
};

LinuxTrayState &linuxTray()
{
    static LinuxTrayState *state = new LinuxTrayState();
    return *state;
}

void pushTrayAction(const char *action)
{
    auto &st = linuxTray();
    {
        std::lock_guard<std::mutex> lock(st.mutex);
        st.actions.emplace_back(action);
    }
    st.cv.notify_all();
}

template <typename F>
void runOnQtMainThread(F &&fn)
{
    QCoreApplication *app = QCoreApplication::instance();
    if (!app)
        return;
    if (QThread::currentThread() == app->thread()) {
        fn();
    } else {
        QMetaObject::invokeMethod(app, std::forward<F>(fn), Qt::QueuedConnection);
    }
}

template <typename F>
bool runOnQtMainThreadSync(F &&fn, uint32_t timeout_ms = 500)
{
    QCoreApplication *app = QCoreApplication::instance();
    if (!app)
        return false;
    if (QThread::currentThread() == app->thread()) {
        fn();
        return true;
    }
    if (!qtEventLoopRunning().load(std::memory_order_acquire))
        return false;
    struct SyncState {
        std::mutex mutex;
        std::condition_variable cv;
        bool done = false;
    };
    auto state = std::make_shared<SyncState>();
    auto task = std::make_shared<std::decay_t<F>>(std::forward<F>(fn));
    QMetaObject::invokeMethod(
        app,
        [state, task]() {
            (*task)();
            {
                std::lock_guard<std::mutex> lock(state->mutex);
                state->done = true;
            }
            state->cv.notify_all();
        },
        Qt::QueuedConnection);
    std::unique_lock<std::mutex> lock(state->mutex);
    return state->cv.wait_for(lock, std::chrono::milliseconds(timeout_ms), [&state]() {
        return state->done;
    });
}
} // namespace
#endif

#ifdef __linux__
namespace {
// This process's arguments, as the kernel has them.
std::vector<std::string> processArguments()
{
    std::vector<std::string> args;
    std::ifstream cmdline("/proc/self/cmdline", std::ios::binary);
    std::string arg;
    while (std::getline(cmdline, arg, '\0'))
        args.push_back(arg);
    if (args.empty())
        args.emplace_back("nectarlink");
    return args;
}
} // namespace
#endif

std::unique_ptr<QGuiApplication> create_qt_app()
{
#ifdef __linux__
    // The real arguments, as on the other systems: QML reads --minimized and
    // --send-to from them (#336). Qt keeps pointers to them, so they live on.
    static std::vector<std::string> s_args = processArguments();
    static std::vector<char *> s_argv_list = [] {
        std::vector<char *> list;
        for (std::string &arg : s_args)
            list.push_back(arg.data());
        list.push_back(nullptr);
        return list;
    }();
    static int s_argc = static_cast<int>(s_args.size());
    auto app = std::unique_ptr<QGuiApplication>(new QApplication(s_argc, s_argv_list.data()));
    // On Wayland this is the app id, which is how the dock finds our icon and name.
    QGuiApplication::setDesktopFileName(QStringLiteral("nectarlink"));
    QObject::connect(app.get(), &QCoreApplication::aboutToQuit, app.get(), []() {
        qtEventLoopRunning().store(false, std::memory_order_release);
    });
    QMetaObject::invokeMethod(
        app.get(),
        []() {
            qtEventLoopRunning().store(true, std::memory_order_release);
        },
        Qt::QueuedConnection);
    if (QClipboard *cb = QGuiApplication::clipboard()) {
        QObject::connect(cb, &QClipboard::dataChanged, cb, []() {
            linux_clipboard_notify_external();
        });
    }
    return app;
#else
    static int s_argc = 1;
    static char s_arg0[] = "nectarlink";
    static char *s_argv[] = { s_arg0, nullptr };
    return std::make_unique<QGuiApplication>(s_argc, s_argv);
#endif
}

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
    rust::Str quit_label)
{
#ifdef __linux__
    const QString iconPath = QString::fromUtf8(icon_path.data(), static_cast<qsizetype>(icon_path.size()));
    const QString tooltipStr = QString::fromUtf8(tooltip.data(), static_cast<qsizetype>(tooltip.size()));
    const QString openStr = QString::fromUtf8(open_label.data(), static_cast<qsizetype>(open_label.size()));
    const QString paletteStr = QString::fromUtf8(command_palette_label.data(), static_cast<qsizetype>(command_palette_label.size()));
    const QString shelfStr = QString::fromUtf8(shelf_label.data(), static_cast<qsizetype>(shelf_label.size()));
    const QString photoStr = QString::fromUtf8(take_photo_label.data(), static_cast<qsizetype>(take_photo_label.size()));
    const QString scanStr = QString::fromUtf8(scan_document_label.data(), static_cast<qsizetype>(scan_document_label.size()));
    const QString findPhoneStr = QString::fromUtf8(find_phone_label.data(), static_cast<qsizetype>(find_phone_label.size()));
    const QString openLinkStr = QString::fromUtf8(open_link_label.data(), static_cast<qsizetype>(open_link_label.size()));
    const QString pauseStr = QString::fromUtf8(pause_notifications_label.data(), static_cast<qsizetype>(pause_notifications_label.size()));
    const QString quitStr = QString::fromUtf8(quit_label.data(), static_cast<qsizetype>(quit_label.size()));

    runOnQtMainThread([=]() {
        if (!qobject_cast<QApplication *>(QCoreApplication::instance()))
            return;
        auto &st = linuxTray();
        if (st.icon) {
            st.icon->hide();
            delete st.icon.data();
        }
        if (st.menu) {
            delete st.menu.data();
        }
        {
            std::lock_guard<std::mutex> lock(st.mutex);
            st.actions.clear();
        }

        // Menu order matches win/tray.rs:
        // Take Photo, Scan Document, Separator, Open, Command Palette, Shelf, Find Phone, Open Link, Separator, Quit.
        const bool enabled = st.phoneActionsEnabled;
        auto *menu = new QMenu();
        QAction *takePhoto = menu->addAction(photoStr);
        takePhoto->setEnabled(enabled);
        QObject::connect(takePhoto, &QAction::triggered, menu, []() { pushTrayAction("TAKE_PHOTO"); });

        QAction *scanDoc = menu->addAction(scanStr);
        scanDoc->setEnabled(enabled);
        QObject::connect(scanDoc, &QAction::triggered, menu, []() { pushTrayAction("SCAN_DOCUMENT"); });

        menu->addSeparator();
        QObject::connect(menu->addAction(openStr), &QAction::triggered, menu, []() { pushTrayAction("OPEN"); });
        QObject::connect(menu->addAction(paletteStr), &QAction::triggered, menu, []() { pushTrayAction("COMMAND_PALETTE"); });
        QObject::connect(menu->addAction(shelfStr), &QAction::triggered, menu, []() { pushTrayAction("SHELF"); });

        QAction *findPhone = menu->addAction(findPhoneStr);
        findPhone->setEnabled(enabled);
        QObject::connect(findPhone, &QAction::triggered, menu, []() { pushTrayAction("FIND_PHONE"); });

        QAction *openLink = menu->addAction(openLinkStr);
        openLink->setEnabled(enabled);
        QObject::connect(openLink, &QAction::triggered, menu, []() { pushTrayAction("OPEN_LINK"); });

        QAction *pause = menu->addAction(pauseStr);
        QObject::connect(pause, &QAction::triggered, menu, []() { pushTrayAction("PAUSE_NOTIFICATIONS"); });

        menu->addSeparator();
        QObject::connect(menu->addAction(quitStr), &QAction::triggered, menu, []() { pushTrayAction("QUIT"); });

        auto *tray = new QSystemTrayIcon(QCoreApplication::instance());
        QIcon icon = appIcon();
        if (icon.isNull() && !iconPath.isEmpty())
            icon = QIcon(iconPath);
        tray->setIcon(icon);
        tray->setToolTip(tooltipStr);
        tray->setContextMenu(menu);
        QObject::connect(tray, &QSystemTrayIcon::activated, tray, [](QSystemTrayIcon::ActivationReason reason) {
            if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick)
                pushTrayAction("OPEN");
        });
        tray->show();

        st.icon = tray;
        st.menu = menu;
        st.takePhotoAction = takePhoto;
        st.scanDocumentAction = scanDoc;
        st.findPhoneAction = findPhone;
        st.openLinkAction = openLink;
        st.pauseAction = pause;
    });
#else
    (void)icon_path;
    (void)tooltip;
    (void)open_label;
    (void)command_palette_label;
    (void)shelf_label;
    (void)take_photo_label;
    (void)scan_document_label;
    (void)find_phone_label;
    (void)open_link_label;
    (void)pause_notifications_label;
    (void)quit_label;
#endif
}

void linux_tray_set_pause_label(rust::Str label)
{
#ifdef __linux__
    const QString text = QString::fromUtf8(label.data(), static_cast<qsizetype>(label.size()));
    runOnQtMainThread([text]() {
        if (auto *action = linuxTray().pauseAction.data())
            action->setText(text);
    });
#else
    (void)label;
#endif
}

void linux_tray_set_tooltip(rust::Str tooltip)
{
#ifdef __linux__
    const QString tooltipStr = QString::fromUtf8(tooltip.data(), static_cast<qsizetype>(tooltip.size()));
    runOnQtMainThread([tooltipStr]() {
        if (auto *icon = linuxTray().icon.data())
            icon->setToolTip(tooltipStr);
    });
#else
    (void)tooltip;
#endif
}

void linux_tray_set_find_phone_enabled(bool enabled)
{
#ifdef __linux__
    runOnQtMainThread([enabled]() {
        auto &st = linuxTray();
        st.phoneActionsEnabled = enabled;
        if (auto *act = st.takePhotoAction.data())
            act->setEnabled(enabled);
        if (auto *act = st.scanDocumentAction.data())
            act->setEnabled(enabled);
        if (auto *act = st.findPhoneAction.data())
            act->setEnabled(enabled);
        if (auto *act = st.openLinkAction.data())
            act->setEnabled(enabled);
    });
#else
    (void)enabled;
#endif
}

void linux_tray_teardown()
{
#ifdef __linux__
    auto &st = linuxTray();
    st.cv.notify_all();
    runOnQtMainThread([]() {
        auto &s = linuxTray();
        if (s.icon) {
            s.icon->hide();
            delete s.icon.data();
        }
        if (s.menu) {
            delete s.menu.data();
        }
        s.takePhotoAction = nullptr;
        s.scanDocumentAction = nullptr;
        s.findPhoneAction = nullptr;
        s.openLinkAction = nullptr;
    });
#endif
}

rust::String linux_tray_wait_action(uint32_t timeout_ms)
{
#ifdef __linux__
    auto &st = linuxTray();
    std::unique_lock<std::mutex> lock(st.mutex);
    if (st.actions.empty() && timeout_ms > 0) {
        st.cv.wait_for(lock, std::chrono::milliseconds(timeout_ms));
    }
    if (!st.actions.empty()) {
        std::string next = std::move(st.actions.front());
        st.actions.pop_front();
        return rust::String(next);
    }
#else
    (void)timeout_ms;
#endif
    return rust::String();
}

bool linux_tray_is_available()
{
#ifdef __linux__
    bool available = false;
    if (runOnQtMainThreadSync([&available]() {
            available = QSystemTrayIcon::isSystemTrayAvailable();
        })) {
        return available;
    }
#endif
    return false;
}

bool linux_work_area(int32_t &x, int32_t &y, int32_t &w, int32_t &h)
{
#ifdef __linux__
    bool ok = false;
    int32_t rx = 0, ry = 0, rw = 0, rh = 0;
    if (runOnQtMainThreadSync([&]() {
            if (QScreen *screen = QGuiApplication::primaryScreen()) {
                const QRect avail = screen->availableGeometry();
                const qreal dpr = screen->devicePixelRatio() > 0.0 ? screen->devicePixelRatio() : 1.0;
                if (avail.width() > 0 && avail.height() > 0) {
                    rx = static_cast<int32_t>(avail.x() * dpr);
                    ry = static_cast<int32_t>(avail.y() * dpr);
                    rw = static_cast<int32_t>(avail.width() * dpr);
                    rh = static_cast<int32_t>(avail.height() * dpr);
                    ok = true;
                }
            }
        })
        && ok) {
        x = rx;
        y = ry;
        w = rw;
        h = rh;
        return true;
    }
#else
    (void)x;
    (void)y;
    (void)w;
    (void)h;
#endif
    return false;
}

bool linux_screen_geometry(int32_t &x, int32_t &y, int32_t &w, int32_t &h)
{
#ifdef __linux__
    bool ok = false;
    int32_t rx = 0, ry = 0, rw = 0, rh = 0;
    if (runOnQtMainThreadSync([&]() {
            if (QScreen *screen = QGuiApplication::primaryScreen()) {
                const QRect geom = screen->geometry();
                const qreal dpr = screen->devicePixelRatio() > 0.0 ? screen->devicePixelRatio() : 1.0;
                if (geom.width() > 0 && geom.height() > 0) {
                    rx = static_cast<int32_t>(geom.x() * dpr);
                    ry = static_cast<int32_t>(geom.y() * dpr);
                    rw = static_cast<int32_t>(geom.width() * dpr);
                    rh = static_cast<int32_t>(geom.height() * dpr);
                    ok = true;
                }
            }
        })
        && ok) {
        x = rx;
        y = ry;
        w = rw;
        h = rh;
        return true;
    }
#else
    (void)x;
    (void)y;
    (void)w;
    (void)h;
#endif
    return false;
}

void linux_set_activation_token(rust::Str token)
{
#ifdef __linux__
    if (token.empty())
        return;
    const QByteArray tok(token.data(), static_cast<qsizetype>(token.size()));
    runOnQtMainThread([tok]() {
        qputenv("XDG_ACTIVATION_TOKEN", tok);
    });
#else
    (void)token;
#endif
}

uint64_t linux_clipboard_sequence()
{
#ifdef __linux__
    return linuxClipboardWatch().seq.load(std::memory_order_acquire);
#else
    return 0;
#endif
}

uint64_t linux_clipboard_wait_change(uint64_t last_seq, uint32_t timeout_ms)
{
#ifdef __linux__
    auto &w = linuxClipboardWatch();
    uint64_t cur = w.seq.load(std::memory_order_acquire);
    if (cur != last_seq || timeout_ms == 0)
        return cur;
    std::unique_lock<std::mutex> lock(w.mutex);
    w.cv.wait_for(lock, std::chrono::milliseconds(timeout_ms), [&w, last_seq]() {
        return w.seq.load(std::memory_order_acquire) != last_seq;
    });
    return w.seq.load(std::memory_order_acquire);
#else
    (void)last_seq;
    (void)timeout_ms;
    return 0;
#endif
}

void linux_clipboard_notify_external()
{
#ifdef __linux__
    auto &w = linuxClipboardWatch();
    {
        std::lock_guard<std::mutex> lock(w.mutex);
        w.seq.fetch_add(1, std::memory_order_release);
    }
    w.cv.notify_all();
#endif
}

bool linux_clipboard_read(
    bool check_privacy,
    bool &is_private,
    bool &has_files,
    rust::String &text,
    rust::String &html,
    rust::String &rtf,
    rust::Vec<uint8_t> &png_bytes)
{
    is_private = false;
    has_files = false;
    text = rust::String();
    html = rust::String();
    rtf = rust::String();
    png_bytes.clear();
#ifdef __linux__
    bool ran = false;
    bool priv = false;
    bool files = false;
    std::string outText;
    std::string outHtml;
    std::string outRtf;
    QByteArray outPng;

    if (!runOnQtMainThreadSync([&]() {
            QClipboard *cb = QGuiApplication::clipboard();
            if (!cb)
                return;
            ran = true;
            const QMimeData *mime = cb->mimeData(QClipboard::Clipboard);
            if (!mime)
                return;

            const QStringList formats = mime->formats();
            if (check_privacy) {
                for (const QString &fmt : formats) {
                    if (fmt.compare(QStringLiteral("x-gnome-privacy-hint"), Qt::CaseInsensitive) == 0
                        || fmt.compare(QStringLiteral("ExcludeClipboardContentFromMonitorProcessing"), Qt::CaseInsensitive) == 0
                        || fmt.compare(QStringLiteral("Clipboard Viewer Ignore"), Qt::CaseInsensitive) == 0) {
                        priv = true;
                        return;
                    }
                    if (fmt.compare(QStringLiteral("x-kde-passwordManagerHint"), Qt::CaseInsensitive) == 0) {
                        const QByteArray val = mime->data(fmt).trimmed();
                        if (val.isEmpty() || QString::fromUtf8(val).trimmed().compare(QStringLiteral("secret"), Qt::CaseInsensitive) == 0) {
                            priv = true;
                            return;
                        }
                    }
                }
            }

            if (mime->hasFormat(QStringLiteral("image/png"))) {
                outPng = mime->data(QStringLiteral("image/png"));
                if (!outPng.isEmpty())
                    return;
            }
            if (mime->hasImage()) {
                const QImage img = qvariant_cast<QImage>(mime->imageData());
                if (!img.isNull()) {
                    QBuffer buf(&outPng);
                    if (buf.open(QIODevice::WriteOnly) && img.save(&buf, "PNG")) {
                        return;
                    }
                    outPng.clear();
                }
            }

            for (const QString &fmt : formats) {
                if (fmt.compare(QStringLiteral("x-special/gnome-copied-files"), Qt::CaseInsensitive) == 0
                    || fmt.compare(QStringLiteral("x-special/mate-copied-files"), Qt::CaseInsensitive) == 0) {
                    files = true;
                    return;
                }
            }
            if (mime->hasUrls()) {
                for (const QUrl &u : mime->urls()) {
                    if (u.isLocalFile()) {
                        files = true;
                        return;
                    }
                }
            }

            if (mime->hasText()) {
                outText = mime->text().toStdString();
            }
            if (mime->hasHtml()) {
                outHtml = mime->html().toStdString();
            }
            if (mime->hasFormat(QStringLiteral("text/rtf"))) {
                const QByteArray r = mime->data(QStringLiteral("text/rtf"));
                outRtf.assign(r.constData(), static_cast<size_t>(r.size()));
            } else if (mime->hasFormat(QStringLiteral("application/rtf"))) {
                const QByteArray r = mime->data(QStringLiteral("application/rtf"));
                outRtf.assign(r.constData(), static_cast<size_t>(r.size()));
            }
        })) {
        return false;
    }
    if (!ran)
        return false;

    is_private = priv;
    has_files = files;
    text = rust::String(outText);
    html = rust::String(outHtml);
    rtf = rust::String(outRtf);
    if (!outPng.isEmpty()) {
        png_bytes.reserve(static_cast<size_t>(outPng.size()));
        const auto *raw = reinterpret_cast<const uint8_t *>(outPng.constData());
        for (qsizetype i = 0; i < outPng.size(); ++i)
            png_bytes.push_back(raw[i]);
    }
    return true;
#else
    (void)check_privacy;
    return false;
#endif
}

rust::Vec<rust::String> linux_clipboard_read_files()
{
    rust::Vec<rust::String> result;
#ifdef __linux__
    std::vector<std::string> paths;
    if (runOnQtMainThreadSync([&]() {
            QClipboard *cb = QGuiApplication::clipboard();
            if (!cb)
                return;
            const QMimeData *mime = cb->mimeData(QClipboard::Clipboard);
            if (!mime)
                return;
            if (mime->hasUrls()) {
                for (const QUrl &u : mime->urls()) {
                    if (u.isLocalFile()) {
                        const QString local = u.toLocalFile();
                        if (!local.isEmpty() && QFileInfo::exists(local))
                            paths.push_back(local.toStdString());
                    }
                }
            }
            if (paths.empty()) {
                for (const char *fmt : { "x-special/gnome-copied-files", "x-special/mate-copied-files", "text/uri-list" }) {
                    const QString qfmt = QString::fromLatin1(fmt);
                    if (mime->hasFormat(qfmt)) {
                        const QString raw = QString::fromUtf8(mime->data(qfmt));
                        for (const QString &line : raw.split(QLatin1Char('\n'))) {
                            const QString trimmed = line.trimmed();
                            if (trimmed.startsWith(QStringLiteral("file://"))) {
                                const QUrl u(trimmed);
                                if (u.isLocalFile()) {
                                    const QString local = u.toLocalFile();
                                    if (!local.isEmpty() && QFileInfo::exists(local))
                                        paths.push_back(local.toStdString());
                                }
                            }
                        }
                        if (!paths.empty())
                            break;
                    }
                }
            }
        })) {
        for (const std::string &p : paths)
            result.push_back(rust::String(p));
    }
#endif
    return result;
}

bool linux_clipboard_write_text(rust::Str text, rust::Str html, rust::Str rtf, bool is_private)
{
#ifdef __linux__
    const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
    const QString qhtml = QString::fromUtf8(html.data(), static_cast<qsizetype>(html.size()));
    const QByteArray qrtf(rtf.data(), static_cast<qsizetype>(rtf.size()));
    bool ok = false;
    if (runOnQtMainThreadSync([&]() {
            QClipboard *cb = QGuiApplication::clipboard();
            if (!cb)
                return;
            auto *mime = new QMimeData();
            mime->setText(qtext);
            if (is_private) {
                mime->setData(QStringLiteral("x-kde-passwordManagerHint"), QByteArrayLiteral("secret"));
                mime->setData(QStringLiteral("x-gnome-privacy-hint"), QByteArrayLiteral("1"));
                mime->setData(QStringLiteral("ExcludeClipboardContentFromMonitorProcessing"), QByteArrayLiteral("1"));
                mime->setData(QStringLiteral("Clipboard Viewer Ignore"), QByteArrayLiteral("1"));
            } else {
                if (!qhtml.isEmpty())
                    mime->setHtml(qhtml);
                if (!qrtf.isEmpty()) {
                    mime->setData(QStringLiteral("text/rtf"), qrtf);
                    mime->setData(QStringLiteral("application/rtf"), qrtf);
                }
            }
            cb->setMimeData(mime, QClipboard::Clipboard);
            ok = true;
        })) {
        return ok;
    }
#else
    (void)text;
    (void)html;
    (void)rtf;
    (void)is_private;
#endif
    return false;
}

bool linux_clipboard_write_image_png(rust::Slice<const uint8_t> png)
{
#ifdef __linux__
    if (png.empty())
        return false;
    const QByteArray bytes(reinterpret_cast<const char *>(png.data()), static_cast<qsizetype>(png.size()));
    bool ok = false;
    if (runOnQtMainThreadSync([&]() {
            QClipboard *cb = QGuiApplication::clipboard();
            if (!cb)
                return;
            auto *mime = new QMimeData();
            mime->setData(QStringLiteral("image/png"), bytes);
            QImage img;
            if (img.loadFromData(bytes, "PNG") && !img.isNull())
                mime->setImageData(img);
            cb->setMimeData(mime, QClipboard::Clipboard);
            ok = true;
        })) {
        return ok;
    }
#else
    (void)png;
#endif
    return false;
}

bool linux_clipboard_write_files(rust::Slice<const rust::String> paths)
{
#ifdef __linux__
    if (paths.empty())
        return false;
    QList<QUrl> urls;
    QByteArray gnomePayload("copy");
    for (const rust::String &p : paths) {
        const QString qp = QString::fromUtf8(p.data(), static_cast<qsizetype>(p.size()));
        if (!qp.isEmpty()) {
            const QUrl u = QUrl::fromLocalFile(qp);
            urls.append(u);
            gnomePayload.append('\n');
            gnomePayload.append(u.toEncoded());
        }
    }
    if (urls.isEmpty())
        return false;
    bool ok = false;
    if (runOnQtMainThreadSync([&]() {
            QClipboard *cb = QGuiApplication::clipboard();
            if (!cb)
                return;
            auto *mime = new QMimeData();
            mime->setUrls(urls);
            mime->setData(QStringLiteral("x-special/gnome-copied-files"), gnomePayload);
            mime->setData(QStringLiteral("x-special/mate-copied-files"), gnomePayload);
            cb->setMimeData(mime, QClipboard::Clipboard);
            ok = true;
        })) {
        return ok;
    }
#else
    (void)paths;
#endif
    return false;
}
