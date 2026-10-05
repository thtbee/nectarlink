// SPDX-License-Identifier: GPL-3.0-or-later
#include "video_surface.h"

#include "frame_notifier.h"
#include "s1-qt-rust/src/video.cxx.h"

#include <rust/cxx.h>

#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGRendererInterface>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/qsgtexture_platform.h>

#include <d3d11_1.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

#include <optional>

using Microsoft::WRL::ComPtr;

namespace nl {

namespace {

constexpr quint32 kWidth = 1920;
constexpr quint32 kHeight = 1080;
constexpr quint32 kFps = 60;

// Owns the producer and the GPU resources on Qt's device. Lives on the render
// thread, so the producer stops whenever the scene graph lets go of the node.
class VideoNode final : public QSGSimpleTextureNode
{
public:
    static VideoNode *create(QQuickWindow *window, std::shared_ptr<FrameNotifier> notifier, QString *error)
    {
        QSGRendererInterface *ri = window->rendererInterface();
        if (ri->graphicsApi() != QSGRendererInterface::Direct3D11) {
            *error = QStringLiteral("video needs the D3D11 backend");
            return nullptr;
        }
        auto *device = static_cast<ID3D11Device *>(ri->getResource(window, QSGRendererInterface::DeviceResource));
        auto *context = static_cast<ID3D11DeviceContext *>(
            ri->getResource(window, QSGRendererInterface::DeviceContextResource));
        ComPtr<ID3D11Device1> device1;
        ComPtr<IDXGIDevice> dxgiDevice;
        ComPtr<IDXGIAdapter> adapter;
        DXGI_ADAPTER_DESC adapterDesc {};
        if (!device || !context || FAILED(device->QueryInterface(IID_PPV_ARGS(&device1)))
            || FAILED(device->QueryInterface(IID_PPV_ARGS(&dxgiDevice)))
            || FAILED(dxgiDevice->GetAdapter(&adapter)) || FAILED(adapter->GetDesc(&adapterDesc))) {
            *error = QStringLiteral("no usable D3D11 device");
            return nullptr;
        }
        const qint64 luid = (qint64(adapterDesc.AdapterLuid.HighPart) << 32) | adapterDesc.AdapterLuid.LowPart;

        auto node = std::unique_ptr<VideoNode>(new VideoNode);
        try {
            node->m_source.emplace(video_source_start(luid, kWidth, kHeight, kFps, std::move(notifier)));
        } catch (const rust::Error &e) {
            *error = QString::fromUtf8(e.what());
            return nullptr;
        }
        auto handle = reinterpret_cast<HANDLE>((*node->m_source)->shared_handle());
        if (FAILED(device1->OpenSharedResource1(handle, IID_PPV_ARGS(&node->m_shared)))
            || FAILED(node->m_shared.As(&node->m_mutex))) {
            *error = QStringLiteral("could not open the shared video texture");
            return nullptr;
        }
        D3D11_TEXTURE2D_DESC desc {};
        node->m_shared->GetDesc(&desc);
        // QSGD3D11Texture::fromNative assumes RGBA8.
        if (desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM) {
            *error = QStringLiteral("the video texture must be RGBA8");
            return nullptr;
        }
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        desc.MiscFlags = 0;
        if (FAILED(device->CreateTexture2D(&desc, nullptr, &node->m_texture))) {
            *error = QStringLiteral("could not create the video texture");
            return nullptr;
        }
        node->m_context = context;
        node->m_sgTexture.reset(QNativeInterface::QSGD3D11Texture::fromNative(
            node->m_texture.Get(), window, QSize(int(desc.Width), int(desc.Height))));
        node->setTexture(node->m_sgTexture.get());
        node->setFiltering(QSGTexture::Linear);
        return node.release();
    }

    // Copies the newest published frame, GPU to GPU. Returns false if the
    // producer held the texture and the copy should be retried next frame.
    bool sync(quint64 *shown)
    {
        const quint64 published = (*m_source)->frames_published();
        if (published == m_lastPublished)
            return true;
        // S_OK only: WAIT_TIMEOUT is a success code meaning "not acquired".
        if (m_mutex->AcquireSync(0, 0) != S_OK)
            return false;
        m_context->CopyResource(m_texture.Get(), m_shared.Get());
        m_mutex->ReleaseSync(0);
        m_lastPublished = published;
        ++*shown;
        markDirty(QSGNode::DirtyMaterial);
        return true;
    }

    quint64 published() const { return (*m_source)->frames_published(); }
    quint64 dropped() const { return (*m_source)->frames_dropped(); }

private:
    VideoNode() = default;

    // Declared first so it is destroyed last: the producer stops only after
    // Qt's references to the shared texture are released.
    std::optional<rust::Box<VideoSource>> m_source;
    ComPtr<ID3D11Texture2D> m_shared;
    ComPtr<IDXGIKeyedMutex> m_mutex;
    ComPtr<ID3D11Texture2D> m_texture;
    // Wraps m_texture without owning it, so it must be destroyed first.
    std::unique_ptr<QSGTexture> m_sgTexture;
    ID3D11DeviceContext *m_context = nullptr; // owned by Qt
    quint64 m_lastPublished = 0;
};

QRectF fitted(const QRectF &bounds, QSizeF content)
{
    content.scale(bounds.size(), Qt::KeepAspectRatio);
    return QRectF(bounds.center() - QPointF(content.width(), content.height()) / 2, content);
}

} // namespace

VideoSurface::VideoSurface(QQuickItem *parent)
    : QQuickItem(parent)
    , m_notifier(std::make_shared<FrameNotifier>(this))
{
    setFlag(ItemHasContents);
}

VideoSurface::~VideoSurface()
{
    m_notifier->detach();
}

void VideoSurface::setRunning(bool running)
{
    if (m_running == running)
        return;
    m_running = running;
    emit runningChanged();
    update();
}

QVariantMap VideoSurface::stats() const
{
    return {
        { QStringLiteral("published"), m_published },
        { QStringLiteral("dropped"), m_dropped },
        { QStringLiteral("shown"), m_shown },
    };
}

QSGNode *VideoSurface::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    auto *node = static_cast<VideoNode *>(oldNode);
    if (!m_running) {
        delete node;
        return nullptr;
    }
    if (!node) {
        QString error;
        node = VideoNode::create(window(), m_notifier, &error);
        if (!node) {
            if (error != m_error) {
                m_error = error;
                // The GUI thread is blocked during sync; emit once it resumes.
                QMetaObject::invokeMethod(this, &VideoSurface::errorChanged, Qt::QueuedConnection);
            }
            return nullptr;
        }
    }
    if (!node->sync(&m_shown))
        update(); // the producer held the texture; try again next frame
    m_published = node->published();
    m_dropped = node->dropped();
    node->setRect(fitted(boundingRect(), QSizeF(kWidth, kHeight)));
    return node;
}

} // namespace nl
