// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// Hover hint for an Island control: the Island draws the bubble so it follows
// the pointer, stays inside the window and never opens a second window. The
// owner is passed explicitly because it is often an inline component.
HoverHandler {
    id: tipHover
    property var island
    property string tip
    property Item owner

    onHoveredChanged: {
        if (!island || !owner)
            return
        if (hovered)
            island.showTip(tip, owner, point.position)
        else
            island.hideTip(owner)
    }
    onPointChanged: {
        if (hovered && island && owner)
            island.moveTip(owner, point.position)
    }
    onTipChanged: {
        if (hovered && island && owner)
            island.showTip(tip, owner, point.position)
    }
}
