// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A navigation rail destination: an icon in a pill indicator above a label.
FocusScope {
    id: item
    property string iconPath
    property string text
    property bool selected: false
    signal clicked

    implicitWidth: 72
    implicitHeight: 56
    activeFocusOnTab: true

    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: selected
    Accessible.onPressAction: item.clicked()
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            item.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        id: pill
        anchors.horizontalCenter: parent.horizontalCenter
        y: 2
        width: 52; height: 30
        radius: Theme.pill(height)
        color: item.selected ? Theme.indicator : (hover.hovered ? Theme.surfaceContainerHigh : "transparent")
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
        scale: tap.pressed ? Theme.pressScale : 1
        Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        Icon {
            anchors.centerIn: parent
            path: item.iconPath
            color: item.selected ? Theme.indicatorContent : Theme.surfaceContentVariant
        }
    }
    Txt {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: pill.bottom
        anchors.topMargin: 4
        text: item.text
        role: "caption"
        weight: Theme.graphite ? 400 : 600
        color: item.selected ? Theme.surfaceContent : Theme.surfaceContentVariant
    }
    Rectangle {
        anchors.fill: pill
        anchors.margins: -3
        radius: pill.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(item)
    }
    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { id: tap; onTapped: item.clicked() }
}
