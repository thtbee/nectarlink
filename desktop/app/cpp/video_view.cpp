// SPDX-License-Identifier: GPL-3.0-or-later
#include "video_view.h"

#include <QtCore/QHash>
#include <QtCore/QMetaMethod>
#include <QtCore/QUrl>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QInputMethod>
#include <QtGui/QInputMethodEvent>
#include <QtGui/QPainter>
#include <QtGui/QPixmap>
#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/QSGTexture>

#ifdef _WIN32
#include <windows.h>
#include <propkey.h>
#include <propsys.h>
#include <shobjidl.h>
#endif

#include <algorithm>
#include <mutex>
#include <vector>

namespace {

struct Latest
{
    QImage image;
    quint64 serial = 0;
};

// Pictures by stream, and the views to tell. Views register and leave
// under the lock, so a picture never wakes a view that's being destroyed.
// Never destroyed: a decoder thread can still be finishing while the app exits, after
// destructors of globals have run (a crash on quitting with a mirror open).
std::mutex &lock = *new std::mutex;
QHash<QString, Latest> &latest = *new QHash<QString, Latest>;
std::vector<VideoView *> &views = *new std::vector<VideoView *>;
quint64 nextSerial = 1;

void wakeViews(const QString &stream)
{
    for (VideoView *view : views) {
        if (view->stream() == stream)
            view->queueFrame();
    }
}

// A small, heavily blurred, dimmed copy of a picture. Made once per resize
// on the CPU (about a millisecond), so it's on screen with the very next
// frame and each step of the drag only scales one small texture.
QImage frost(const QImage &picture)
{
    QImage img = picture.scaled(96, 96, Qt::KeepAspectRatio, Qt::SmoothTransformation)
                     .convertToFormat(QImage::Format_RGB32);
    QImage tmp(img.size(), img.format());
    const int w = img.width(), h = img.height();
    const int radius = 8;
    // Three box blurs each way come close to a Gaussian one.
    for (int pass = 0; pass < 6; ++pass) {
        const bool across = pass % 2 == 0;
        const int length = across ? w : h;
        for (int y = 0; y < h; ++y) {
            const QRgb *from = reinterpret_cast<const QRgb *>(img.constScanLine(y));
            QRgb *to = reinterpret_cast<QRgb *>(tmp.scanLine(y));
            for (int x = 0; x < w; ++x) {
                int r = 0, g = 0, b = 0;
                for (int d = -radius; d <= radius; ++d) {
                    const int at = qBound(0, (across ? x : y) + d, length - 1);
                    const QRgb p = across ? from[at] : reinterpret_cast<const QRgb *>(img.constScanLine(at))[x];
                    r += qRed(p);
                    g += qGreen(p);
                    b += qBlue(p);
                }
                const int n = 2 * radius + 1;
                to[x] = qRgb(r / n, g / n, b / n);
            }
        }
        img.swap(tmp);
    }
    QPainter(&img).fillRect(img.rect(), QColor(0, 0, 0, 70));
    return img;
}

// Whether two pictures look different: more than a few of a grid of points
// differ clearly. A blinking caret or the encoder's noise doesn't count; an
// app that laid itself out again does.
bool looksDifferent(const QImage &a, const QImage &b)
{
    if (a.size() != b.size() || a.isNull())
        return a.size() != b.size();
    const int grid = 48;
    int differing = 0;
    for (int gy = 0; gy < grid; ++gy) {
        const int y = (2 * gy + 1) * a.height() / (2 * grid);
        const QRgb *rowA = reinterpret_cast<const QRgb *>(a.constScanLine(y));
        const QRgb *rowB = reinterpret_cast<const QRgb *>(b.constScanLine(y));
        for (int gx = 0; gx < grid; ++gx) {
            const int x = (2 * gx + 1) * a.width() / (2 * grid);
            if (qAbs(qGray(rowA[x]) - qGray(rowB[x])) > 16)
                ++differing;
        }
    }
    return differing >= 8;
}

// Owns its texture (replaced with each picture).
class VideoNode : public QSGSimpleTextureNode
{
public:
    VideoNode() { setOwnsTexture(false); }
    ~VideoNode() override { delete texture(); }

