// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A content surface: filled in Bloom, honey-edged in Hive, frosted optical glass in Glass,
// hairline-bordered in Graphite.
Rectangle {
    id: cardRoot
    property int padding: 16
    default property alias content: inner.data

    radius: Theme.radiusLg
    color: Theme.cardColor
    // Glass draws its own lit rim (GlassRim) instead of a flat hairline.
    border.width: (Theme.graphite || Theme.hive || Theme.fluent || Theme.ambientBlurActive || Theme.amoled) && !Theme.glass ? 1 : 0
    border.color: Theme.cardBorder
    implicitHeight: inner.childrenRect.height + padding * 2
    Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }

    GlassRim {
        radius: cardRoot.radius
    }

    Item {
        id: inner
        z: 1
        anchors.fill: parent
        anchors.margins: parent.padding
    }
}
