// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Curved overhead specular sheen and luminous 1 px top-edge meniscus for
// Liquid Glass pills, tiles, and buttons.
Item {
    id: sheen
    property real radius: Theme.radiusMd
    property real strength: 1.0
    property bool filled: false

    anchors.fill: parent
    visible: Theme.glass && width > 0 && height > 0
    opacity: Math.min(1.0, 0.74 * strength)

    Behavior on strength {
        enabled: !Theme.reduceMotion
        NumberAnimation { duration: 190; easing.type: Easing.OutCubic }
    }

    // Soft top-half inner glass reflection
    Rectangle {
        anchors.fill: parent
        anchors.margins: 1
        radius: Math.max(0, sheen.radius - 1)
        gradient: Gradient {
            GradientStop {
                position: 0.0
                color: Qt.rgba(1, 1, 1, sheen.filled ? (Theme.dark ? 0.28 : 0.42) : (Theme.dark ? 0.18 : 0.52))
            }
            GradientStop { position: 0.54; color: Qt.rgba(1, 1, 1, 0.0) }
        }
    }

    // Crisp luminous top-edge meniscus catch
    Rectangle {
        anchors.top: parent.top
        anchors.topMargin: 1
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: Math.max(4, Math.min(sheen.radius * 0.65, Math.floor(parent.width * 0.22)))
        anchors.rightMargin: Math.max(4, Math.min(sheen.radius * 0.65, Math.floor(parent.width * 0.22)))
        height: 1
        color: Qt.rgba(1, 1, 1, sheen.filled ? (Theme.dark ? 0.46 : 0.72) : (Theme.dark ? 0.34 : 0.92))
    }
}
