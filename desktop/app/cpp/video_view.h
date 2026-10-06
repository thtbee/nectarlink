// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

#include <rust/cxx.h>

#include <cstdint>

// Shows the latest picture of a video stream (a phone's screen), scaled to
// fit with its aspect ratio kept. Pictures come from Rust, on any thread,
// through `video_frame`; only the newest is drawn.
class VideoView : public QQuickItem
{
    Q_OBJECT
    QML_ELEMENT

    // Which stream to show (the phone's device ID).
    Q_PROPERTY(QString stream READ stream WRITE setStream NOTIFY streamChanged)
    // The pictures' size, in pixels (empty before the first one).
    Q_PROPERTY(QSize frameSize READ frameSize NOTIFY frameSizeChanged)
    // Pictures shown so far, for a frame rate readout.
    Q_PROPERTY(quint64 frames READ frames NOTIFY framesChanged)

public:
    explicit VideoView(QQuickItem *parent = nullptr);
    ~VideoView() override;

    QString stream() const { return m_stream; }
    void setStream(const QString &stream);
    QSize frameSize() const { return m_frameSize; }
    quint64 frames() const { return m_frames; }

    // Where the picture is drawn in the item (letterboxed).
    Q_INVOKABLE QRectF pictureRect() const;

signals:
    void streamChanged();
    void frameSizeChanged();
    void framesChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;

private:
    Q_INVOKABLE void frameArrived();

    QString m_stream;
    QSize m_frameSize;
    quint64 m_frames = 0;
    // The serial of the picture on screen.
    quint64 m_shown = 0;
};

// The latest picture of `stream`: `width` x `height`, 32-bit BGRX rows.
// Any thread.
void video_frame(rust::Str stream, uint32_t width, uint32_t height, rust::Slice<const uint8_t> bgrx);

// The stream ended: views of it go blank. Any thread.
void video_clear(rust::Str stream);
