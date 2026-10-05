// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A round device or app badge: an icon, or the first letter of a name.
Rectangle {
    id: avatar
    property string name
    property string iconPath
    property bool emphasized: false

    implicitWidth: 36
    implicitHeight: 36
    radius: Theme.graphite ? Theme.radiusSm : width / 2
    color: Theme.graphite ? "transparent" : (emphasized ? Theme.primaryContainer : Theme.surfaceContainerHighest)
    border.width: Theme.graphite ? 1 : 0
    border.color: Theme.outlineVariant

    readonly property color ink: Theme.graphite ? Theme.surfaceContent
                                                : (emphasized ? Theme.primaryContainerContent : Theme.surfaceContentVariant)
    Icon {
        visible: avatar.iconPath.length > 0
        anchors.centerIn: parent
        width: avatar.width * 0.5; height: width
        path: avatar.iconPath
        color: avatar.ink
    }
    Txt {
        visible: avatar.iconPath.length === 0
        anchors.centerIn: parent
        text: avatar.name.length > 0 ? avatar.name.charAt(0).toUpperCase() : ""
        role: "label"
        font.weight: 700
        color: avatar.ink
    }
}
