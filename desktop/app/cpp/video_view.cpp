// SPDX-License-Identifier: GPL-3.0-or-later
#include "video_view.h"

#include <QtCore/QHash>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QMetaMethod>
#include <QtCore/QMetaObject>
#include <QtCore/QSet>
#include <QtCore/QUrl>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QInputMethod>
#include <QtGui/QInputMethodEvent>
#include <QtGui/QPainter>
#include <QtGui/QPixmap>
#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGRendererInterface>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/QSGTexture>

#ifdef _WIN32
#include <QtQuick/qsgtexture_platform.h>
#include <windows.h>
#include <d3d11_1.h>
#include <dxgi1_2.h>
#include <propkey.h>
#include <propsys.h>
#include <shobjidl.h>
#include <wrl/client.h>

using Microsoft::WRL::ComPtr;
#endif

#include <algorithm>
#ifdef _WIN32
#include <chrono>
#endif
#include <memory>
#include <mutex>
#include <vector>

namespace {

struct Latest
{
    QImage image;
    quint64 serial = 0;
#ifdef _WIN32
    quint64 gpuEpoch = 0;
    QSize gpuSize;
    std::shared_ptr<void> gpuHandle;
#endif

    bool isValid() const
    {
#ifdef _WIN32
        if (gpuHandle && !gpuSize.isEmpty())
            return true;
#endif
        return !image.isNull();
    }

    bool isGpu() const
    {
#ifdef _WIN32
        return gpuHandle && !gpuSize.isEmpty();
#else
        return false;
#endif
    }

    QSize size() const
    {
#ifdef _WIN32
        if (gpuHandle && !gpuSize.isEmpty())
            return gpuSize;
#endif
        return image.size();
    }
};

// Pictures by stream, and the views to tell. Views register and leave
// under the lock, so a picture never wakes a view that's being destroyed.
// Never destroyed: a decoder thread can still be finishing while the app exits, after
// destructors of globals have run (a crash on quitting with a mirror open).
std::mutex &lock = *new std::mutex;
QHash<QString, Latest> &latest = *new QHash<QString, Latest>;
std::vector<VideoView *> &views = *new std::vector<VideoView *>;
quint64 nextSerial = 1;
#ifdef _WIN32
bool g_gpuUnsupported = false;
QSet<QString> &failedGpuStreams = *new QSet<QString>;

qint64 steadyMs()
{
    return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
        .count();
}
#endif

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

// Owns its CPU or D3D11-backed texture.
class VideoNode : public QSGSimpleTextureNode
{
public:
    VideoNode() { setOwnsTexture(false); }
    ~VideoNode() override
    {
        setTexture(nullptr);
#ifdef _WIN32
        m_sgTexture.reset();
#endif
        delete m_cpuTexture;
        m_cpuTexture = nullptr;
    }

    void replaceCpuTexture(QSGTexture *next, bool keepGpu = false)
    {
        setTexture(next);
#ifdef _WIN32
        m_sgTexture.reset();
#endif
        delete m_cpuTexture;
        m_cpuTexture = next;
#ifdef _WIN32
        if (!keepGpu)
            resetGpu();
#else
        Q_UNUSED(keepGpu)
#endif
    }

#ifdef _WIN32
    enum class GpuStatus
    {
        Ok,
        Busy,
        Fatal,
    };

