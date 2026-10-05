// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

Item {
    id: card
    required property string app
    required property string title
    required property string body
    required property string time
    readonly property bool isCode: body.indexOf("code") >= 0
    signal opened(Item source)

    implicitHeight: content.implicitHeight + 22

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: hover.hovered ? Theme.surfaceContainerHigh : "transparent"
        Behavior on color { ColorAnimation { duration: 140 } }
    }

    Row {
        id: content
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 11 }
        spacing: 12
        Rectangle {
            width: 34; height: 34; radius: 17
            color: Theme.surfaceContainerHighest
            Text { anchors.centerIn: parent; text: card.app.charAt(0); font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Bold; color: Theme.surfaceContentVariant }
        }
        Column {
            width: parent.width - 46
            spacing: 2
            Row {
                spacing: 8
                Text { text: card.title; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; color: Theme.surfaceContent }
                Text { text: card.app + " · " + card.time; font.family: Theme.font; font.pixelSize: 12; color: Theme.surfaceContentVariant; anchors.baseline: parent.children[0].baseline }
            }
            Text { width: parent.width; text: card.body; elide: Text.ElideRight; font.family: Theme.font; font.pixelSize: 13; color: Theme.surfaceContentVariant }
            Rectangle {
                visible: card.isCode
                height: visible ? 32 : 0; width: codeRow.width + 18
                radius: 16; color: Theme.primaryContainer
                Row {
                    id: codeRow
                    anchors.centerIn: parent; spacing: 10
                    Text { text: "482 913"; font.family: "Cascadia Mono"; font.pixelSize: 13; font.weight: Font.Bold; font.letterSpacing: 1.5; color: Theme.primaryContainerContent; anchors.verticalCenter: parent.verticalCenter }
                    Rectangle {
                        width: 52; height: 24; radius: 12; color: Theme.primary
                        Text { anchors.centerIn: parent; text: "Copy"; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.DemiBold; color: Theme.primaryContent }
                    }
                }
            }
        }
    }
    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: card.opened(card) }
}
