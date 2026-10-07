// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// Click-through, always-on-top laser pointer dot controlled by a paired phone
// in presentation mode (docs/protocol/remote.md).
Window {
    id: overlay

    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
        | Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus
    color: "transparent"
    width: 56
    height: 56
    visible: dotRoot.opacity > 0.01

    x: Math.round(Screen.virtualX + AppController.laserX * Math.max(1, Screen.width) - width / 2)
    y: Math.round(Screen.virtualY + AppController.laserY * Math.max(1, Screen.height) - height / 2)

    Item {
        id: dotRoot
        anchors.fill: parent
        opacity: AppController.laserActive ? 1.0 : 0.0
        Behavior on opacity {
            NumberAnimation {
                duration: Theme.reduceMotion ? 0 : 160
                easing.type: Easing.OutCubic
            }
        }

        Rectangle {
            anchors.centerIn: parent
            width: 48
            height: 48
            radius: 24
            color: Theme.primary
            opacity: 0.28
        }

        Rectangle {
            anchors.centerIn: parent
            width: 28
            height: 28
            radius: 14
            color: Theme.primary
            opacity: 0.58
        }

        Rectangle {
            anchors.centerIn: parent
            width: 14
            height: 14
            radius: 7
            color: Theme.primary
            border.width: 2
            border.color: "#FFFFFF"
        }
    }
}
