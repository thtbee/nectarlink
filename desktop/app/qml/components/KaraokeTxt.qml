// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import app.nectarlink

// One lyric line with a karaoke sweep: the text to the left of `progress` is
// painted in the fill colour, the rest stays muted. The base text does the
// layout, so both layers elide and wrap identically.
Item {
    id: root
    property string text: ""
    property real progress: 0
    property string role: "body"
    property real size: 11
    property int weight: Font.DemiBold
    property color baseColor: "#FFFFFF"
    property color fillColor: "#FFFFFF"
    property int elideMode: Text.ElideRight
    implicitHeight: base.implicitHeight
    clip: true

    // The sweep follows the visible part of the line, not the hidden overflow
    // of an elided line. Centred lines start away from the left edge, so the
    // mask starts there too; otherwise a short line would never fill.
    readonly property real visibleWidth: Math.min(base.contentWidth, width)
    readonly property real textLeft: Math.max(0, (width - visibleWidth) / 2)
    readonly property real revealed: Math.max(0, Math.min(1, progress)) * visibleWidth

    Txt {
        id: base
        width: root.width
        height: root.height
        text: root.text
        role: root.role
        size: root.size
        weight: root.weight
        color: root.baseColor
        elide: root.elideMode
        horizontalAlignment: Text.AlignHCenter
    }

    Item {
        id: fillClip
        x: root.textLeft
        width: root.revealed
        height: root.height
        clip: true
        visible: root.revealed > 0

        Txt {
            x: -fillClip.x
            width: root.width
            height: root.height
            text: root.text
            role: root.role
            size: root.size
            weight: root.weight
            color: root.fillColor
            elide: root.elideMode
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
