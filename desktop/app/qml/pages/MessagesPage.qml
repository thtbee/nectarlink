// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Messages: the phone's conversations on the left, the open one on the
// right, and a box to text from the PC (sent through the phone).
Item {
    id: page
    property bool active: true
    // The phone shown (the one on Home).
    property string deviceId
    property string deviceName
    // Writing to someone new (no conversation open).
    property bool composing: false

    readonly property var threads: { try { return JSON.parse(Messages.threads) } catch (e) { return [] } }
    readonly property var messages: { try { return JSON.parse(Messages.messages) } catch (e) { return [] } }
    readonly property var openThread: threads.find(t => t.id === Messages.thread) || null
    readonly property bool ready: Messages.status === "ready"

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    function load() {
        if (active && deviceId.length > 0)
            Messages.open(deviceId)
    }
    onActiveChanged: load()
    onDeviceIdChanged: { composing = false; load() }

    // "14:05", "Tue", or a date.
    function shortTime(ms) {
        const date = new Date(ms)
        const now = new Date()
        if (date.toDateString() === now.toDateString())
            return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        if (now - date < 6 * 24 * 3600 * 1000)
            return date.toLocaleDateString(Qt.locale(), "ddd")
        return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    // ---- No phone, or messages not available ----
    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.contentPadding * 2, 420)
        spacing: 12
        visible: !list.visible && (page.deviceId.length === 0 || Messages.status !== "loading")
        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 40; height: 40
            stroke: 1.5
            path: Icons.messages
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: page.deviceId.length === 0 ? qsTr("Text from your PC")
                : Messages.status === "offline" ? qsTr("%1 isn't connected").arg(page.deviceName)
                : Messages.status === "off" ? qsTr("Messages are off for %1").arg(page.deviceName)
                : Messages.status === "unsupported" ? qsTr("Allow texting on %1").arg(page.deviceName)
                : qsTr("Couldn't load messages")
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: page.deviceId.length === 0 ? qsTr("Pair your phone to read and send texts here.")
                : Messages.status === "offline" ? qsTr("Your conversations show up here when it's back.")
                : Messages.status === "off" ? qsTr("Turn on Messages for this phone, here in Settings and in the Nectarlink app on the phone.")
                : Messages.status === "unsupported" ? qsTr("In the Nectarlink app on your phone, tap Allow on “Text from your PC”.")
                : qsTr("Something went wrong reading them from the phone.")
        }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.deviceId.length > 0 && Messages.status === "failed"
            variant: "tonal"
            text: qsTr("Try again")
            onClicked: page.load()
        }
    }

    Spinner {
        anchors.centerIn: parent
        visible: Messages.status === "loading" && page.threads.length === 0
    }

    // ---- Conversations ----
    Item {
        id: list
        visible: page.threads.length > 0 || (page.ready && page.deviceId.length > 0)
        width: Math.min(340, Math.max(260, page.width * 0.34))
        height: parent.height

        Item {
            id: listHeader
            width: parent.width
            height: 56
            Txt {
                anchors.left: parent.left
                anchors.leftMargin: 20
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Conversations")
                role: "title"
            }
            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.plus
                label: qsTr("New message")
                tonal: page.composing
                enabled: page.ready
                onClicked: {
                    Messages.closeThread()
                    page.composing = true
                    toField.forceActiveFocus()
                }
            }
        }

        ListView {
            id: threadList
            anchors.top: listHeader.bottom
            anchors.bottom: parent.bottom
            width: parent.width
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: page.threads
            delegate: ThreadRow {
                width: threadList.width
                thread: modelData
            }
            Txt {
                anchors.centerIn: parent
                visible: page.ready && page.threads.length === 0
                role: "body"
                muted: true
                text: qsTr("No conversations yet.")
            }
        }
        Divider { anchors.right: parent.right; width: 1; height: parent.height }
    }

    // ---- The open conversation, or a new one ----
    Item {
        id: chat
        anchors.left: list.right
        anchors.right: parent.right
        height: parent.height
        visible: list.visible

        Txt {
            anchors.centerIn: parent
            visible: !page.openThread && !page.composing && list.visible
            role: "body"
            muted: true
            text: qsTr("Choose a conversation, or start a new one.")
        }

        Item {
            id: chatHeader
            visible: !!page.openThread || page.composing
            width: parent.width
            height: 64
            Column {
                anchors.left: parent.left
                anchors.leftMargin: 24
                anchors.right: parent.right
                anchors.rightMargin: 24
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                visible: !page.composing
                Txt {
                    width: parent.width
                    text: page.openThread ? page.openThread.title : ""
                    role: "title"
                    elide: Text.ElideRight
                }
                Txt {
                    width: parent.width
                    visible: page.openThread && page.openThread.title !== page.openThread.addresses
                    text: page.openThread ? page.openThread.addresses : ""
                    role: "bodySmall"
                    muted: true
                    elide: Text.ElideRight
                }
            }
            // New message: who to.
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 24
                anchors.right: parent.right
                anchors.rightMargin: 24
                anchors.verticalCenter: parent.verticalCenter
                spacing: 12
                visible: page.composing
                Txt { anchors.verticalCenter: parent.verticalCenter; text: qsTr("To"); role: "title" }
                TextInput {
                    id: toField
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - 40
                    font.family: Theme.fontUi
                    font.pixelSize: 15
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    inputMethodHints: Qt.ImhDialableCharactersOnly
                    clip: true
                    Accessible.name: qsTr("Phone number")
                    Keys.onReturnPressed: composer.focusText()
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: toField.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Phone number")
                    }
                }
            }
            Divider { anchors.bottom: parent.bottom; width: parent.width }
        }

        ListView {
            id: messageList
            anchors.top: chatHeader.bottom
            anchors.bottom: composer.top
            anchors.bottomMargin: 8
            width: parent.width
            visible: !!page.openThread
            clip: true
            spacing: 6
            // A short conversation sits at the bottom, by the box.
            topMargin: Math.max(16, height - contentHeight - bottomMargin)
            bottomMargin: 8
            boundsBehavior: Flickable.StopAtBounds
            model: page.messages
            // The latest at the bottom; scrolling up loads older ones.
            property int lastCount: 0
            onCountChanged: {
                const grewAtTop = count > lastCount && lastCount > 0 && Messages.loadingOlder === false && atYBeginning
                if (!grewAtTop)
                    Qt.callLater(positionViewAtEnd)
                lastCount = count
            }
            onAtYBeginningChanged: if (atYBeginning && Messages.more && count > 0) Messages.loadOlder()
            header: Item {
                width: messageList.width
                height: Messages.loadingOlder ? 40 : 0
                Spinner { anchors.centerIn: parent; visible: Messages.loadingOlder }
            }
            delegate: Bubble {
                width: messageList.width
                message: modelData
            }
            Txt {
                anchors.centerIn: parent
                width: Math.min(parent.width - 48, 420)
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                visible: page.openThread && page.messages.length === 0 && page.ready
                role: "body"
                muted: true
                text: qsTr("No messages to show. Android keeps texts with one-time codes from other apps for a few hours.")
            }
        }

        // Writing a text.
        Rectangle {
            id: composer
            visible: !!page.openThread || page.composing
            readonly property bool group: !!page.openThread && page.openThread.group
            readonly property bool canSend: page.ready && !Messages.sending && !group
                && input.text.trim().length > 0 && (!page.composing || toField.text.trim().length > 0)
            function focusText() { input.forceActiveFocus() }
            function send() {
                if (!canSend)
                    return
                if (page.composing)
                    Messages.sendTo(toField.text, input.text)
                else
                    Messages.send(input.text)
                input.text = ""
                page.composing = false
                toField.text = ""
            }
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 16
            height: Math.min(Math.max(input.contentHeight + 22, 48), 160)
            radius: Theme.graphite ? Theme.radiusSm : 24
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: input.activeFocus ? 2 : 1
            border.color: input.activeFocus ? Theme.primary : Theme.outlineVariant

            Flickable {
                id: inputScroll
                anchors.left: parent.left
                anchors.leftMargin: 18
                anchors.right: sendButton.left
                anchors.rightMargin: 8
                anchors.top: parent.top
                anchors.topMargin: 11
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 11
                contentHeight: input.contentHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                TextEdit {
                    id: input
                    width: inputScroll.width
                    wrapMode: TextEdit.Wrap
                    readOnly: composer.group
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    Accessible.name: qsTr("Text message")
                    // Enter sends; Shift+Enter starts a new line.
                    Keys.onPressed: (event) => {
                        if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && !(event.modifiers & Qt.ShiftModifier)) {
                            composer.send()
                            event.accepted = true
                        }
                    }
                    onCursorRectangleChanged: {
                        if (cursorRectangle.y < inputScroll.contentY)
                            inputScroll.contentY = cursorRectangle.y
                        else if (cursorRectangle.y + cursorRectangle.height > inputScroll.contentY + inputScroll.height)
                            inputScroll.contentY = cursorRectangle.y + cursorRectangle.height - inputScroll.height
                    }
                    Txt {
                        visible: input.text.length === 0
                        role: "body"
                        muted: true
                        text: composer.group ? qsTr("Group texts can't be sent from the PC yet")
                            : qsTr("Text message")
                    }
                }
            }
            IconButton {
                id: sendButton
                anchors.right: parent.right
                anchors.rightMargin: 6
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                iconPath: Icons.send
                label: qsTr("Send")
                tonal: composer.canSend
                enabled: composer.canSend
                onClicked: composer.send()
            }
            Spinner {
                anchors.centerIn: sendButton
                visible: Messages.sending
            }
        }
    }

    // A conversation in the list.
    component ThreadRow: Item {
        id: row
        property var thread
        readonly property bool selected: thread.id === Messages.thread
        height: 72
        Accessible.role: Accessible.ListItem
        Accessible.name: thread.title + ". " + thread.snippet
        Accessible.onPressAction: open()
        function open() {
            page.composing = false
            Messages.openThread(thread.id)
        }

        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            radius: Theme.radiusLg
            color: row.selected ? Theme.secondaryContainer
                 : rowHover.hovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                 : "transparent"
            Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
        }
        Item {
            id: face
            x: 20
            anchors.verticalCenter: parent.verticalCenter
            width: 40; height: 40
            RoundedImage {
                anchors.fill: parent
                radius: width / 2
                visible: row.thread.photo.length > 0 && status === Image.Ready
                source: row.thread.photo
                sourceSize: Qt.size(80, 80)
                fillMode: Image.PreserveAspectCrop
            }
            Avatar {
                anchors.fill: parent
                visible: row.thread.photo.length === 0
                // A contact's initial; a number (no letter to show) or a group gets an icon.
                readonly property bool named: !/^[\d+(#*]/.test(row.thread.title)
                name: row.thread.group || !named ? "" : row.thread.title
                iconPath: row.thread.group ? Icons.messages : named ? "" : Icons.person
                emphasized: row.thread.unread > 0
            }
        }
        Column {
            anchors.left: face.right
            anchors.leftMargin: 12
            anchors.right: parent.right
            anchors.rightMargin: 20
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Item {
                width: parent.width
                height: name.height
                Txt {
                    id: name
                    anchors.left: parent.left
                    anchors.right: when.left
                    anchors.rightMargin: 8
                    text: row.thread.title
                    role: "body"
                    weight: row.thread.unread > 0 ? 650 : 500
                    elide: Text.ElideRight
                }
                Txt {
                    id: when
                    anchors.right: parent.right
                    anchors.baseline: name.baseline
                    text: page.shortTime(row.thread.date)
                    role: "bodySmall"
                    color: row.thread.unread > 0 ? Theme.primary : Theme.surfaceContentVariant
                }
            }
            Item {
                width: parent.width
                height: snippet.height
                Txt {
                    id: snippet
                    anchors.left: parent.left
                    anchors.right: badge.visible ? badge.left : parent.right
                    anchors.rightMargin: 8
                    text: row.thread.snippet
                    role: "bodySmall"
                    muted: row.thread.unread === 0
                    maximumLineCount: 1
                    elide: Text.ElideRight
                }
                Rectangle {
                    id: badge
                    anchors.right: parent.right
                    anchors.verticalCenter: snippet.verticalCenter
                    visible: row.thread.unread > 0
                    width: Math.max(20, count.implicitWidth + 10)
                    height: 20
                    radius: 10
                    color: Theme.primary
                    Txt {
                        id: count
                        anchors.centerIn: parent
                        text: row.thread.unread
                        role: "label"
                        size: 11
                        color: Theme.primaryContent
                    }
                }
            }
        }
        HoverHandler { id: rowHover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: row.open() }
    }

    // A message: right-aligned when sent from the phone.
    component Bubble: Item {
        id: bubble
        property var message
        readonly property bool mine: message.outgoing
        readonly property real maxWidth: Math.min(width * 0.72, 520)
        height: content.height + meta.height + 4

        Rectangle {
            id: content
            anchors.right: bubble.mine ? parent.right : undefined
            anchors.left: bubble.mine ? undefined : parent.left
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            width: Math.max(pictures.width, Math.min(text.implicitWidth, bubble.maxWidth - 28) + 28)
            height: pictures.height + (text.visible ? text.height + 18 : 0)
            radius: 18
            color: bubble.mine ? Theme.primaryContainer : Theme.surfaceContainerHigh
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.outlineVariant
            Column {
                id: pictures
                width: bubble.message.images.length > 0 ? Math.min(260, bubble.maxWidth) : 0
                Repeater {
                    model: bubble.message.images
                    delegate: Item {
                        required property string modelData
                        width: pictures.width
                        height: modelData.length > 0 && picture.status === Image.Ready
                            ? Math.min(320, width * picture.implicitImageHeight / Math.max(1, picture.implicitImageWidth))
                            : 140
                        RoundedImage {
                            id: picture
                            anchors.fill: parent
                            radius: 18
                            source: parent.modelData
                            fillMode: Image.PreserveAspectCrop
                            sourceSize.width: 520
                        }
                        Spinner { anchors.centerIn: parent; visible: parent.modelData.length === 0 }
                        TapHandler { onTapped: if (parent.modelData.length > 0) Qt.openUrlExternally(parent.modelData) }
                        HoverHandler { cursorShape: parent.modelData.length > 0 ? Qt.PointingHandCursor : Qt.ArrowCursor }
                    }
                }
            }
            TextEdit {
                id: text
                anchors.top: pictures.bottom
                anchors.topMargin: 9
                x: 14
                width: Math.min(implicitWidth, bubble.maxWidth - 28)
                visible: bubble.message.body.length > 0 || bubble.message.attachments > 0
                text: bubble.message.body.length > 0 ? bubble.message.body
                    : qsTr("Attachment (open it on the phone)")
                readOnly: true
                selectByMouse: true
                wrapMode: TextEdit.Wrap
                textFormat: TextEdit.PlainText
                font.family: Theme.fontUi
                font.pixelSize: 14
                font.italic: bubble.message.body.length === 0
                color: bubble.mine ? Theme.primaryContainerContent : Theme.surfaceContent
                selectionColor: Theme.primary
                selectedTextColor: Theme.primaryContent
            }
        }
        Txt {
            id: meta
            anchors.top: content.bottom
            anchors.topMargin: 3
            anchors.right: bubble.mine ? content.right : undefined
            anchors.left: bubble.mine ? undefined : content.left
            anchors.leftMargin: 6
            anchors.rightMargin: 6
            role: "bodySmall"
            size: 11
            color: bubble.message.status === "failed" ? Theme.error : Theme.surfaceContentVariant
            text: bubble.message.status === "failed" ? qsTr("Not sent")
                : bubble.message.status === "pending" ? qsTr("Sending…")
                : page.shortTime(bubble.message.date)
        }
    }
}
