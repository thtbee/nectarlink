// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

Rectangle {
    id: action
    property string title
    property string subtitle
    property string iconPath
    signal clicked

    radius: Theme.radiusLg
    color: hover.hovered ? Qt.darker(Theme.surfaceContainerHigh, 1.03) : Theme.surfaceContainerHigh
    implicitHeight: 104
    scale: tap.pressed ? 0.97 : 1.0
    transform: Translate { y: hover.hovered && !tap.pressed ? -2 : 0; Behavior on y { SpringAnimation { spring: 6; damping: 0.4 } } }

    Behavior on scale { SpringAnimation { spring: 6.2; damping: 0.34 } }
    Behavior on color { ColorAnimation { duration: 160 } }

    Column {
        anchors.fill: parent
        anchors.margins: 14
        spacing: 12
        Rectangle {
            width: 36; height: 36; radius: Theme.radiusMd
            color: Theme.secondaryContainer
            Icon { anchors.centerIn: parent; path: action.iconPath; color: Theme.secondaryContainerContent }
        }
        Column {
            spacing: 2
            Text { text: action.title; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; color: Theme.surfaceContent }
            Text { text: action.subtitle; font.family: Theme.font; font.pixelSize: 12; color: Theme.surfaceContentVariant }
        }
    }
    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { id: tap; onTapped: action.clicked() }
}