    bool ensureShared(QQuickWindow *window, quint64 epoch, HANDLE handle, bool createRenderTexture, bool *nonD3d11)
    {
        *nonD3d11 = false;
        QSGRendererInterface *ri = window->rendererInterface();
        if (!ri || ri->graphicsApi() != QSGRendererInterface::Direct3D11) {
            *nonD3d11 = true;
            return false;
        }
        auto *device = static_cast<ID3D11Device *>(ri->getResource(window, QSGRendererInterface::DeviceResource));
        auto *context = static_cast<ID3D11DeviceContext *>(
            ri->getResource(window, QSGRendererInterface::DeviceContextResource));
        if (!device || !context)
            return false;

        if (m_epoch == epoch && m_device == device && m_shared && m_mutex
            && (!createRenderTexture || (m_texture && m_sgTexture))) {
            return true;
        }

        setTexture(nullptr);
        m_sgTexture.reset();
        resetGpu();
        delete m_cpuTexture;
        m_cpuTexture = nullptr;

        ComPtr<ID3D11Device1> device1;
        if (FAILED(device->QueryInterface(IID_PPV_ARGS(&device1))))
            return false;
        if (FAILED(device1->OpenSharedResource1(handle, IID_PPV_ARGS(&m_shared))) || !m_shared
            || FAILED(m_shared.As(&m_mutex)) || !m_mutex) {
            resetGpu();
            return false;
        }
        D3D11_TEXTURE2D_DESC desc {};
        m_shared->GetDesc(&desc);
        // QSGD3D11Texture::fromNative assumes RGBA8.
        if (desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM || desc.Width == 0 || desc.Height == 0) {
            resetGpu();
            return false;
        }
        if (createRenderTexture) {
            D3D11_TEXTURE2D_DESC texDesc = desc;
            texDesc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
            texDesc.MiscFlags = 0;
            if (FAILED(device->CreateTexture2D(&texDesc, nullptr, &m_texture)) || !m_texture) {
                resetGpu();
                return false;
            }
            m_sgTexture.reset(QNativeInterface::QSGD3D11Texture::fromNative(
                m_texture.Get(), window, QSize(int(desc.Width), int(desc.Height))));
            if (!m_sgTexture) {
                resetGpu();
                return false;
            }
            m_sgTexture->setFiltering(QSGTexture::Linear);
            setTexture(m_sgTexture.get());
        }
        m_device = device;
        m_context = context;
        m_epoch = epoch;
        return true;
    }

    GpuStatus copyGpuFrame()
    {
        if (!m_mutex || !m_context || !m_texture || !m_shared)
            return GpuStatus::Fatal;
        // S_OK only: WAIT_TIMEOUT is a success code meaning "not acquired".
        if (m_mutex->AcquireSync(0, 0) != S_OK)
            return GpuStatus::Busy;
        m_context->CopyResource(m_texture.Get(), m_shared.Get());
        m_mutex->ReleaseSync(0);
        markDirty(QSGNode::DirtyMaterial);
        return GpuStatus::Ok;
    }

    GpuStatus readGpuImage(QQuickWindow *window, quint64 epoch, HANDLE handle, QImage *out, bool *nonD3d11)
    {
        *out = QImage();
        if (!ensureShared(window, epoch, handle, false, nonD3d11))
            return GpuStatus::Fatal;
        D3D11_TEXTURE2D_DESC desc {};
        m_shared->GetDesc(&desc);
        if (!m_staging) {
            D3D11_TEXTURE2D_DESC stagingDesc = desc;
            stagingDesc.BindFlags = 0;
            stagingDesc.MiscFlags = 0;
            stagingDesc.Usage = D3D11_USAGE_STAGING;
            stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            if (FAILED(m_device->CreateTexture2D(&stagingDesc, nullptr, &m_staging)) || !m_staging)
                return GpuStatus::Fatal;
        }
        if (m_mutex->AcquireSync(0, 0) != S_OK)
            return GpuStatus::Busy;
        m_context->CopyResource(m_staging.Get(), m_shared.Get());
        m_mutex->ReleaseSync(0);

        D3D11_MAPPED_SUBRESOURCE mapped {};
        if (FAILED(m_context->Map(m_staging.Get(), 0, D3D11_MAP_READ, 0, &mapped)) || !mapped.pData)
            return GpuStatus::Fatal;
        QImage img(int(desc.Width), int(desc.Height), QImage::Format_RGB32);
        if (!img.isNull() && mapped.RowPitch >= desc.Width * 4) {
            const auto *srcBase = static_cast<const uint8_t *>(mapped.pData);
            for (UINT y = 0; y < desc.Height; ++y) {
                const uint8_t *src = srcBase + size_t(y) * mapped.RowPitch;
                uint8_t *dst = img.scanLine(int(y));
                for (UINT x = 0; x < desc.Width; ++x) {
                    dst[x * 4 + 0] = src[x * 4 + 2];
                    dst[x * 4 + 1] = src[x * 4 + 1];
                    dst[x * 4 + 2] = src[x * 4 + 0];
                    dst[x * 4 + 3] = 0xFF;
                }
            }
        }
        m_context->Unmap(m_staging.Get(), 0);
        if (img.isNull())
            return GpuStatus::Fatal;
        *out = std::move(img);
        return GpuStatus::Ok;
    }
#endif

private:
#ifdef _WIN32
    void resetGpu()
    {
        m_shared.Reset();
        m_mutex.Reset();
        m_texture.Reset();
        m_staging.Reset();
        m_device = nullptr;
        m_context = nullptr;
        m_epoch = 0;
    }
#endif

