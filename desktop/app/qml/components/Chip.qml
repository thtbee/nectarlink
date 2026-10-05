// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A small pill with optional icon. `selected` uses the indicator colors.
Rectangle {
    id: chip
    property string text
    property string iconPath
    property bool selected: false

    implicitHeight: 30
    implicitWidth: row.implicitWidth + 24
    radius: Theme.pill(height)
    color: Theme.graphite ? (selected ? Theme.surfaceContent : "transparent")
                          : (selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh)
    border.width: Theme.graphite ? 1 : 0
    border.color: selected ? Theme.surfaceContent : Theme.outlineVariant
    Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

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
}