    void replaceTexture(QSGTexture *next)
    {
        QSGTexture *previous = texture();
        setTexture(next);
        delete previous;
    }
};

} // namespace

VideoView::VideoView(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    QGuiApplication::instance()->installNativeEventFilter(this);
    std::lock_guard guard(lock);
    views.push_back(this);
}

VideoView::~VideoView()
{
    QGuiApplication::instance()->removeNativeEventFilter(this);
    std::lock_guard guard(lock);
    views.erase(std::remove(views.begin(), views.end(), this), views.end());
}

void VideoView::setStream(const QString &stream)
{
    if (stream == m_stream)
        return;
    {
        std::lock_guard guard(lock);
        m_stream = stream;
    }
    m_shown = 0;
    emit streamChanged();
    frameArrived();
}

void VideoView::setWindowIcon(const QString &icon)
{
    if (icon == m_windowIcon)
        return;
    m_windowIcon = icon;
    emit windowIconChanged();
    applyWindowChrome();
}

void VideoView::setWindowAppId(const QString &appId)
{
    if (appId == m_windowAppId)
        return;
    m_windowAppId = appId;
    emit windowAppIdChanged();
    applyWindowChrome();
}

void VideoView::setAcceptsText(bool on)
{
    if (on == m_acceptsText)
        return;
    m_acceptsText = on;
    setFlag(ItemAcceptsInputMethod, on);
    if (hasActiveFocus())
        QGuiApplication::inputMethod()->update(Qt::ImEnabled);
    emit acceptsTextChanged();
}

void VideoView::setFrosted(bool on)
{
    if (on == m_frosted)
        return;
    m_frosted = on;
    m_shown = 0;
    emit frostedChanged();
    update();
}

void VideoView::setFrozen(bool on)
{
    if (on == m_frozen)
        return;
    m_frozen = on;
    // Turned on: the newest picture is taken once more, then kept.
    m_holding = false;
    {
        std::lock_guard guard(lock);
        m_kept = on ? latest.value(m_stream).image : QImage();
    }
    if (m_changedSinceFrozen) {
        m_changedSinceFrozen = false;
        emit changedSinceFrozenChanged();
    }
    emit frozenChanged();
    update();
}

void VideoView::setKeepShape(bool on)
{
    if (on == m_keepShape)
        return;
    m_keepShape = on;
    emit keepShapeChanged();
}

