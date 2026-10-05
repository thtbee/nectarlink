// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtCore/QPointer>
#include <QtQuick/QQuickItem>

#include <atomic>
#include <mutex>

namespace nl {

// Lets the Rust video producer wake a Qt Quick item from any thread.
// notify() is thread-safe and coalesced: at most one update() is queued at a
// time, so a fast producer can't flood the GUI thread.
class FrameNotifier
{
public:
    explicit FrameNotifier(QQuickItem *item);

    // Called from the producer thread.
    void notify() const;
    // Called on the GUI thread before the item goes away.
    void detach();

private:
    mutable std::mutex m_mutex;
    QPointer<QQuickItem> m_item;
    mutable std::atomic<bool> m_pending { false };
};

} // namespace nl
