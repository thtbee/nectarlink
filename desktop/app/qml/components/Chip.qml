// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A small pill with optional icon. `selected` uses the indicator colors.
Rectangle {
    id: chip
    property string text
    property string iconPath
    property bool selected: false
    property bool interactive: false
    signal clicked()

    implicitHeight: 30
    implicitWidth: row.implicitWidth + 24
    radius: Theme.pill(height)
    color: Theme.graphite ? (selected ? Theme.surfaceContent : "transparent")
                          : (selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh)
    border.width: Theme.graphite ? 1 : 0
    border.color: selected ? Theme.surfaceContent : Theme.outlineVariant
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }

    activeFocusOnTab: interactive && visible && enabled
    Accessible.role: interactive ? Accessible.Button : Accessible.StaticText
    Accessible.name: chip.text
    Accessible.onPressAction: if (chip.interactive) chip.clicked()
    Keys.onPressed: (event) => {
        if (chip.interactive && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            chip.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -3
        radius: chip.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: chip.interactive && Theme.focusVisible(chip)
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 6
        Icon {
            visible: chip.iconPath.length > 0
            anchors.verticalCenter: parent.verticalCenter
            width: 15; height: 15
            path: chip.iconPath
            color: label.color
        }
        Txt {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            text: chip.text
            role: "label"
            color: Theme.graphite ? (chip.selected ? Theme.surface : Theme.surfaceContent)
                                  : (chip.selected ? Theme.secondaryContainerContent : Theme.surfaceContent)
        }
    }

    HoverHandler { enabled: chip.interactive; cursorShape: Qt.PointingHandCursor }
    TapHandler { enabled: chip.interactive; onTapped: chip.clicked() }
}
