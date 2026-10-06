// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A notification from the last day that's no longer on the phone: app,
// when it arrived, title and text. Read-only.
Item {
    id: item
    required property string appName
    required property string title
    required property string text
    required property string sub
    required property real when
    required property string iconUrl

    implicitHeight: Math.max(icon.height, content.height) + 20

    function timeText() {
        const date = new Date(when)
        const today = new Date().toDateString() === date.toDateString()
        return today ? date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
                     : qsTr("Yesterday %1").arg(date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat))
    }

    Item {
        id: icon
        x: 4; y: 10
        width: 28; height: 28
        Image {
            id: image
            anchors.fill: parent
            source: item.iconUrl
            sourceSize: Qt.size(56, 56)
            smooth: true
            mipmap: true
            visible: status === Image.Ready
        }
        Avatar {
            anchors.fill: parent
            visible: image.status !== Image.Ready
            name: item.appName
        }
    }

    Column {
        id: content
        anchors.left: icon.right
        anchors.leftMargin: 12
        anchors.right: parent.right
        anchors.rightMargin: 4
        y: 10
        spacing: 2
        Txt {
            width: parent.width
            role: "caption"
            muted: true
            text: [item.appName, item.sub, item.timeText()].filter(s => s.length > 0).join(" · ")
            elide: Text.ElideRight
        }
        Txt {
            width: parent.width
            visible: item.title.length > 0
            role: "title"
            size: 14
            text: item.title
            elide: Text.ElideRight
        }
        Txt {
            width: parent.width
            visible: item.text.length > 0
            role: "bodySmall"
            text: item.text
            wrapMode: Text.Wrap
            maximumLineCount: 3
            elide: Text.ElideRight
        }
    }
}
