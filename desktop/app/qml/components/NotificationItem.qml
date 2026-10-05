// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// One phone notification in the feed: app, title and text, its actions,
// an inline reply when the app offers one, and a dismiss button on hover.
Item {
    id: item
    required property string deviceId
    required property string key
    required property string appName
    required property string title
    required property string text
    required property string sub
    required property real when
    required property string iconUrl
    required property string actions
    required property string replyAction
    required property string replyLabel

    readonly property var buttons: { try { return JSON.parse(actions) } catch (e) { return [] } }
    property bool replying: false

    implicitHeight: content.height + 24

    function timeText() {
        const minutes = Math.round((Date.now() - when) / 60000)
        if (minutes < 1) return qsTr("now")
        if (minutes < 60) return qsTr("%n min", "", minutes)
        const date = new Date(when)
        if (Date.now() - when < 24 * 3600 * 1000) return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    function send() {
        if (input.text.trim().length === 0)
            return
        NotificationList.reply(item.deviceId, item.key, item.replyAction, input.text)
        input.text = ""
        item.replying = false
    }

    HoverHandler { id: hover }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: hover.hovered ? (Theme.graphite ? Theme.surfaceContainer : Theme.surfaceContainerHigh) : "transparent"
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
    }

    // App icon, or its initial when the phone didn't send one.
    Item {
        id: icon
        x: 12; y: 12
        width: 32; height: 32
        Image {
            id: image
            anchors.fill: parent
            source: item.iconUrl
            sourceSize: Qt.size(64, 64)
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
        anchors.rightMargin: 44
        y: 12
        spacing: 4

        Txt {
            width: parent.width
            role: "caption"
            muted: true
            text: [item.appName, item.sub, item.timeText()].filter(s => s.length > 0).join(" · ")
        }
        Txt {
            width: parent.width
            visible: item.title.length > 0
            role: "title"
            size: 14
            text: item.title
        }
        Txt {
            width: parent.width
            visible: item.text.length > 0
            role: "body"
            text: item.text
            wrapMode: Text.Wrap
            maximumLineCount: 4
        }

        Flow {
            width: parent.width
            spacing: 6
            topPadding: 4
            visible: !item.replying && (item.replyAction.length > 0 || item.buttons.length > 0)
            Button {
                visible: item.replyAction.length > 0
                variant: "tonal"
                size: "sm"
                text: item.replyLabel.length > 0 ? item.replyLabel : qsTr("Reply")
                onClicked: { item.replying = true; input.forceActiveFocus() }
            }
            Repeater {
                model: item.buttons
                delegate: Button {
                    required property var modelData
                    variant: "text"
                    size: "sm"
                    text: modelData.title
                    onClicked: NotificationList.runAction(item.deviceId, item.key, modelData.id)
                }
            }
        }

        // Inline reply.
        Rectangle {
            width: parent.width
            height: 40
            visible: item.replying
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: input.activeFocus ? 2 : 1
            border.color: input.activeFocus ? Theme.primary : Theme.outlineVariant

            TextInput {
                id: input
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.right: sendButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.fontUi
                font.pixelSize: 14
                color: Theme.surfaceContent
                selectionColor: Theme.primaryContainer
                selectedTextColor: Theme.primaryContainerContent
                clip: true
                Keys.onReturnPressed: item.send()
                Keys.onEnterPressed: item.send()
                Keys.onEscapePressed: { text = ""; item.replying = false }
                Accessible.name: qsTr("Reply to %1").arg(item.title)

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: input.text.length === 0
                    role: "body"
                    muted: true
                    text: qsTr("Reply to %1").arg(item.title.length > 0 ? item.title : item.appName)
                }
            }
            IconButton {
                id: sendButton
                anchors.right: parent.right
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.send
                label: qsTr("Send")
                enabled: input.text.trim().length > 0
                onClicked: item.send()
            }
        }
    }

    IconButton {
        anchors.right: parent.right
        anchors.rightMargin: 8
        y: 8
        iconPath: Icons.close
        label: qsTr("Dismiss")
        // Always there (for keyboards and screen readers), quiet until hovered.
        opacity: hover.hovered || activeFocus ? 1 : 0.55
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        onClicked: NotificationList.dismiss(item.deviceId, item.key)
    }
}
