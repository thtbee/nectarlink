// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A round icon-only button with a tooltip-style accessible name.
FocusScope {
    id: button
    property string iconPath
    property string label
    property bool tonal: false
    property color iconColor: tonal ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
    signal clicked

    implicitWidth: 36
    implicitHeight: 36
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.Button
    Accessible.name: label
    Accessible.onPressAction: if (enabled) button.clicked()
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            button.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        id: body
        anchors.fill: parent
        radius: Theme.pill(height)
        color: button.tonal ? Theme.secondaryContainer : "transparent"
        scale: tap.pressed ? Theme.pressScale : 1
        Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.surfaceContent
            opacity: tap.pressed ? 0.12 : (hover.hovered ? 0.07 : 0)
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        }
        Icon {
            anchors.centerIn: parent
            width: 18; height: 18
            path: button.iconPath
            color: button.iconColor
        }
    }
    Rectangle {
        anchors.fill: body
        anchors.margins: -3
        radius: body.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(button)
    }

    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { id: tap; onTapped: button.clicked() }
}
