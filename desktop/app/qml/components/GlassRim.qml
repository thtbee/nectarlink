// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The rim of a Liquid Glass surface (shaders/glassrim.frag): a delicate outer
// contrast hairline, a crisp specular bevel lit from above-left with a soft
// bottom-right bounce, inner edge thickness glow, and a subtle top-down glass
// reflection. Fill its parent; it only draws in the Glass theme.
ShaderEffect {
    id: rim
    property real radius: Theme.radiusLg
    property real strength: 1.0
    property real sheenStrength: Theme.dark ? 0.045 : 0.13
    readonly property size itemSize: Qt.size(width, height)
    readonly property color rimColor: Theme.dark ? Qt.rgba(1, 1, 1, 0.56) : Qt.rgba(1, 1, 1, 0.96)
    readonly property color outerColor: Theme.dark ? Qt.rgba(0, 0, 0, 0.32) : Qt.rgba(0, 0, 0, 0.07)

    anchors.fill: parent
    visible: Theme.glass && width > 0 && height > 0
    blending: true
    fragmentShader: "qrc:/shaders/glassrim.frag.qsb"

    Behavior on strength {
        enabled: !Theme.reduceMotion
        NumberAnimation { duration: 190; easing.type: Easing.OutCubic }
    }
}
