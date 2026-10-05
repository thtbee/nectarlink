// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Connection indicator: a dot with a soft halo when online (never green:
// the theme's primary carries "connected").
Item {
    property bool online: false
    implicitWidth: 13
    implicitHeight: 13

    Rectangle {
        anchors.centerIn: parent
        width: 13; height: 13; radius: 6.5
        color: Theme.success
        opacity: parent.online ? 0.22 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    }
    Rectangle {
        anchors.centerIn: parent
        width: 7; height: 7; radius: 3.5
        color: parent.online ? Theme.success : Theme.outline
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
    }
}
