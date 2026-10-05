// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Effects
import app.nectarlink.spike

// Screen-mirroring preview: frames rendered on the GPU by the Rust producer,
// shown with rounded corners the way the mirror window is in the mockups.
Rectangle {
    id: panel
    property alias running: surface.running
    readonly property alias surface: surface

    width: 520
    height: 16 + 488 * 9 / 16 + 16
    radius: Theme.radiusXl
    color: Theme.surfaceContainerHighest
    visible: running
    layer.enabled: true
    layer.effect: MultiEffect { shadowEnabled: true; shadowBlur: 0.7; shadowOpacity: 0.18; shadowVerticalOffset: 10 }

    Item {
        id: screen
        anchors { fill: parent; margins: 16 }
        layer.enabled: true
        layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: mask
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
        }
        VideoSurface { id: surface; anchors.fill: parent }
    }
    Rectangle {
        id: mask
        anchors.fill: screen
        radius: Theme.radiusMd
        layer.enabled: true
        visible: false
    }
    Text {
        anchors.centerIn: screen
        visible: surface.error.length > 0
        text: surface.error
        font.family: Theme.font; font.pixelSize: 13; color: Theme.surfaceContentVariant
    }
}