    QSGTexture *m_cpuTexture = nullptr;
#ifdef _WIN32
    ComPtr<ID3D11Texture2D> m_shared;
    ComPtr<IDXGIKeyedMutex> m_mutex;
    ComPtr<ID3D11Texture2D> m_texture;
    ComPtr<ID3D11Texture2D> m_staging;
    // Wraps m_texture without owning it, so it must be destroyed first.
    std::unique_ptr<QSGTexture> m_sgTexture;
    ID3D11Device *m_device = nullptr;       // owned by Qt
    ID3D11DeviceContext *m_context = nullptr; // owned by Qt
    quint64 m_epoch = 0;
#endif
};

} // namespace

VideoView::VideoView(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    QGuiApplication::instance()->installNativeEventFilter(this);
#ifdef _WIN32
    m_gpuCheckTimer.setSingleShot(true);
    m_gpuCheckTimer.setTimerType(Qt::PreciseTimer);
    connect(&m_gpuCheckTimer, &QTimer::timeout, this, [this]() {
        if (!m_frozen || !m_holding.load(std::memory_order_relaxed) || !m_watchChanges || m_changedSinceFrozen)
            return;
        const qint64 last = m_lastGpuCheckMs.load(std::memory_order_relaxed);
        const qint64 elapsed = steadyMs() - last;
        if (last != 0 && elapsed < 100)
            m_gpuCheckTimer.start(qMax(1, int(100 - elapsed)));
        else
            update();
    });
#endif
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

void VideoView::setWindowRelaunch(const QString &relaunch)
{
    if (relaunch == m_windowRelaunch)
        return;
    m_windowRelaunch = relaunch;
    emit windowRelaunchChanged();
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
    m_holding.store(false, std::memory_order_relaxed);
#ifdef _WIN32
    m_lastFrozenCheckSerial = 0;
    m_lastGpuCheckMs.store(0, std::memory_order_relaxed);
    m_gpuCheckTimer.stop();
#endif
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

void VideoView::setWatchChanges(bool on)
{
    if (on == m_watchChanges)
        return;
    m_watchChanges = on;
#ifdef _WIN32
    m_lastFrozenCheckSerial = 0;
    m_lastGpuCheckMs.store(0, std::memory_order_relaxed);
    m_gpuCheckTimer.stop();
#endif
    emit watchChangesChanged();
    if (on && m_frozen && !m_changedSinceFrozen) {
        QImage image;
        {
            std::lock_guard guard(lock);
            image = latest.value(m_stream).image;
        }
        if (!image.isNull() && looksDifferent(m_kept, image)) {
            m_changedSinceFrozen = true;
            emit changedSinceFrozenChanged();
        }
        update();
    }
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

#ifdef _WIN32
namespace {
// Takes the window's taskbar ID and relaunch details off it again.
void clearTaskbarProperties(QWindow *win)
{
    HWND hwnd = reinterpret_cast<HWND>(win->winId());
    IPropertyStore *store = nullptr;
    if (!hwnd || FAILED(SHGetPropertyStoreForWindow(hwnd, IID_PPV_ARGS(&store))) || !store)
        return;
    PROPVARIANT empty{};
    for (const PROPERTYKEY &key : {PKEY_AppUserModel_RelaunchIconResource,
                                   PKEY_AppUserModel_RelaunchDisplayNameResource,
                                   PKEY_AppUserModel_RelaunchCommand, PKEY_AppUserModel_ID})
        store->SetValue(key, empty);
    store->Commit();
    store->Release();
}
} // namespace
#endif

void VideoView::itemChange(ItemChange change, const ItemChangeData &value)
{
    QQuickItem::itemChange(change, value);
    if (change == ItemSceneChange && value.window) {
        connect(value.window, &QWindow::visibleChanged, this, [this](bool) { applyWindowChrome(); });
#ifdef _WIN32
        // Windows wants the taskbar properties gone before the window is.
        QQuickWindow *win = value.window;
        connect(win, &QWindow::visibleChanged, this, [win](bool visible) {
            if (!visible)
                clearTaskbarProperties(win);
        });
#endif
        applyWindowChrome();
    }
}

void VideoView::applyWindowChrome()
{
    QQuickWindow *win = window();
    if (!win)
        return;
#ifdef _WIN32
    if (!m_windowAppId.isEmpty() && win->isVisible()) {
        if (HWND hwnd = reinterpret_cast<HWND>(win->winId())) {
            IPropertyStore *store = nullptr;
            if (SUCCEEDED(SHGetPropertyStoreForWindow(hwnd, IID_PPV_ARGS(&store))) && store) {
                auto set = [store](const PROPERTYKEY &key, const QString &text) {
                    PROPVARIANT pv{};
                    pv.vt = VT_LPWSTR;
                    pv.pwszVal = const_cast<LPWSTR>(reinterpret_cast<LPCWSTR>(text.utf16()));
                    store->SetValue(key, pv);
                };
                // Pinned, the window starts its app again, not NectarLink
                // (#465). Windows reads these with the ID, so they go first.
                const QJsonObject relaunch = QJsonDocument::fromJson(m_windowRelaunch.toUtf8()).object();
                const QString command = relaunch.value(QStringLiteral("command")).toString();
                const QString name = relaunch.value(QStringLiteral("name")).toString();
                const QString icon = relaunch.value(QStringLiteral("icon")).toString();
                PROPVARIANT empty{};
                if (!command.isEmpty() && !name.isEmpty()) {
                    set(PKEY_AppUserModel_RelaunchCommand, command);
                    set(PKEY_AppUserModel_RelaunchDisplayNameResource, name);
                    if (!icon.isEmpty())
                        set(PKEY_AppUserModel_RelaunchIconResource, icon);
                    else
                        store->SetValue(PKEY_AppUserModel_RelaunchIconResource, empty);
                } else {
                    store->SetValue(PKEY_AppUserModel_RelaunchIconResource, empty);
                    store->SetValue(PKEY_AppUserModel_RelaunchDisplayNameResource, empty);
                    store->SetValue(PKEY_AppUserModel_RelaunchCommand, empty);
                }
                set(PKEY_AppUserModel_ID, m_windowAppId);
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
    Latest picture;
    {
        std::lock_guard guard(lock);
        picture = latest.value(m_stream);
    }
    const QSize size = picture.size();
    if (m_frozen && m_watchChanges && !m_changedSinceFrozen && !picture.image.isNull()
        && looksDifferent(m_kept, picture.image)) {
        m_changedSinceFrozen = true;
        emit changedSinceFrozenChanged();
    }
    if (size != m_frameSize) {
        m_frameSize = size;
        emit frameSizeChanged();
    }
    ++m_frames;
    emit framesChanged();
    if (!m_frozen || !m_holding.load(std::memory_order_relaxed)) {
        update();
#ifdef _WIN32
    } else if (m_watchChanges && !m_changedSinceFrozen && picture.isGpu()) {
        const qint64 last = m_lastGpuCheckMs.load(std::memory_order_relaxed);
        const qint64 elapsed = steadyMs() - last;
        if (last == 0 || elapsed >= 100) {
            m_gpuCheckTimer.stop();
            update();
        } else if (!m_gpuCheckTimer.isActive()) {
            m_gpuCheckTimer.start(qMax(1, int(100 - elapsed)));
        }
#endif
    }
}

QSGNode *VideoView::updatePaintNode(QSGNode *old, UpdatePaintNodeData *)
{
    Latest picture;
    {
        std::lock_guard guard(lock);
        picture = latest.value(m_stream);
    }
    if (!picture.isValid() || !window()) {
        delete old;
        m_shown = 0;
        return nullptr;
    }
    auto *node = static_cast<VideoNode *>(old);
    if (!node)
        node = new VideoNode;

#ifdef _WIN32
    if (picture.isGpu()) {
        const HANDLE handle = reinterpret_cast<HANDLE>(picture.gpuHandle.get());
        const bool holding = m_holding.load(std::memory_order_relaxed);
        if (m_frosted || m_frozen) {
            const bool needDisplay = !node->texture() || (picture.serial != m_shown && !holding)
                || (m_frozen && !holding && m_kept.isNull());
            if (needDisplay) {
                QImage img;
                bool nonD3d11 = false;
                const auto status = node->readGpuImage(window(), picture.gpuEpoch, handle, &img, &nonD3d11);
                if (status == VideoNode::GpuStatus::Fatal) {
                    {
                        std::lock_guard guard(lock);
                        if (nonD3d11)
                            g_gpuUnsupported = true;
                        else
                            failedGpuStreams.insert(m_stream);
                    }
                    delete node;
                    m_shown = 0;
                    return nullptr;
                }
                if (status == VideoNode::GpuStatus::Busy) {
                    update();
                } else {
                    if (m_frozen && m_kept.isNull()) {
                        m_kept = img;
                        m_lastFrozenCheckSerial = picture.serial;
                        m_lastGpuCheckMs.store(steadyMs(), std::memory_order_relaxed);
                    }
                    QSGTexture *texture = window()->createTextureFromImage(m_frosted ? frost(img) : img);
                    texture->setFiltering(QSGTexture::Linear);
                    node->replaceCpuTexture(texture, true);
                    m_shown = picture.serial;
                    m_holding.store(m_frozen, std::memory_order_relaxed);
                }
            } else if (m_frozen && holding && m_watchChanges && !m_changedSinceFrozen
                       && picture.serial != m_lastFrozenCheckSerial) {
                const qint64 last = m_lastGpuCheckMs.load(std::memory_order_relaxed);
                const qint64 now = steadyMs();
                if (last == 0 || now - last >= 100) {
                    QImage newest;
                    bool nonD3d11 = false;
                    const auto status = node->readGpuImage(window(), picture.gpuEpoch, handle, &newest, &nonD3d11);
                    if (status == VideoNode::GpuStatus::Busy) {
                        update();
                    } else if (status == VideoNode::GpuStatus::Ok) {
                        m_lastFrozenCheckSerial = picture.serial;
                        m_lastGpuCheckMs.store(now, std::memory_order_relaxed);
                        if (!m_kept.isNull() && looksDifferent(m_kept, newest)) {
                            QMetaObject::invokeMethod(this, [this]() {
                                if (m_frozen && m_watchChanges && !m_changedSinceFrozen) {
                                    m_changedSinceFrozen = true;
                                    emit changedSinceFrozenChanged();
                                }
                            }, Qt::QueuedConnection);
                        }
                    }
                }
            }
        } else {
            bool nonD3d11 = false;
            if (!node->ensureShared(window(), picture.gpuEpoch, handle, true, &nonD3d11)) {
                {
                    std::lock_guard guard(lock);
                    if (nonD3d11)
                        g_gpuUnsupported = true;
                    else
                        failedGpuStreams.insert(m_stream);
                }
                delete node;
                m_shown = 0;
                return nullptr;
            }
            if (picture.serial != m_shown) {
                const auto status = node->copyGpuFrame();
                if (status == VideoNode::GpuStatus::Fatal) {
                    {
                        std::lock_guard guard(lock);
                        failedGpuStreams.insert(m_stream);
                    }
                    delete node;
                    m_shown = 0;
                    return nullptr;
                }
                if (status == VideoNode::GpuStatus::Busy)
                    update();
                else
                    m_shown = picture.serial;
            }
            m_holding.store(false, std::memory_order_relaxed);
        }
    } else
#endif
    {
        if (!node->texture() || (picture.serial != m_shown && !m_holding.load(std::memory_order_relaxed))) {
            QSGTexture *texture = window()->createTextureFromImage(m_frosted ? frost(picture.image) : picture.image);
            texture->setFiltering(QSGTexture::Linear);
            node->replaceCpuTexture(texture);
            m_shown = picture.serial;
            m_holding.store(m_frozen, std::memory_order_relaxed);
        }
    }

    if (!node->texture()) {
        delete node;
        m_shown = 0;
        return nullptr;
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
    Latest entry;
    entry.image = std::move(copy);
    entry.serial = nextSerial++;
    latest.insert(key, std::move(entry));
    wakeViews(key);
}

bool video_gpu_frame(rust::Str stream, uint32_t width, uint32_t height, uint64_t epoch, size_t handle)
{
#ifndef _WIN32
    Q_UNUSED(stream)
    Q_UNUSED(width)
    Q_UNUSED(height)
    Q_UNUSED(epoch)
    Q_UNUSED(handle)
    return false;
#else
    if (width == 0 || height == 0 || epoch == 0 || handle == 0)
        return false;
    const auto api = QQuickWindow::graphicsApi();
    if (api != QSGRendererInterface::Unknown && api != QSGRendererInterface::Direct3D11)
        return false;

    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    {
        std::lock_guard guard(lock);
        if (g_gpuUnsupported || failedGpuStreams.contains(key))
            return false;
        auto it = latest.find(key);
        if (it != latest.end() && it->gpuEpoch == epoch && it->gpuHandle) {
            it->image = QImage();
            it->gpuSize = QSize(int(width), int(height));
            it->serial = nextSerial++;
            wakeViews(key);
            return true;
        }
    }

    HANDLE dup = nullptr;
    if (!DuplicateHandle(GetCurrentProcess(), reinterpret_cast<HANDLE>(uintptr_t(handle)), GetCurrentProcess(), &dup,
                         0, FALSE, DUPLICATE_SAME_ACCESS)
        || !dup) {
        return false;
    }
    auto sharedHandle = std::shared_ptr<void>(dup, [](void *h) {
        if (h && h != INVALID_HANDLE_VALUE)
            CloseHandle(h);
    });

    std::lock_guard guard(lock);
    if (g_gpuUnsupported || failedGpuStreams.contains(key))
        return false;
    Latest entry;
    entry.serial = nextSerial++;
    entry.gpuEpoch = epoch;
    entry.gpuSize = QSize(int(width), int(height));
    entry.gpuHandle = std::move(sharedHandle);
    latest.insert(key, std::move(entry));
    wakeViews(key);
    return true;
#endif
}

void video_clear(rust::Str stream)
{
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.remove(key);
#ifdef _WIN32
    failedGpuStreams.remove(key);
#endif
    wakeViews(key);
}

void clear_idle_video_frames()
{
    std::lock_guard guard(lock);
    if (views.empty()) {
        latest.clear();
        latest.squeeze();
#ifdef _WIN32
        failedGpuStreams.clear();
#endif
    }
}

