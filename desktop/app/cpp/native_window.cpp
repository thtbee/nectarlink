// SPDX-License-Identifier: GPL-3.0-or-later
#include "native_window.h"
#include "app_helpers.h"

#include <QtGui/QImage>
#include <QtGui/QPlatformSurfaceEvent>
#include <QtQml/QtQml>
#include <QtQuick/QSGRendererInterface>

#include <algorithm>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d11.h>
#include <dwmapi.h>
#include <dxgi1_3.h>
#include <windowsx.h>
#elif defined(__linux__)
#include <QtGui/QCursor>
#include <QtGui/QGuiApplication>
#include <QtGui/QMouseEvent>
#include <cstring>
#include <dlfcn.h>
#endif

namespace {

#ifdef _WIN32
// Values from the Windows 11 SDK, defined here so older SDK headers work.
constexpr DWORD kUseImmersiveDarkMode = 20;  // DWMWA_USE_IMMERSIVE_DARK_MODE
constexpr DWORD kCaptionColor = 35;          // DWMWA_CAPTION_COLOR
constexpr DWORD kSystemBackdropType = 38;    // DWMWA_SYSTEMBACKDROP_TYPE
constexpr int kBackdropNone = 1;             // DWMSBT_NONE
constexpr int kBackdropMica = 2;             // DWMSBT_MAINWINDOW
constexpr int kBackdropAcrylic = 3;          // DWMSBT_TRANSIENTWINDOW (Desktop Acrylic blur)
constexpr COLORREF kColorNone = 0xFFFFFFFE;  // DWMWA_COLOR_NONE

constexpr DWORD kFirstWindows11Build = 22000;

// The real Windows build: GetVersionEx lies without a manifest entry, and
// RtlGetVersion doesn't.
DWORD windowsBuild()
{
    using RtlGetVersionFn = LONG(WINAPI *)(OSVERSIONINFOW *);
    static const DWORD build = [] {
        OSVERSIONINFOW info { sizeof(info) };
        if (const HMODULE ntdll = GetModuleHandleW(L"ntdll.dll")) {
            const auto fn = reinterpret_cast<RtlGetVersionFn>(GetProcAddress(ntdll, "RtlGetVersion"));
            if (fn && fn(&info) == 0)
                return info.dwBuildNumber;
        }
        return DWORD(0);
    }();
    return build;
}

HWND hwndOf(const QWindow *window)
{
    return reinterpret_cast<HWND>(window->winId());
}

void trimD3DResources(QQuickWindow *window)
{
    if (!window)
        return;
    QSGRendererInterface *rif = window->rendererInterface();
    if (!rif)
        return;
    auto *device = static_cast<ID3D11Device *>(
        rif->getResource(window, QSGRendererInterface::DeviceResource));
    if (!device)
        return;
    IDXGIDevice3 *dxgi3 = nullptr;
    if (SUCCEEDED(device->QueryInterface(__uuidof(IDXGIDevice3), reinterpret_cast<void **>(&dxgi3))) && dxgi3) {
        dxgi3->Trim();
        dxgi3->Release();
    }
}
#else
void trimD3DResources(QQuickWindow *)
{
}
#endif

#ifdef __linux__
int ignoreX11Error(void *, void *)
{
    return 0;
}

void activateX11Window(WId winId)
{
    if (!winId || qEnvironmentVariableIsEmpty("DISPLAY"))
        return;
    // On Wayland, QWindow::winId() is not an X11 Window ID even when XWayland sets DISPLAY.
    if (QGuiApplication::platformName() != QLatin1String("xcb"))
        return;

    void *lib = dlopen("libX11.so.6", RTLD_LAZY | RTLD_LOCAL);
    if (!lib)
        return;

    using XErrorHandlerFn = int (*)(void *, void *);
    using XSetErrorHandlerFn = XErrorHandlerFn (*)(XErrorHandlerFn);
    using XOpenDisplayFn = void *(*)(const char *);
    using XCloseDisplayFn = int (*)(void *);
    using XDefaultRootWindowFn = unsigned long (*)(void *);
    using XInternAtomFn = unsigned long (*)(void *, const char *, int);
    using XSendEventFn = int (*)(void *, unsigned long, int, long, void *);
    using XMapRaisedFn = int (*)(void *, unsigned long);
    using XSetInputFocusFn = int (*)(void *, unsigned long, int, unsigned long);
    using XSyncFn = int (*)(void *, int);
    using XFlushFn = int (*)(void *);

    auto xSetErrorHandler = reinterpret_cast<XSetErrorHandlerFn>(dlsym(lib, "XSetErrorHandler"));
    auto xOpenDisplay = reinterpret_cast<XOpenDisplayFn>(dlsym(lib, "XOpenDisplay"));
    auto xCloseDisplay = reinterpret_cast<XCloseDisplayFn>(dlsym(lib, "XCloseDisplay"));
    auto xDefaultRootWindow = reinterpret_cast<XDefaultRootWindowFn>(dlsym(lib, "XDefaultRootWindow"));
    auto xInternAtom = reinterpret_cast<XInternAtomFn>(dlsym(lib, "XInternAtom"));
    auto xSendEvent = reinterpret_cast<XSendEventFn>(dlsym(lib, "XSendEvent"));
    auto xMapRaised = reinterpret_cast<XMapRaisedFn>(dlsym(lib, "XMapRaised"));
    auto xSetInputFocus = reinterpret_cast<XSetInputFocusFn>(dlsym(lib, "XSetInputFocus"));
    auto xSync = reinterpret_cast<XSyncFn>(dlsym(lib, "XSync"));
    auto xFlush = reinterpret_cast<XFlushFn>(dlsym(lib, "XFlush"));

    if (xOpenDisplay && xCloseDisplay && xDefaultRootWindow && xInternAtom && xSendEvent && xFlush) {
        if (void *dpy = xOpenDisplay(nullptr)) {
            XErrorHandlerFn prevHandler = xSetErrorHandler ? xSetErrorHandler(ignoreX11Error) : nullptr;
            const unsigned long root = xDefaultRootWindow(dpy);
            const unsigned long window = static_cast<unsigned long>(winId);
            const unsigned long netActiveWindow = xInternAtom(dpy, "_NET_ACTIVE_WINDOW", 0);

            if (xMapRaised)
                xMapRaised(dpy, window);

            struct XClientMessageEventCompat {
                int type;
                unsigned long serial;
                int send_event;
                void *display;
                unsigned long window;
                unsigned long message_type;
                int format;
                long l[5];
                long pad[12];
            } ev {};
            ev.type = 33; // ClientMessage
            ev.send_event = 1;
            ev.display = dpy;
            ev.window = window;
            ev.message_type = netActiveWindow;
            ev.format = 32;
            ev.l[0] = 2; // Source indication: 2 = pager / direct user action (bypasses WM focus-stealing prevention)
            ev.l[1] = 0; // CurrentTime
            ev.l[2] = 0;

            constexpr long kSubstructureRedirectMask = (1L << 20);
            constexpr long kSubstructureNotifyMask = (1L << 19);
            xSendEvent(dpy, root, 0, kSubstructureRedirectMask | kSubstructureNotifyMask, &ev);

            if (xSetInputFocus)
                xSetInputFocus(dpy, window, 1 /* RevertToPointerRoot */, 0 /* CurrentTime */);

            if (xSync)
                xSync(dpy, 0);
            else
                xFlush(dpy);
            if (xSetErrorHandler)
                xSetErrorHandler(prevHandler);
            xCloseDisplay(dpy);
        }
    }
    dlclose(lib);
}

class LinuxWindowEventFilter : public QObject
{
public:
    explicit LinuxWindowEventFilter(NativeWindow *window)
        : QObject(window)
        , window_(window)
    {
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (watched != window_)
            return QObject::eventFilter(watched, event);

        switch (event->type()) {
        case QEvent::MouseMove: {
            auto *me = static_cast<QMouseEvent *>(event);
            if (me->buttons() == Qt::NoButton) {
                pendingMove_ = false;
                manualMove_ = false;
                const Qt::Edges edges = hitResizeEdges(me->position());
                updateCursorForEdges(edges);
            } else if (me->buttons() & Qt::LeftButton) {
                if (pendingMove_) {
                    if ((me->position() - pressPos_).manhattanLength() >= 4) {
                        pendingMove_ = false;
                        if (!window_->startSystemMove()) {
                            manualMove_ = true;
                        }
                        return true;
                    }
                } else if (manualMove_) {
                    const QPointF delta = me->globalPosition() - globalPressPos_;
                    window_->setPosition(winStartPos_ + delta.toPoint());
                    return true;
                }
            }
            break;
        }
        case QEvent::MouseButtonPress: {
            auto *me = static_cast<QMouseEvent *>(event);
            if (me->button() == Qt::LeftButton) {
                pendingMove_ = false;
                manualMove_ = false;
                const QPointF pos = me->position();
                const Qt::Edges edges = hitResizeEdges(pos);
                if (edges != Qt::Edges()) {
                    window_->startSystemResize(edges);
                    return true;
                }
                const qreal titleBarHeight = window_->captionHeight();
                if (pos.y() < titleBarHeight
                    && !window_->isCaptionHoleAt(pos)
                    && !hitInteractiveControl(window_->contentItem(), pos)) {
                    pendingMove_ = true;
                    pressPos_ = pos;
                    globalPressPos_ = me->globalPosition();
                    winStartPos_ = window_->position();
                }
            }
            break;
        }
        case QEvent::MouseButtonRelease: {
            auto *me = static_cast<QMouseEvent *>(event);
            if (me->button() == Qt::LeftButton) {
                pendingMove_ = false;
                if (manualMove_) {
                    manualMove_ = false;
                    return true;
                }
            }
            break;
        }
        case QEvent::MouseButtonDblClick: {
            auto *me = static_cast<QMouseEvent *>(event);
            if (me->button() == Qt::LeftButton) {
                pendingMove_ = false;
                manualMove_ = false;
                const QPointF pos = me->position();
                const Qt::Edges edges = hitResizeEdges(pos);
                const qreal titleBarHeight = window_->captionHeight();
                if (edges == Qt::Edges() && pos.y() < titleBarHeight
                    && !window_->isCaptionHoleAt(pos)
                    && !hitInteractiveControl(window_->contentItem(), pos)) {
                    window_->toggleMaximized();
                    return true;
                }
            }
            break;
        }
        case QEvent::Leave:
            if (borderCursorActive_) {
                window_->unsetCursor();
                borderCursorActive_ = false;
            }
            break;
        default:
            break;
        }
        return QObject::eventFilter(watched, event);
    }

private:
    Qt::Edges hitResizeEdges(const QPointF &pos) const
    {
        if (!window_ || window_->visibility() == QWindow::Maximized
            || window_->minimumSize() == window_->maximumSize()) {
            return {};
        }
        if (pos.y() < window_->captionHeight()
            && (window_->isCaptionHoleAt(pos) || hitInteractiveControl(window_->contentItem(), pos))) {
            return {};
        }
        const qreal w = window_->width();
        const qreal h = window_->height();
        const qreal b = resizeBorderWidth_;
        Qt::Edges edges;
        if (pos.x() < b)
            edges |= Qt::LeftEdge;
        else if (pos.x() >= w - b)
            edges |= Qt::RightEdge;
        if (pos.y() < b)
            edges |= Qt::TopEdge;
        else if (pos.y() >= h - b)
            edges |= Qt::BottomEdge;
        return edges;
    }

