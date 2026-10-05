// SPDX-License-Identifier: GPL-3.0-or-later
#include "frame_notifier.h"

namespace nl {

FrameNotifier::FrameNotifier(QQuickItem *item)
    : m_item(item)
{
}

void FrameNotifier::notify() const
{
    if (m_pending.exchange(true, std::memory_order_acq_rel))
        return; // an update is already queued and will show this frame
    std::lock_guard lock(m_mutex);
    QQuickItem *item = m_item.data();
    if (!item) {
        m_pending.store(false, std::memory_order_release);
        return;
    }
    // Queued with the item as context: Qt drops the call if the item is
    // deleted before it runs. detach() takes the mutex, so the item can't be
    // destroyed while it is being posted to.
    QMetaObject::invokeMethod(
        item,
        [this, item] {
            m_pending.store(false, std::memory_order_release);
            item->update();
        },
        Qt::QueuedConnection);
}

void FrameNotifier::detach()
{
    std::lock_guard lock(m_mutex);
    m_item.clear();
}

} // namespace nl
