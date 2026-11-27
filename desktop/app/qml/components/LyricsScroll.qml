// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import app.nectarlink

// A small lyric viewport that follows the current line. It keeps the active
// line near the middle and slides the column as the song moves on; past lines
// stay filled, future ones stay muted, and the active line sweeps word by word.
Item {
    id: root
    property var lines: []
    property int current: -1
    property real progress: 0
    property color baseColor: "#FFFFFF"
    property color fillColor: "#FFFFFF"
    property int lineHeight: 22
    property int visibleLines: 5

    implicitHeight: lineHeight * visibleLines
    height: implicitHeight
    clip: true

    readonly property real contentHeight: Math.max(height,
        (lines ? lines.length : 0) * lineHeight)
    readonly property real preferredTop: {
        if (!lines || lines.length === 0 || current < 0)
            return 0
        const centre = (visibleLines - 1) / 2
        return (current - centre) * lineHeight
    }
    readonly property real maxTop: Math.max(0, contentHeight - height)
    readonly property real baseTop: Math.max(0, (height - contentHeight) / 2)

    Item {
        id: content
        width: root.width
        height: root.contentHeight
        y: root.baseTop - Math.max(0, Math.min(root.maxTop, root.preferredTop))
        Behavior on y {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 260; easing.type: Easing.OutCubic }
        }

        Column {
            width: parent.width
            spacing: 0

            Repeater {
                model: root.lines

                delegate: Item {
                    id: lineRow
                    required property int index
                    required property var modelData
                    width: root.width
                    height: root.lineHeight

                    readonly property bool active: index === root.current
                    readonly property bool past: index < root.current

                    KaraokeTxt {
                        anchors.fill: parent
                        text: lineRow.modelData.text
                        progress: lineRow.past ? 1 : (lineRow.active ? root.progress : 0)
                        role: "body"
                        size: lineRow.active ? 12.5 : 11.5
                        weight: lineRow.active ? Font.DemiBold : Font.Normal
                        baseColor: root.baseColor
                        fillColor: root.fillColor
                        opacity: lineRow.active ? 1 : (lineRow.past ? 0.92 : 0.46)
                        Behavior on opacity {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { duration: 160 }
                        }
                    }
                }
            }
        }
    }
}
