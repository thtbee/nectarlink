// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// One phone notification in the feed: app, title and text, its actions,
// an inline reply when the app offers one, and dismiss and options buttons
// (what this app's notifications do on this PC).
Item {
    id: item
    required property string deviceId
    required property string key
    required property string app
    required property string appName
    required property string title
    required property string text
    required property string sub
    required property real when
    required property string iconUrl
    required property string actions
    required property string replyAction
    required property string replyLabel
    required property string replies
    // Bumped by the feed every minute, so relative times stay right.
    property int clock: 0

    readonly property var buttons: { try { return JSON.parse(actions) } catch (e) { return [] } }
    readonly property var sent: { try { return JSON.parse(replies) } catch (e) { return [] } }
    property bool replying: false
    signal optionsRequested

    implicitHeight: content.height + 24

    function timeText() {
        clock // re-evaluated each minute
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

    // Hover tracking only: clicks go to the buttons inside.
    MouseArea {
        id: hover
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
        readonly property bool hovered: containsMouse
    }

    // A light wash of the text color, so hovering never looks like selection.
    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: Theme.surfaceContent
        opacity: hover.hovered ? 0.045 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
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
        anchors.rightMargin: 84
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

        // Replies sent from this PC.
        Repeater {
            model: item.sent
            delegate: Row {
                required property var modelData
                width: content.width
                spacing: 6
                opacity: modelData.pending ? 0.55 : 1
                Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
                Icon {
                    width: 14; height: 14
                    anchors.verticalCenter: parent.verticalCenter
                    path: modelData.pending ? Icons.send : Icons.check
                    color: Theme.primary
                }
                Txt {
                    width: parent.width - 20
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.Wrap
                    maximumLineCount: 3
                    text: modelData.pending ? qsTr("Sending: %1").arg(modelData.text) : qsTr("You: %1").arg(modelData.text)
                }
            }
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
        anchors.right: dismiss.left
        y: 8
        iconPath: Icons.more
        label: qsTr("Options for %1").arg(item.appName)
        // Like Dismiss: always there (for keyboards and screen readers),
        // quiet until hovered.
        opacity: hover.hovered || activeFocus ? 1 : 0.55
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        onClicked: item.optionsRequested()
    }
    IconButton {
        id: dismiss
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
