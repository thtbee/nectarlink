// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

#include <memory>

namespace nl {

class FrameNotifier;

// Shows frames produced on the GPU by the Rust video source without copying
// them through the CPU. Requires the D3D11 scene graph backend.
class VideoSurface : public QQuickItem
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)

public:
    explicit VideoSurface(QQuickItem *parent = nullptr);
    ~VideoSurface() override;

    bool running() const { return m_running; }
    void setRunning(bool running);
    QString error() const { return m_error; }

    // {published, dropped, shown}: frames the producer published, frames it
    // dropped because the consumer held the texture, and frames copied for
    // display (published frames replaced before a render are never shown).
    Q_INVOKABLE QVariantMap stats() const;

signals:
    void runningChanged();
    void errorChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;

private:
    bool m_running = false;
    QString m_error;
    std::shared_ptr<FrameNotifier> m_notifier;
    // Written on the render thread during sync, while the GUI thread is blocked.
    quint64 m_published = 0;
    quint64 m_dropped = 0;
    quint64 m_shown = 0;
};

} // namespace nl
