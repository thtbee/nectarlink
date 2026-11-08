// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A segmented choice: `options` is a list of { value, label }. A value can be a
// number, as in the app size: `value` holds it as text, so they're compared as text.
Rectangle {
    id: segmented
    property var options: []
    property string value
    signal picked(string value)

    readonly property int selectedIndex: {
        const opts = options || []
        for (let i = 0; i < opts.length; i++) {
            if (String(opts[i].value) === value)
                return i
        }
        return -1
    }
    readonly property Item selectedItem: (selectedIndex >= 0 && selectedIndex < segRepeater.count)
        ? segRepeater.itemAt(selectedIndex) : null

    implicitHeight: Theme.fluent ? 32 : 36
    implicitWidth: row.implicitWidth + 6
    radius: Theme.graphite ? Theme.radiusSm : Theme.pill(height)
    color: Theme.graphite
        ? "transparent"
        : (Theme.fluent ? Theme.controlAltFill
            : (Theme.glass ? Qt.rgba(1, 1, 1, Theme.dark ? 0.08 : 0.44) : Theme.surfaceContainerHigh))
    border.width: Theme.graphite || Theme.glass || Theme.fluent ? 1 : 0
    border.color: Theme.fluent ? Theme.controlStroke : Theme.glass
        ? Qt.rgba(1, 1, 1, Theme.dark ? 0.18 : 0.65)
        : Theme.outlineVariant

    Accessible.role: Accessible.PageTabList

    // Sliding selection pill behind the segments.
    Rectangle {
        id: slideIndicator
        visible: segmented.selectedItem !== null
        x: segmented.selectedItem ? row.x + segmented.selectedItem.x : row.x
        y: 3
        width: segmented.selectedItem ? segmented.selectedItem.width : 0
        height: segmented.height - 6
        radius: Theme.graphite ? Math.max(2, Theme.radiusSm - 2) : Theme.pill(height)
        color: Theme.graphite
            ? Theme.surfaceContent
            : Theme.fluent ? (Theme.dark ? Qt.rgba(1, 1, 1, 0.084) : "#FFFFFF")
            : (Theme.glass
                ? Qt.tint(Qt.rgba(1, 1, 1, Theme.dark ? 0.16 : 0.78), Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.24 : 0.14))
                : (Theme.hive ? Theme.primaryContainer : Theme.surface))
        border.width: Theme.glass || Theme.fluent ? 1 : 0
        border.color: Theme.fluent ? Theme.controlStroke : Qt.rgba(1, 1, 1, Theme.dark ? 0.28 : 0.85)
        // Fluent: an accent pill under the selected label.
        Rectangle {
            visible: Theme.fluent
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 2
            width: 16
            height: 3
            radius: 1.5
            color: Theme.primary
        }
        Behavior on x {
            enabled: !Theme.reduceMotion && slideIndicator.width > 0
            SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy }
        }
        Behavior on width {
            enabled: !Theme.reduceMotion && slideIndicator.width > 0
            SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy }
        }
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 2
        Repeater {
            id: segRepeater
            model: segmented.options
            delegate: FocusScope {
                id: option
                required property var modelData
                required property int index
                readonly property bool selected: String(modelData.value) === segmented.value
                width: label.implicitWidth + 26
                height: segmented.height - 6
                activeFocusOnTab: true
                scale: segMouse.pressed && !Theme.reduceMotion ? Theme.pressScale : 1.0
                Behavior on scale {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 120; easing.type: Easing.OutCubic }
                }
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
                    radius: Theme.graphite ? Math.max(2, Theme.radiusSm - 2) : Theme.pill(height)
                    color: "transparent"
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
                    size: Theme.fluent ? 14 : Theme.typography.label.size
                    weight: Theme.fluent ? (option.selected ? 600 : 400) : Theme.typography.label.weight
                    color: option.selected
                        ? (Theme.graphite ? Theme.surface : (Theme.hive ? Theme.primaryContainerContent : Theme.surfaceContent))
                        : (segMouse.containsMouse ? Theme.surfaceContent : Theme.surfaceContentVariant)
                    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
                }
                MouseArea {
                    id: segMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Theme.controlCursor
                    onClicked: segmented.picked(option.modelData.value)
                }
            }
        }
    }
}
