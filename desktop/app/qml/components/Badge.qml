// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A count of what's new, in a small red pill ("9+" above nine). Hidden at 0.
// `ring` is the color it's drawn over, so its edge cuts cleanly into it.
Rectangle {
    id: badge
    property int count: 0
    property color ring: Theme.surface

    visible: count > 0
    width: Math.max(16, label.implicitWidth + 8)
    height: 16
    radius: 8
    color: Theme.error
    border.width: 2
    border.color: ring

    Txt {
        id: label
        anchors.centerIn: parent
        text: badge.count > 9 ? "9+" : badge.count
        role: "label"
        size: 10
        weight: 700
        color: Theme.errorContent
    }
}
