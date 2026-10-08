// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A settings-style row: optional icon, title and description, and a
// trailing control placed in `trailing`.
Item {
    id: row
    property string iconPath
    property string title
    property string description
    default property alias trailing: trailingSlot.data

    implicitHeight: Math.max(56, texts.implicitHeight + 20, trailingSlot.childrenRect.height + 16)

    Icon {
        id: icon
        visible: row.iconPath.length > 0
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        path: row.iconPath
        color: Theme.surfaceContentVariant
    }
    Column {
        id: texts
        anchors.left: icon.visible ? icon.right : parent.left
        anchors.leftMargin: icon.visible ? 14 : 0
        anchors.right: trailingSlot.left
        anchors.rightMargin: 16
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2
        Txt { width: parent.width; text: row.title; role: "title"; size: 14 }
        Txt {
            visible: row.description.length > 0
            width: parent.width
            text: row.description
            role: "bodySmall"
            muted: true
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
    }
    Item {
        id: trailingSlot
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: childrenRect.width
        height: childrenRect.height
    }
}
