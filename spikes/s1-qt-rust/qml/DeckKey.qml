// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

Rectangle {
    id: key
    property string label
    property string detail
    property string iconPath
    property bool active: false

    radius: Theme.radiusLg
    color: active ? Theme.primary : Theme.surfaceContainer
    scale: tap.pressed ? 0.94 : 1.0
    Behavior on scale { SpringAnimation { spring: 6.2; damping: 0.34 } }
    Behavior on color { ColorAnimation { duration: 180 } }

    Rectangle {
        x: 11; y: 11; width: 30; height: 30; radius: Theme.radiusSm
        color: key.active ? Qt.rgba(1, 1, 1, 0.18) : Theme.surfaceContainerHighest
        Icon { anchors.centerIn: parent; path: key.iconPath; width: 18; height: 18; color: key.active ? Theme.primaryContent : Theme.surfaceContent }
    }
    Column {
        anchors { left: parent.left; bottom: parent.bottom; margins: 11 }
        Text { text: key.label; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.DemiBold; color: key.active ? Theme.primaryContent : Theme.surfaceContent }
        Text { text: key.detail; font.family: Theme.font; font.pixelSize: 10; opacity: 0.75; color: key.active ? Theme.primaryContent : Theme.surfaceContent }
    }
    TapHandler { id: tap; onTapped: key.active = !key.active }
}
