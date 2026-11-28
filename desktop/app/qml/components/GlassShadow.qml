// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Soft multi-layered ambient drop shadow for floating Liquid Glass panes and
// elevated controls. Uses lightweight layered rectangles (no offscreen FBOs),
// so it scrolls cleanly inside Flickables.
Item {
    id: shadow
    property real radius: Theme.radiusLg
    property real elevation: 1.0

    anchors.fill: parent
    z: -1
    visible: Theme.glass && !Theme.amoled && width > 0 && height > 0

    readonly property real baseAlpha: (Theme.dark ? 0.24 : 0.075) * elevation

    Behavior on elevation {
        enabled: !Theme.reduceMotion
        NumberAnimation { duration: 190; easing.type: Easing.OutCubic }
    }

    // Wide ambient bloom
    Rectangle {
        x: -4 * shadow.elevation
        y: Math.round(6 * shadow.elevation)
        width: parent.width + 8 * shadow.elevation
        height: parent.height + 6 * shadow.elevation
        radius: shadow.radius + 4 * shadow.elevation
        color: Qt.rgba(0, 0, 0, Math.min(0.45, shadow.baseAlpha * 0.34))
    }

    // Mid soft drop
    Rectangle {
        x: -1.5 * shadow.elevation
        y: Math.round(3 * shadow.elevation)
        width: parent.width + 3 * shadow.elevation
        height: parent.height + 2.5 * shadow.elevation
        radius: shadow.radius + 1.5 * shadow.elevation
        color: Qt.rgba(0, 0, 0, Math.min(0.55, shadow.baseAlpha * 0.48))
    }

    // Tight contact shadow under the glass pane
    Rectangle {
        x: 0
        y: Math.round(1.2 * shadow.elevation)
        width: parent.width
        height: parent.height
        radius: shadow.radius
        color: Qt.rgba(0, 0, 0, Math.min(0.65, shadow.baseAlpha * 0.64))
    }
}
