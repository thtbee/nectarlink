// SPDX-License-Identifier: GPL-3.0-or-later
#include "video_view.h"

#include <QtCore/QHash>
#include <QtCore/QUrl>
#include <QtGui/QIcon>
#include <QtGui/QImage>
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
std::mutex lock;
QHash<QString, Latest> latest;
std::vector<VideoView *> views;
quint64 nextSerial = 1;

void wakeViews(const QString &stream)
{
    for (VideoView *view : views) {
        if (view->stream() == stream)
            QMetaObject::invokeMethod(view, "frameArrived", Qt::QueuedConnection);
    }
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
    std::lock_guard guard(lock);
    views.push_back(this);
}

VideoView::~VideoView()
{
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
    const QSizeF fitted = QSizeF(m_frameSize).scaled(size(), Qt::KeepAspectRatio);
    return QRectF((width() - fitted.width()) / 2, (height() - fitted.height()) / 2, fitted.width(), fitted.height());
}

void VideoView::frameArrived()
{
    QSize size;
    {
        std::lock_guard guard(lock);
        size = latest.value(m_stream).image.size();
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
    if (picture.serial != m_shown || !node->texture()) {
        QSGTexture *texture = window()->createTextureFromImage(picture.image);
        texture->setFiltering(QSGTexture::Linear);
        node->replaceTexture(texture);
        m_shown = picture.serial;
    }
    node->setFiltering(QSGTexture::Linear);
    node->setRect(pictureRect());
    return node;
}

void video_frame(rust::Str stream, uint32_t width, uint32_t height, rust::Slice<const uint8_t> bgrx)
{
    if (bgrx.size() < size_t(width) * height * 4)
        return;
    // One copy, into an image Qt owns.
    QImage image(reinterpret_cast<const uchar *>(bgrx.data()), int(width), int(height), int(width) * 4,
                 QImage::Format_RGB32);
    const QString key = QString::fromUtf8(stream.data(), qsizetype(stream.size()));
    std::lock_guard guard(lock);
    latest.insert(key, Latest{image.copy(), nextSerial++});
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

