// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtCore/QAbstractNativeEventFilter>
#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtGui/QImage>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

#include <rust/cxx.h>

#include <atomic>
#include <cstdint>

// Shows the latest picture of a video stream (a phone's screen), scaled to
// fit with its aspect ratio kept. Pictures come from Rust, on any thread,
// through `video_frame`; only the newest is drawn.
class VideoView : public QQuickItem, public QAbstractNativeEventFilter
{
    Q_OBJECT
    QML_ELEMENT

    // Which stream to show (the phone's device ID).
    Q_PROPERTY(QString stream READ stream WRITE setStream NOTIFY streamChanged)
    // The pictures' size, in pixels (empty before the first one).
    Q_PROPERTY(QSize frameSize READ frameSize NOTIFY frameSizeChanged)
    // Pictures shown so far, for a frame rate readout.
    Q_PROPERTY(quint64 frames READ frames NOTIFY framesChanged)
    // Optional PNG icon file URL/path for the containing window and taskbar button.
    Q_PROPERTY(QString windowIcon READ windowIcon WRITE setWindowIcon NOTIFY windowIconChanged)
    // Optional Windows AppUserModelID so each app window gets its own taskbar identity.
    Q_PROPERTY(QString windowAppId READ windowAppId WRITE setWindowAppId NOTIFY windowAppIdChanged)
    // Whether text from input methods (the emoji panel, IMEs for Chinese and
    // other languages) is taken, and given out through `textCommitted`.
    Q_PROPERTY(bool acceptsText READ acceptsText WRITE setAcceptsText NOTIFY acceptsTextChanged)
    // Frosted, the picture is drawn blurred and dimmed, and covers the item
    // (what overflows is cut off): what a window shows while it's resized.
    Q_PROPERTY(bool frosted READ frosted WRITE setFrosted NOTIFY frostedChanged)
    // A picture within 2% of the item's shape fills it instead of leaving
    // thin bars (an app window's, whose sides the phone rounds to 16 pixels).
    Q_PROPERTY(bool fillsNearShape MEMBER m_fillsNearShape NOTIFY fillsNearShapeChanged)
    // Frozen, the view keeps the picture it has when this turns on.
    Q_PROPERTY(bool frozen READ frozen WRITE setFrozen NOTIFY frozenChanged)
    // Frozen, and a picture has come since that looks different from the
    // kept one (the app drew itself anew, say after a change of zoom).
    Q_PROPERTY(bool changedSinceFrozen READ changedSinceFrozen NOTIFY changedSinceFrozenChanged)
    // While the user drags its edges, the window keeps the picture's shape
    // (on Windows; maximized and full screen are left alone).
    Q_PROPERTY(bool keepShape READ keepShape WRITE setKeepShape NOTIFY keepShapeChanged)
    // The user holds an edge of the window: on from the moment it's grabbed,
    // before the size changes, off when it's let go. Windows only; elsewhere
    // never on.
    Q_PROPERTY(bool sizing READ sizing NOTIFY sizingChanged)

public:
    explicit VideoView(QQuickItem *parent = nullptr);
    ~VideoView() override;

    QString stream() const { return m_stream; }
    void setStream(const QString &stream);
    QSize frameSize() const { return m_frameSize; }
    quint64 frames() const { return m_frames; }
    QString windowIcon() const { return m_windowIcon; }
    void setWindowIcon(const QString &icon);
    QString windowAppId() const { return m_windowAppId; }
    void setWindowAppId(const QString &appId);
    bool acceptsText() const { return m_acceptsText; }
    void setAcceptsText(bool on);
    bool frosted() const { return m_frosted; }
    void setFrosted(bool on);
    bool frozen() const { return m_frozen; }
    void setFrozen(bool on);
    bool changedSinceFrozen() const { return m_changedSinceFrozen; }
    bool keepShape() const { return m_keepShape; }
    void setKeepShape(bool on);
    bool sizing() const { return m_sizing; }

    bool nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result) override;

    // Where the picture is drawn in the item (letterboxed).
    Q_INVOKABLE QRectF pictureRect() const;

    // Any thread, while the view registry's lock protects this view's lifetime.
    void queueFrame();

signals:
    void streamChanged();
    void frameSizeChanged();
    void framesChanged();
    void windowIconChanged();
    void windowAppIdChanged();
    void acceptsTextChanged();
    void frostedChanged();
    void frozenChanged();
    void changedSinceFrozenChanged();
    void fillsNearShapeChanged();
    void keepShapeChanged();
    void sizingChanged();
    // An input method finished some text (an emoji, a word in Chinese...).
    void textCommitted(const QString &text);

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    void inputMethodEvent(QInputMethodEvent *event) override;
    QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;

private:
    Q_INVOKABLE void frameArrived();
    void applyWindowChrome();

    QString m_stream;
    QSize m_frameSize;
    quint64 m_frames = 0;
    // The serial of the picture on screen.
    quint64 m_shown = 0;
    QString m_windowIcon;
    QString m_windowAppId;
    bool m_acceptsText = false;
    bool m_frosted = false;
    bool m_frozen = false;
    bool m_fillsNearShape = false;
    // Frozen, and the picture to keep is on screen.
    bool m_holding = false;
    // The picture when it froze, to tell a different one by.
    QImage m_kept;
    bool m_changedSinceFrozen = false;
    bool m_keepShape = false;
    bool m_sizing = false;
    std::atomic_bool m_framePending{false};
};

// The latest picture of `stream`: `width` x `height`, 32-bit BGRX rows.
// Any thread.
void video_frame(rust::Str stream, uint32_t width, uint32_t height, rust::Slice<const uint8_t> bgrx);

// The stream ended: views of it go blank. Any thread.
void video_clear(rust::Str stream);

// Drops cached video frames for streams that have no live VideoView.
void clear_idle_video_frames();

