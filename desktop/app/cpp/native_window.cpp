// SPDX-License-Identifier: GPL-3.0-or-later
#include "native_window.h"

#include <QtGui/QPlatformSurfaceEvent>

#include <algorithm>

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <dwmapi.h>
#include <windowsx.h>

namespace {

// Values from the Windows 11 SDK, defined here so older SDK headers work.
constexpr DWORD kUseImmersiveDarkMode = 20;  // DWMWA_USE_IMMERSIVE_DARK_MODE
constexpr DWORD kCaptionColor = 35;          // DWMWA_CAPTION_COLOR
constexpr DWORD kSystemBackdropType = 38;    // DWMWA_SYSTEMBACKDROP_TYPE
constexpr int kBackdropNone = 1;             // DWMSBT_NONE
constexpr int kBackdropMica = 2;             // DWMSBT_MAINWINDOW
constexpr COLORREF kColorNone = 0xFFFFFFFE;  // DWMWA_COLOR_NONE

HWND hwndOf(const QWindow *window)
{
    return reinterpret_cast<HWND>(window->winId());
}

} // namespace

NativeWindow::NativeWindow(QWindow *parent)
    : QQuickWindow(parent)
{
    // Qt removes the system title bar and lets content fill the window.
    setFlags(flags() | Qt::ExpandedClientAreaHint);
    setColor(Qt::transparent);
}

void NativeWindow::setBackdrop(bool on)
{
    if (m_backdrop == on)
        return;
    m_backdrop = on;
    applyFrame();
    emit backdropChanged();
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
    if (item && std::none_of(m_holes.begin(), m_holes.end(), [item](const auto &h) { return h == item; }))
        m_holes.emplace_back(item);
}

void NativeWindow::removeCaptionHole(QQuickItem *item)
{
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
    if (visibility() == QWindow::Minimized || !isVisible())
        showNormal();
    raise();
    requestActivate();
    if (handle())
        SetForegroundWindow(hwndOf(this));
}

void NativeWindow::flash()
{
    if (!handle())
        return;
    FLASHWINFO info { sizeof(FLASHWINFO), hwndOf(this), FLASHW_TRAY | FLASHW_TIMERNOFG, 0, 0 };
    FlashWindowEx(&info);
}

bool NativeWindow::event(QEvent *event)
{
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
    if (!handle())
        return;
    const HWND hwnd = hwndOf(this);
    // No frame extension: Mica still shows behind transparent pixels (the
    // swap chain is composited with DirectComposition), and DWM doesn't draw
    // its own caption buttons over the app's.
    const MARGINS margins { 0, 0, 0, 0 };
    DwmExtendFrameIntoClientArea(hwnd, &margins);
    const BOOL dark = m_darkFrame;
    DwmSetWindowAttribute(hwnd, kUseImmersiveDarkMode, &dark, sizeof(dark));
    const int backdrop = m_backdrop ? kBackdropMica : kBackdropNone;
    DwmSetWindowAttribute(hwnd, kSystemBackdropType, &backdrop, sizeof(backdrop));
    const COLORREF none = kColorNone;
    DwmSetWindowAttribute(hwnd, kCaptionColor, &none, sizeof(none));
}

bool NativeWindow::itemContains(const QQuickItem *item, const QPointF &scenePoint) const
{
    if (!item || !item->isVisible())
        return false;
    return item->mapRectToScene(QRectF(0, 0, item->width(), item->height())).contains(scenePoint);
}

long NativeWindow::hitTest(int screenX, int screenY) const
{
    const HWND hwnd = hwndOf(this);
    POINT pt { screenX, screenY };
    ScreenToClient(hwnd, &pt);

    // The system title bar is gone, and with it the top resize border.
    if (!IsZoomed(hwnd)) {
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
}

bool NativeWindow::nativeEvent(const QByteArray &eventType, void *message, qintptr *result)
{
    if (eventType != "windows_generic_MSG")
        return QQuickWindow::nativeEvent(eventType, message, result);
    const MSG *msg = static_cast<const MSG *>(message);

    switch (msg->message) {
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
    case WM_NCMOUSEMOVE:
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
    return QQuickWindow::nativeEvent(eventType, message, result);
}
