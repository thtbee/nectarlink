// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A thin scrollbar along the right edge of `flick`, a sibling Flickable,
// ListView or GridView in the same parent. It shows while the view moves or
// the pointer is over its edge, and can be dragged, or clicked to jump there;
// the wheel still scrolls the view underneath.
Item {
    id: rail
    required property Flickable flick
    // Kept off the window's edge, where Windows' resize border takes the mouse.
    property real inset: 6

    readonly property real ratio: flick ? Math.min(1, flick.visibleArea.heightRatio) : 1
    readonly property bool hot: area.containsMouse || area.pressed
    readonly property bool shown: hot || (flick && flick.moving) || recently.running

    z: 2
    x: flick ? flick.x + flick.width - width - inset : 0
    y: flick ? flick.y : 0
    width: 12
    height: flick ? flick.height : 0
    visible: flick !== null && flick.visible && ratio < 0.999

    Timer {
        id: recently
        interval: 900
    }

    Connections {
        target: rail.flick
        function onContentYChanged() { recently.restart() }
    }

    Rectangle {
        id: thumb
        width: rail.hot ? 8 : 5
        radius: width / 2
        x: rail.width - width - 2
        height: Math.max(32, rail.height * rail.ratio)
        y: {
            const free = 1 - rail.ratio
            const at = free > 0 ? rail.flick.visibleArea.yPosition / free : 0
            return (rail.height - height) * Math.max(0, Math.min(1, at))
        }
        color: Theme.surfaceContentVariant
        opacity: rail.shown ? (rail.hot ? 0.7 : 0.45) : 0

        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 180 }
        }
        Behavior on width {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 120 }
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.ArrowCursor
        // Where on the thumb it was taken hold of.
        property real grip: 0

        function scrollTo(py) {
            const track = rail.height - thumb.height
            const at = track > 0 ? Math.max(0, Math.min(1, (py - grip) / track)) : 0
            const f = rail.flick
            f.contentY = f.originY + at * Math.max(0, f.contentHeight - f.height)
        }

        onPressed: (mouse) => {
            const onThumb = mouse.y >= thumb.y && mouse.y <= thumb.y + thumb.height
            grip = onThumb ? mouse.y - thumb.y : thumb.height / 2
            if (!onThumb)
                scrollTo(mouse.y)
        }
        onPositionChanged: (mouse) => {
            if (pressed)
                scrollTo(mouse.y)
        }
    }
}