    void updateCursorForEdges(Qt::Edges edges)
    {
        if (edges == Qt::Edges()) {
            if (borderCursorActive_) {
                window_->unsetCursor();
                borderCursorActive_ = false;
            }
            return;
        }
        Qt::CursorShape shape = Qt::ArrowCursor;
        if ((edges == (Qt::TopEdge | Qt::LeftEdge)) || (edges == (Qt::BottomEdge | Qt::RightEdge)))
            shape = Qt::SizeFDiagCursor;
        else if ((edges == (Qt::TopEdge | Qt::RightEdge)) || (edges == (Qt::BottomEdge | Qt::LeftEdge)))
            shape = Qt::SizeBDiagCursor;
        else if (edges & (Qt::LeftEdge | Qt::RightEdge))
            shape = Qt::SizeHorCursor;
        else if (edges & (Qt::TopEdge | Qt::BottomEdge))
            shape = Qt::SizeVerCursor;
        window_->setCursor(QCursor(shape));
        borderCursorActive_ = true;
    }

    bool hitInteractiveControl(QQuickItem *item, const QPointF &scenePos) const
    {
        if (!item || !item->isVisible() || !item->isEnabled() || item->opacity() <= 0.0)
            return false;
        const QPointF localPos = item->mapFromScene(scenePos);
        if (!item->contains(localPos))
            return false;
        if (item == window_->maximizeButton() || item->property("_nl_caption_hole").toBool())
            return true;
        const auto children = item->childItems();
        for (int i = children.size() - 1; i >= 0; --i) {
            if (hitInteractiveControl(children[i], scenePos))
                return true;
        }
        if (item != window_->contentItem()
            && item->acceptedMouseButtons() != Qt::NoButton) {
            return true;
        }
        return false;
    }