bool VideoView::nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result)
{
#ifdef _WIN32
    if (eventType != "windows_generic_MSG")
        return false;
    const MSG *msg = static_cast<const MSG *>(message);
    // An edge is grabbed (the press on it, or the system's Size command,
    // before the mouse moves), dragged, or let go.
    const bool grabbed = (msg->message == WM_NCLBUTTONDOWN && msg->wParam >= HTLEFT && msg->wParam <= HTBOTTOMRIGHT)
        || (msg->message == WM_SYSCOMMAND && (msg->wParam & 0xFFF0) == SC_SIZE);
    if (!grabbed && msg->message != WM_SIZING && msg->message != WM_EXITSIZEMOVE)
        return false;
    QQuickWindow *win = window();
    // Only a window that exists: winId() would create one.
    if (!win || !win->handle() || msg->hwnd != reinterpret_cast<HWND>(win->winId()))
        return false;
    if (m_sizing != (msg->message != WM_EXITSIZEMOVE)) {
        m_sizing = !m_sizing;
        emit sizingChanged();
        // Only where a window listens (a mirror window, not a preview on a page).
        if (m_sizing && isSignalConnected(QMetaMethod::fromSignal(&VideoView::sizingChanged))) {
            // What the window changed for it is drawn now: once the press is
            // handed on, Windows holds the thread until the mouse moves.
            win->update();
            QEvent draw(QEvent::UpdateRequest);
            QCoreApplication::sendEvent(win, &draw);
        }
    }
    if (msg->message != WM_SIZING || !m_keepShape || m_frameSize.isEmpty())
        return false;
    // The rect being dragged is the whole window; the picture fills the
    // client area, so the frame's thickness stays out of the shape.
    RECT outer{}, client{};
    GetWindowRect(msg->hwnd, &outer);
    GetClientRect(msg->hwnd, &client);
    const int frameW = (outer.right - outer.left) - client.right;
    const int frameH = (outer.bottom - outer.top) - client.bottom;
    RECT *r = reinterpret_cast<RECT *>(msg->lParam);
    const double ratio = double(m_frameSize.width()) / m_frameSize.height();
    const int w = qMax(1, int(r->right - r->left) - frameW);
    const int h = qMax(1, int(r->bottom - r->top) - frameH);
    switch (msg->wParam) {
    case WMSZ_TOP:
    case WMSZ_BOTTOM:
        // Height leads; the width follows to the right.
        r->right = r->left + qRound(h * ratio) + frameW;
        break;
    case WMSZ_TOPLEFT:
    case WMSZ_TOPRIGHT:
        // Width leads; the top edge follows.
        r->top = r->bottom - (qRound(w / ratio) + frameH);
        break;
    default:
        // Left, right and the bottom corners: width leads, the bottom follows.
        r->bottom = r->top + qRound(w / ratio) + frameH;
        break;
    }
    *result = TRUE;
    return true;
#else
    Q_UNUSED(eventType)
    Q_UNUSED(message)
    Q_UNUSED(result)
    return false;
#endif
}

void VideoView::inputMethodEvent(QInputMethodEvent *event)
{
    // Text being composed stays in the input method's own window; only
    // finished text goes to the phone.
    if (m_acceptsText && !event->commitString().isEmpty())
        emit textCommitted(event->commitString());
    event->accept();
}

QVariant VideoView::inputMethodQuery(Qt::InputMethodQuery query) const
{
    switch (query) {
    case Qt::ImEnabled:
        return m_acceptsText;
    case Qt::ImCursorRectangle: {
        // There is no caret to point at: the emoji panel and candidate
        // lists open at the middle of the phone's picture.
        const QPointF middle = pictureRect().center();
        return QRectF(middle.x(), middle.y(), 1, 1);
    }
    case Qt::ImSurroundingText:
        return QString();
    case Qt::ImCursorPosition:
    case Qt::ImAnchorPosition:
        return 0;
    default:
        return QQuickItem::inputMethodQuery(query);
    }
}

void VideoView::itemChange(ItemChange change, const ItemChangeData &value)
{
    QQuickItem::itemChange(change, value);
    if (change == ItemSceneChange && value.window) {
        connect(value.window, &QWindow::visibleChanged, this, [this](bool) { applyWindowChrome(); });
        applyWindowChrome();
    }
}

void VideoView::applyWindowChrome()
{
    QQuickWindow *win = window();
    if (!win)
        return;
#ifdef _WIN32
    if (!m_windowAppId.isEmpty()) {
        if (HWND hwnd = reinterpret_cast<HWND>(win->winId())) {
            IPropertyStore *store = nullptr;
            if (SUCCEEDED(SHGetPropertyStoreForWindow(hwnd, IID_PPV_ARGS(&store))) && store) {
                PROPVARIANT pv{};
                pv.vt = VT_LPWSTR;
                pv.pwszVal = const_cast<LPWSTR>(reinterpret_cast<LPCWSTR>(m_windowAppId.utf16()));
                store->SetValue(PKEY_AppUserModel_ID, pv);
                store->Commit();
                store->Release();
            }
        }
    }
#endif
    if (!m_windowIcon.isEmpty()) {
        const QUrl url(m_windowIcon);
        const QString path = url.isLocalFile() ? url.toLocalFile() : m_windowIcon;
        const QImage img(path);
        if (!img.isNull()) {
            QIcon icon;
            for (int sz : {16, 20, 24, 32, 40, 48, 64, 256}) {
                icon.addPixmap(QPixmap::fromImage(img.scaled(sz, sz, Qt::KeepAspectRatio, Qt::SmoothTransformation)));
            }
            win->setIcon(icon);
        }
    }
}

