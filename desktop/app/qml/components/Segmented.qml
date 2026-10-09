// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A segmented choice: `options` is a list of { value, label }.
Rectangle {
    id: segmented
    property var options: []
    property string value
    signal picked(string value)

    implicitHeight: 36
    implicitWidth: row.implicitWidth + 6
    radius: Theme.pill(height)
    color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
    border.width: Theme.graphite ? 1 : 0
    border.color: Theme.outlineVariant

    Accessible.role: Accessible.PageTabList

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 2
        Repeater {
            model: segmented.options
            delegate: FocusScope {
                id: option
                required property var modelData
                readonly property bool selected: modelData.value === segmented.value
                width: label.implicitWidth + 26
                height: segmented.height - 6
                activeFocusOnTab: true
                Accessible.role: Accessible.PageTab
                Accessible.name: modelData.label
                Accessible.selected: selected
                Accessible.onPressAction: segmented.picked(modelData.value)
                Keys.onPressed: (event) => {
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                        segmented.picked(modelData.value)
                        event.accepted = true
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.pill(height)
                    color: option.selected ? (Theme.graphite ? Theme.surfaceContent : Theme.surface) : "transparent"
                    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -3
                        radius: parent.radius + 3
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        visible: Theme.focusVisible(option)
                    }
                }
                Txt {
                    id: label
                    anchors.centerIn: parent
                    text: option.modelData.label
                    role: "label"
                    color: option.selected ? (Theme.graphite ? Theme.surface : Theme.surfaceContent) : Theme.surfaceContentVariant
                }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: segmented.picked(option.modelData.value) }
            }
        }
    }
}
