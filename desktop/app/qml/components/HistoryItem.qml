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
    property bool canOpenApp: false
    signal openAppRequested

    implicitHeight: Math.max(icon.height, content.height) + 20
    activeFocusOnTab: item.canOpenApp
    Accessible.role: item.canOpenApp ? Accessible.Button : Accessible.Grouping
    Accessible.name: [item.appName, item.title, item.text].filter(s => s.length > 0).join(", ")
    Accessible.description: item.canOpenApp ? qsTr("Open %1 in an app window").arg(item.appName) : ""
    Accessible.onPressAction: if (item.canOpenApp) item.openAppRequested()
    Keys.onPressed: (event) => {
        if (item.canOpenApp && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            item.openAppRequested()
            event.accepted = true
        }
    }

    function timeText() {
        const date = new Date(when)
        const today = new Date().toDateString() === date.toDateString()
        return today ? date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
                     : qsTr("Yesterday %1").arg(date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat))
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: Theme.surfaceContent
        opacity: item.canOpenApp && historyHover.hovered ? 0.045 : 0
        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: Theme.fadeFast }
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: "transparent"
        border.width: Theme.focusVisible(item) ? 2 : 0
        border.color: Theme.primary
        visible: Theme.focusVisible(item)
    }

    HoverHandler {
        id: historyHover
        cursorShape: item.canOpenApp ? Qt.PointingHandCursor : Qt.ArrowCursor
    }

    TapHandler {
        enabled: item.canOpenApp
        onTapped: item.openAppRequested()
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
