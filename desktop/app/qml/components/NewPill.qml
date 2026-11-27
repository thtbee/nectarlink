// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// "New": a setting or Settings section a recent update added (#656).
Rectangle {
    id: pill
    width: label.implicitWidth + 12
    height: 18
    radius: 9
    color: Theme.primaryContainer

    Accessible.role: Accessible.StaticText
    Accessible.name: label.text

    Txt {
        id: label
        anchors.centerIn: parent
        text: qsTr("New")
        role: "label"
        size: 10
        weight: 700
        color: Theme.primaryContainerContent
    }
}
