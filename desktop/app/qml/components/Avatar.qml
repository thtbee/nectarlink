// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A round device or app badge: an icon, or the first letter of a name.
Rectangle {
    id: avatar
    property string name
    property string iconPath
    property bool emphasized: false
    // A color of its own (a contact's), in Bloom; Graphite stays ink on paper.
    property real hue: -1
    readonly property bool tinted: hue >= 0 && !Theme.graphite

    implicitWidth: 36
    implicitHeight: 36
    radius: Theme.graphite ? Theme.radiusSm : width / 2
    color: Theme.graphite ? "transparent"
         : tinted ? Qt.hsla(hue, 0.42, Theme.dark ? 0.28 : 0.86, 1)
         : (emphasized ? Theme.primaryContainer : Theme.surfaceContainerHighest)
    border.width: Theme.graphite ? 1 : 0
    border.color: Theme.outlineVariant

    readonly property color ink: Theme.graphite ? Theme.surfaceContent
                               : tinted ? Qt.hsla(hue, 0.55, Theme.dark ? 0.85 : 0.25, 1)
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
        weight: 700
        color: avatar.ink
    }
}
