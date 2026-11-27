// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A settings-style row: optional icon, title and description, and a
// trailing control placed in `trailing`.
Item {
    id: row
    property string iconPath
    // A solid icon (a brand mark) instead of a stroke one.
    property bool iconFilled: false
    property string title
    property string description
    property bool stackBelow: false
    // A key from new_settings.rs: a "New" pill by the title while it's unseen (#656).
    property string newKey
    readonly property bool isNew: newKey.length > 0 && Preferences.newSettings.indexOf("\"" + newKey + "\"") >= 0
    default property alias trailing: trailingSlot.data

    readonly property bool stackTrailing: stackBelow || (trailingSlot.childrenRect.width > row.width * 0.52 && row.width > 0)

    implicitHeight: stackTrailing
        ? texts.implicitHeight + trailingSlot.childrenRect.height + 34
        : Math.max(56, texts.implicitHeight + 20, trailingSlot.childrenRect.height + 16)

    Icon {
        id: icon
        visible: row.iconPath.length > 0
        anchors.left: parent.left
        anchors.verticalCenter: row.stackTrailing ? texts.verticalCenter : parent.verticalCenter
        path: row.iconPath
        filled: row.iconFilled
        color: Theme.surfaceContentVariant
    }
    Column {
        id: texts
        anchors.left: icon.visible ? icon.right : parent.left
        anchors.leftMargin: icon.visible ? 14 : 0
        anchors.right: row.stackTrailing ? parent.right : trailingSlot.left
        anchors.rightMargin: row.stackTrailing ? 0 : 16
        y: row.stackTrailing ? 12 : Math.round((row.height - height) / 2)
        spacing: 2
        Item {
            width: parent.width
            height: titleText.height
            Txt {
                id: titleText
                width: Math.min(implicitWidth, parent.width - (newPill.visible ? newPill.width + 8 : 0))
                text: row.title
                role: "title"
                size: 14
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
            }
            NewPill {
                id: newPill
                visible: row.isNew
                anchors.left: titleText.right
                anchors.leftMargin: 8
                y: Math.max(0, Math.round((titleText.font.pixelSize * 1.3 - height) / 2))
            }
        }
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
        // Placed by x, not by swapping anchors: a left anchor that a binding
        // turned off could stay on, so a row that stopped stacking stretched
        // across and its control sat over the title.
        x: row.stackTrailing ? texts.x : row.width - width
        y: row.stackTrailing ? (texts.y + texts.implicitHeight + 10) : Math.round((row.height - height) / 2)
        width: childrenRect.width
        height: childrenRect.height
    }
}
