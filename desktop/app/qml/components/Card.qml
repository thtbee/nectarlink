// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A content surface: filled in Bloom, hairline-bordered in Graphite.
Rectangle {
    property int padding: 16
    default property alias content: inner.data

    radius: Theme.radiusLg
    color: Theme.cardColor
    border.width: Theme.graphite ? 1 : 0
    border.color: Theme.cardBorder
    implicitHeight: inner.childrenRect.height + padding * 2
    Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }

    Item {
        id: inner
        anchors.fill: parent
        anchors.margins: parent.padding
    }
}
