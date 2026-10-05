// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtCore/QPointer>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>
#include <QtQuick/QQuickWindow>

#include <vector>

// A top-level window drawn edge to edge: the app draws its own title bar and
// caption buttons, while Windows keeps the native behaviors (dragging,
// double-click to maximize, Aero Snap, the system menu, and the Snap Layouts
// flyout on the maximize button). Optionally shows the Windows 11 Mica
// backdrop behind transparent content.
class NativeWindow : public QQuickWindow
{
    Q_OBJECT
    QML_ELEMENT

    // Show Mica behind transparent areas.
    Q_PROPERTY(bool backdrop READ backdrop WRITE setBackdrop NOTIFY backdropChanged)
    // Dark backdrop tint and window border.
    Q_PROPERTY(bool darkFrame READ darkFrame WRITE setDarkFrame NOTIFY darkFrameChanged)
    // Height of the draggable title strip, in logical pixels.
    Q_PROPERTY(qreal captionHeight READ captionHeight WRITE setCaptionHeight NOTIFY captionHeightChanged)
    // The app's maximize button. Windows treats it as the system maximize
    // button (Snap Layouts), so its mouse input arrives here, not in QML:
    // QML styles it from `maximizeHovered` and `maximizePressed`.
    Q_PROPERTY(QQuickItem *maximizeButton READ maximizeButton WRITE setMaximizeButton NOTIFY maximizeButtonChanged)
    Q_PROPERTY(bool maximizeHovered READ maximizeHovered NOTIFY maximizeStateChanged)
    Q_PROPERTY(bool maximizePressed READ maximizePressed NOTIFY maximizeStateChanged)

public:
    explicit NativeWindow(QWindow *parent = nullptr);

    bool backdrop() const { return m_backdrop; }
    void setBackdrop(bool on);
    bool darkFrame() const { return m_darkFrame; }
    void setDarkFrame(bool dark);
    qreal captionHeight() const { return m_captionHeight; }
    void setCaptionHeight(qreal height);
    QQuickItem *maximizeButton() const { return m_maximizeButton; }
    void setMaximizeButton(QQuickItem *item);
    bool maximizeHovered() const { return m_maximizeHovered; }
    bool maximizePressed() const { return m_maximizePressed; }

    // Interactive items inside the title strip (buttons, search fields):
    // clicks there go to the item instead of dragging the window.
    Q_INVOKABLE void addCaptionHole(QQuickItem *item);
    Q_INVOKABLE void removeCaptionHole(QQuickItem *item);

    Q_INVOKABLE void toggleMaximized();
    // Shows, restores and focuses the window, even when another app is in
    // front (the caller must have the right to take the foreground).
    Q_INVOKABLE void bringToFront();
    // Flashes the taskbar button until the window is activated.
    Q_INVOKABLE void flash();

signals:
    void backdropChanged();
    void darkFrameChanged();
    void captionHeightChanged();
    void maximizeButtonChanged();
    void maximizeStateChanged();

protected:
    bool event(QEvent *event) override;
    bool nativeEvent(const QByteArray &eventType, void *message, qintptr *result) override;

private:
    void applyFrame();
    long hitTest(int screenX, int screenY) const;
    bool itemContains(const QQuickItem *item, const QPointF &scenePoint) const;
    void setMaximizeState(bool hovered, bool pressed);

    bool m_backdrop = true;
    bool m_darkFrame = false;
    qreal m_captionHeight = 48;
    QPointer<QQuickItem> m_maximizeButton;
    bool m_maximizeHovered = false;
    bool m_maximizePressed = false;
    std::vector<QPointer<QQuickItem>> m_holes;
};