    NativeWindow *window_ = nullptr;
    int resizeBorderWidth_ = 6;
    bool borderCursorActive_ = false;
    bool pendingMove_ = false;
    bool manualMove_ = false;
    QPointF pressPos_;
    QPointF globalPressPos_;
    QPoint winStartPos_;
};
#endif

} // namespace

NativeWindow::NativeWindow(QWindow *parent)
    : QQuickWindow(parent)
{
    // Qt removes the system title bar and lets content fill the window.
#ifdef _WIN32
    if (systemBackdropSupported()) {
        setFlags(flags() | Qt::ExpandedClientAreaHint);
    } else {
        // Windows 10 keeps drawing its own caption (icon, title, buttons) over an
        // expanded client area. A frameless window has no caption at all; the
        // app bar's buttons and WM_NCHITTEST still give drag, resize and maximize.
        setFlags(flags() | Qt::FramelessWindowHint);
    }
    setColor(Qt::transparent);
#elif defined(__linux__)
    setFlags(flags() | Qt::FramelessWindowHint);
    setColor(QColor(0x14, 0x13, 0x11));
#else
    setColor(Qt::transparent);
#endif
    // Drop the scene graph and D3D/RHI resources when the window is hidden or
    // closed to the tray instead of keeping GPU buffers in private commit.
    setPersistentSceneGraph(false);
    setPersistentGraphics(false);
    connect(this, &QWindow::visibilityChanged, this, [this](QWindow::Visibility) {
        emit windowVisibilityChanged();
    });
#ifdef __linux__
    setProperty("_nl_linux_filter", true);
    installEventFilter(new LinuxWindowEventFilter(this));
#endif
}