QRectF VideoView::pictureRect() const
{
    if (m_frameSize.isEmpty())
        return {};
    const qreal shapes = (qreal(m_frameSize.width()) / m_frameSize.height()) / (width() / qMax<qreal>(1, height()));
    if (m_fillsNearShape && qAbs(shapes - 1) < 0.02)
        return QRectF(0, 0, width(), height());
    const QSizeF fitted = QSizeF(m_frameSize).scaled(size(), Qt::KeepAspectRatio);
    return QRectF((width() - fitted.width()) / 2, (height() - fitted.height()) / 2, fitted.width(), fitted.height());
}

void VideoView::queueFrame()
{
    // The picture cache already keeps only the newest image. Coalesce UI
    // notifications too, so a busy UI does not replay old frame callbacks.
    if (!m_framePending.exchange(true))
        QMetaObject::invokeMethod(this, "frameArrived", Qt::QueuedConnection);
}

void VideoView::frameArrived()
{
    m_framePending.store(false);
    QImage newest;
    {
        std::lock_guard guard(lock);
        newest = latest.value(m_stream).image;
    }
    const QSize size = newest.size();
    if (m_frozen && !m_changedSinceFrozen && !newest.isNull() && looksDifferent(m_kept, newest)) {
        m_changedSinceFrozen = true;
        emit changedSinceFrozenChanged();
    }
    if (size != m_frameSize) {
        m_frameSize = size;
        emit frameSizeChanged();
    }
    ++m_frames;
    emit framesChanged();
    update();
}

QSGNode *VideoView::updatePaintNode(QSGNode *old, UpdatePaintNodeData *)
{
    Latest picture;
    {
        std::lock_guard guard(lock);
        picture = latest.value(m_stream);
    }
    if (picture.image.isNull() || !window()) {
        delete old;
        m_shown = 0;
        return nullptr;
    }
    auto *node = static_cast<VideoNode *>(old);
    if (!node)
        node = new VideoNode;
    if (!node->texture() || (picture.serial != m_shown && !m_holding)) {
        QSGTexture *texture = window()->createTextureFromImage(m_frosted ? frost(picture.image) : picture.image);
        texture->setFiltering(QSGTexture::Linear);
        node->replaceTexture(texture);
        m_shown = picture.serial;
        m_holding = m_frozen;
    }
    node->setFiltering(QSGTexture::Linear);
    if (m_frosted) {
        // The shape of the picture on screen (a kept one may be older than
        // `frameSize`), covering the item.
        const QSizeF cover = QSizeF(node->texture()->textureSize()).scaled(size(), Qt::KeepAspectRatioByExpanding);
        node->setRect(QRectF((width() - cover.width()) / 2, (height() - cover.height()) / 2, cover.width(),
                             cover.height()));
    } else {
        node->setRect(pictureRect());
    }
    return node;
}

void video_frame(rust::Str stream, uint32_t width, uint32_t height, rust::Slice<const uint8_t> bgrx)
{
    if (bgrx.size() < size_t(width) * height * 4)
        return;
    // One copy, into an image Qt owns, performed outside the shared mutex.
    QImage image(reinterpret_cast<const uchar *>(bgrx.data()), int(width), int(height), int(width) * 4,
                 QImage::Format_RGB32);
    QImage copy = image.copy();
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.insert(key, Latest{std::move(copy), nextSerial++});
    wakeViews(key);
}

void video_clear(rust::Str stream)
{
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.remove(key);
    wakeViews(key);
}

void clear_idle_video_frames()
{
    std::lock_guard guard(lock);
    if (views.empty()) {
        latest.clear();
        latest.squeeze();
    }
}