bool NativeWindow::systemBackdropSupported()
{
#ifdef _WIN32
    return windowsBuild() >= kFirstWindows11Build;
#else
    return false;
#endif
}

void NativeWindow::setBackdrop(bool on)
{
    if (m_backdrop == on)
        return;
    m_backdrop = on;
    applyFrame();
    emit backdropChanged();
}

void NativeWindow::setAcrylic(bool on)
{
    if (m_acrylic == on)
        return;
    m_acrylic = on;
    applyFrame();
    emit acrylicChanged();
}

void NativeWindow::setDarkFrame(bool dark)
{
    if (m_darkFrame == dark)
        return;
    m_darkFrame = dark;
    applyFrame();
    emit darkFrameChanged();
}

void NativeWindow::setCaptionHeight(qreal height)
{
    if (qFuzzyCompare(m_captionHeight, height))
        return;
    m_captionHeight = height;
    emit captionHeightChanged();
}

void NativeWindow::setMaximizeButton(QQuickItem *item)
{
    if (m_maximizeButton == item)
        return;
    m_maximizeButton = item;
    emit maximizeButtonChanged();
}

void NativeWindow::setMaximizeState(bool hovered, bool pressed)
{
    if (m_maximizeHovered == hovered && m_maximizePressed == pressed)
        return;
    m_maximizeHovered = hovered;
    m_maximizePressed = pressed;
    emit maximizeStateChanged();
}

void NativeWindow::addCaptionHole(QQuickItem *item)
{
    if (item)
        item->setProperty("_nl_caption_hole", true);
    if (item && std::none_of(m_holes.begin(), m_holes.end(), [item](const auto &h) { return h == item; }))
        m_holes.emplace_back(item);
}

void NativeWindow::removeCaptionHole(QQuickItem *item)
{
    if (item)
        item->setProperty("_nl_caption_hole", false);
    m_holes.erase(std::remove_if(m_holes.begin(), m_holes.end(),
                                 [item](const auto &h) { return h.isNull() || h == item; }),
                  m_holes.end());
}

void NativeWindow::toggleMaximized()
{
    if (visibility() == QWindow::Maximized)
        showNormal();
    else
        showMaximized();
}

void NativeWindow::bringToFront()
{
    if (visibility() == QWindow::Minimized) {
        setWindowStates((windowStates() & ~Qt::WindowMinimized) | Qt::WindowActive);
        show();
    } else if (!isVisible()) {
        showNormal();
    }
    raise();
    requestActivate();
#ifdef _WIN32
    if (!handle())
        return;
    const HWND hwnd = hwndOf(this);
    if (SetForegroundWindow(hwnd) || GetForegroundWindow() == hwnd)
        return;
    // Windows only lets the process the user is working in change the
    // foreground window, so a copy opened from the background (after an
    // update, or by another instance) would stay behind. Sharing the
    // foreground thread's input state for a moment lets it come forward.
    const HWND foreground = GetForegroundWindow();
    const DWORD foregroundThread = foreground ? GetWindowThreadProcessId(foreground, nullptr) : 0;
    const DWORD thisThread = GetCurrentThreadId();
    const bool attached = foregroundThread != 0 && foregroundThread != thisThread
        && AttachThreadInput(thisThread, foregroundThread, TRUE);
    BringWindowToTop(hwnd);
    SetForegroundWindow(hwnd);
    if (attached)
        AttachThreadInput(thisThread, foregroundThread, FALSE);
#elif defined(__linux__)
    if (handle())
        activateX11Window(winId());
#endif
}

void NativeWindow::flash()
{
    if (!handle())
        return;
#ifdef _WIN32
    FLASHWINFO info { sizeof(FLASHWINFO), hwndOf(this), FLASHW_TRAY | FLASHW_TIMERNOFG, 0, 0 };
    FlashWindowEx(&info);
#else
    alert(0);
#endif
}

void NativeWindow::releaseAndTeardown()
{
    if (QQmlEngine *engine = qmlEngine(this))
        register_qml_engine(engine);
    m_holes.clear();
    m_maximizeButton = nullptr;
    trimD3DResources(this);
    releaseResources();
    hide();
    trimD3DResources(this);
    destroy();
}

bool NativeWindow::grabToFile(const QString &path)
{
    if (path.isEmpty())
        return false;
    const QImage image = grabWindow();
    if (image.isNull())
        return false;
    return image.save(path);
}

bool NativeWindow::event(QEvent *event)
{
    if (QQmlEngine *engine = qmlEngine(this))
        register_qml_engine(engine);
    if (event->type() == QEvent::Close) {
        trimD3DResources(this);
        event->accept();
        emit closeRequested();
        return true;
    }
    if (event->type() == QEvent::Hide) {
        trimD3DResources(this);
    }
    if (event->type() == QEvent::PlatformSurface
        && static_cast<QPlatformSurfaceEvent *>(event)->surfaceEventType()
            == QPlatformSurfaceEvent::SurfaceCreated) {
        const bool handled = QQuickWindow::event(event);
        applyFrame();
        return handled;
    }
    return QQuickWindow::event(event);
}

void NativeWindow::applyFrame()
{
#ifdef _WIN32
    if (!handle())
        return;
    const HWND hwnd = hwndOf(this);
    // No frame extension: Mica/Acrylic still shows behind transparent pixels (the
    // swap chain is composited with DirectComposition), and DWM doesn't draw
    // its own caption buttons over the app's.
    const MARGINS margins { 0, 0, 0, 0 };
    DwmExtendFrameIntoClientArea(hwnd, &margins);
    if (!systemBackdropSupported()) {
        // Windows 10, frameless: give back the frame styles so Snap, Win+arrow,
        // min/max animations and the shadow work. WM_NCCALCSIZE below keeps the
        // whole window as client area, so no caption is drawn.
        const LONG_PTR style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        const LONG_PTR want = (style & ~LONG_PTR(WS_POPUP))
            | WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX;
        if (style != want) {
            SetWindowLongPtrW(hwnd, GWL_STYLE, want);
            SetWindowPos(hwnd, nullptr, 0, 0, 0, 0,
                         SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        const MARGINS shadow { 0, 0, 1, 0 };  // a 1px frame keeps the DWM shadow
        DwmExtendFrameIntoClientArea(hwnd, &shadow);
    }
    const BOOL dark = m_darkFrame;
    DwmSetWindowAttribute(hwnd, kUseImmersiveDarkMode, &dark, sizeof(dark));
    // Windows 10 has no system backdrop or caption color: skip both, and the
    // QML side keeps the window opaque there (see systemBackdropSupported).
    if (systemBackdropSupported()) {
        const int backdrop = m_backdrop ? (m_acrylic ? kBackdropAcrylic : kBackdropMica) : kBackdropNone;
        DwmSetWindowAttribute(hwnd, kSystemBackdropType, &backdrop, sizeof(backdrop));
        const COLORREF none = kColorNone;
        DwmSetWindowAttribute(hwnd, kCaptionColor, &none, sizeof(none));
    }
#elif defined(__linux__)
    setColor(m_darkFrame ? QColor(0x14, 0x13, 0x11) : QColor(0xF8, 0xF6, 0xF2));
    if (!property("_nl_linux_filter").toBool()) {
        setProperty("_nl_linux_filter", true);
        installEventFilter(new LinuxWindowEventFilter(this));
    }
#else
    setColor(m_darkFrame ? QColor(0x14, 0x13, 0x11) : QColor(0xF8, 0xF6, 0xF2));
#endif
}

bool NativeWindow::itemContains(const QQuickItem *item, const QPointF &scenePoint) const
{
    if (!item || !item->isVisible())
        return false;
    return item->mapRectToScene(QRectF(0, 0, item->width(), item->height())).contains(scenePoint);
}

bool NativeWindow::isCaptionHoleAt(const QPointF &scenePoint) const
{
    if (itemContains(m_maximizeButton, scenePoint))
        return true;
    for (const auto &hole : m_holes) {
        if (itemContains(hole, scenePoint))
            return true;
    }
    return false;
}

long NativeWindow::hitTest(int screenX, int screenY) const
{
#ifdef _WIN32
    const HWND hwnd = hwndOf(this);
    POINT pt { screenX, screenY };
    ScreenToClient(hwnd, &pt);

    // The system title bar is gone, and with it the top resize border.
    // A window of fixed size has none.
    if (!IsZoomed(hwnd) && minimumSize() != maximumSize()) {
        const UINT dpi = GetDpiForWindow(hwnd);
        const int border = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        if (pt.y < border) {
            RECT client {};
            GetClientRect(hwnd, &client);
            if (pt.x < border * 2)
                return HTTOPLEFT;
            if (pt.x >= client.right - border * 2)
                return HTTOPRIGHT;
            return HTTOP;
        }
    }

    const qreal dpr = devicePixelRatio();
    const QPointF point(pt.x / dpr, pt.y / dpr);
    if (itemContains(m_maximizeButton, point))
        return HTMAXBUTTON;
    if (point.y() >= m_captionHeight)
        return HTCLIENT;
    for (const auto &hole : m_holes) {
        if (itemContains(hole, point))
            return HTCLIENT;
    }
    return HTCAPTION;
#else
    Q_UNUSED(screenX);
    Q_UNUSED(screenY);
    return 0;
#endif
}

bool NativeWindow::nativeEvent(const QByteArray &eventType, void *message, qintptr *result)
{
#ifdef _WIN32
    if (eventType != "windows_generic_MSG")
        return QQuickWindow::nativeEvent(eventType, message, result);
    const MSG *msg = static_cast<const MSG *>(message);

    switch (msg->message) {
    case WM_NCCALCSIZE:
        // Windows 10 (frame styles on a frameless window): all client area.
        if (msg->wParam == TRUE && !systemBackdropSupported()) {
            if (IsZoomed(msg->hwnd)) {
                // Maximized windows hang their frame off-screen: pull it back in.
                auto *params = reinterpret_cast<NCCALCSIZE_PARAMS *>(msg->lParam);
                const UINT dpi = GetDpiForWindow(msg->hwnd);
                const int fx = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                const int fy = GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                params->rgrc[0].left += fx;
                params->rgrc[0].right -= fx;
                params->rgrc[0].top += fy;
                params->rgrc[0].bottom -= fy;
            }
            *result = 0;
            return true;
        }
        break;
    case WM_NCHITTEST: {
        const long hit = hitTest(GET_X_LPARAM(msg->lParam), GET_Y_LPARAM(msg->lParam));
        if (hit != HTCLIENT) {
            *result = hit;
            return true;
        }
        break;
    }
    // The maximize button reports HTMAXBUTTON, so its input arrives as
    // non-client messages. Handle them here and keep DefWindowProc from
    // drawing and tracking a classic button.
    case WM_NCMOUSEMOVE: {
        QEvent leaveEvent(QEvent::Leave);
        QCoreApplication::sendEvent(this, &leaveEvent);
        if (msg->wParam == HTMAXBUTTON) {
            if (!m_maximizeHovered) {
                TRACKMOUSEEVENT track { sizeof(track), TME_LEAVE | TME_NONCLIENT, msg->hwnd, 0 };
                TrackMouseEvent(&track);
            }
            setMaximizeState(true, m_maximizePressed);
            *result = 0;
            return true;
        }
        setMaximizeState(false, false);
        break;
    }
    case WM_NCMOUSELEAVE:
    case WM_MOUSEMOVE:
        setMaximizeState(false, false);
        break;
    case WM_NCLBUTTONDOWN:
    case WM_NCLBUTTONDBLCLK:
        if (msg->wParam == HTMAXBUTTON) {
            setMaximizeState(true, true);
            *result = 0;
            return true;
        }
        break;
    case WM_NCLBUTTONUP:
        if (msg->wParam == HTMAXBUTTON) {
            const bool wasPressed = m_maximizePressed;
            setMaximizeState(true, false);
            if (wasPressed)
                toggleMaximized();
            *result = 0;
            return true;
        }
        setMaximizeState(false, false);
        break;
    default:
        break;
    }
#endif
    return QQuickWindow::nativeEvent(eventType, message, result);
}
