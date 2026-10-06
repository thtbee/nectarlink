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
    // Conversations matching the search box.
    readonly property string query: search.text.trim().toLocaleLowerCase()
    readonly property var shownThreads: query.length === 0 ? threads
        : threads.filter(t => (t.title + " " + t.addresses + " " + t.snippet).toLocaleLowerCase().indexOf(query) >= 0)

    // Back to the list: nothing open.
    function closeChat() {
        Messages.closeThread()
        composing = false
        toField.text = ""
    }

    // A stable color for a contact (in Bloom).
    function hueOf(name) {
        let h = 0
        for (let i = 0; i < name.length; i++)
            h = (h * 31 + name.charCodeAt(i)) % 360
        return h / 360
    }

    // "Today", "Yesterday", or the date, for separators.
    function dayLabel(ms) {
        const date = new Date(ms)
        const today = new Date()
        const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
        if (date.toDateString() === today.toDateString()) return qsTr("Today")
        if (date.toDateString() === yesterday.toDateString()) return qsTr("Yesterday")
        return date.toLocaleDateString(Qt.locale(), date.getFullYear() === today.getFullYear() ? "dddd, d MMMM" : Locale.LongFormat)
    }

    // Text as HTML with its web links clickable.
    function linkified(text) {
        const escaped = text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\n/g, "<br>")
        return escaped.replace(/\b(https?:\/\/[^\s<]+[^\s<.,;:!?)\]'"])/g, '<a href="$1">$1</a>')
    }

    // How many texts a message takes (GSM letters: 160 in one, 153 each
    // when split; anything else: 70 and 67).
    function smsCount(text) {
        const gsm = /^[\x0A\x0D\x20-\x7E£¥èéùìòÇØøÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉÄÖÑÜ§¿äöñüà€]*$/.test(text)
        const one = gsm ? 160 : 70
        const each = gsm ? 153 : 67
        return text.length <= one ? 1 : Math.ceil(text.length / each)
    }

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
    // Leaving the page closes the open conversation.
    onActiveChanged: active ? load() : closeChat()

    Shortcut {
        sequence: "Esc"
        enabled: page.active && (!!page.openThread || page.composing)
        onActivated: page.closeChat()
    }
    Shortcut {
        sequence: StandardKey.Find
        enabled: page.active && page.ready
        onActivated: search.forceActiveFocus()
    }
    Shortcut {
        sequence: StandardKey.New
        enabled: page.active && page.ready
        onActivated: page.startNew()
    }
    function startNew() {
        Messages.closeThread()
        composing = true
        toField.forceActiveFocus()
    }
    onDeviceIdChanged: { composing = false; load() }

    // "14:05", "Yesterday", "Tue", or a date.
    function shortTime(ms) {
        const date = new Date(ms)
        const now = new Date()
        if (date.toDateString() === now.toDateString())
            return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
        if (date.toDateString() === yesterday.toDateString())
            return qsTr("Yesterday")
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
                id: newButton
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.plus
                label: qsTr("New message (Ctrl+N)")
                tonal: page.composing
                enabled: page.ready
                onClicked: page.startNew()
            }
            IconButton {
                anchors.right: newButton.left
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: Messages.status !== "loading"
                onClicked: Messages.refresh()
            }
        }

        // Search: by name, number or the latest text.
        Rectangle {
            id: searchBox
            anchors.top: listHeader.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            height: 38
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: search.activeFocus ? 2 : (Theme.graphite ? 1 : 0)
            border.color: search.activeFocus ? Theme.primary : Theme.outlineVariant
            Icon {
                id: searchIcon
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 16; height: 16
                path: Icons.search
                color: Theme.surfaceContentVariant
            }
            TextInput {
                id: search
                anchors.left: searchIcon.right
                anchors.leftMargin: 8
                anchors.right: clearSearch.left
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.fontUi
                font.pixelSize: 14
                color: Theme.surfaceContent
                selectionColor: Theme.primaryContainer
                selectedTextColor: Theme.primaryContainerContent
                clip: true
                Accessible.name: qsTr("Search conversations")
                Keys.onEscapePressed: (event) => {
                    event.accepted = text.length > 0
                    text = ""
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: search.text.length === 0
                    role: "body"
                    muted: true
                    text: qsTr("Search")
                }
            }
            IconButton {
                id: clearSearch
                anchors.right: parent.right
                anchors.rightMargin: 2
                anchors.verticalCenter: parent.verticalCenter
                width: 32; height: 32
                visible: search.text.length > 0
                iconPath: Icons.close
                label: qsTr("Clear search")
                onClicked: search.text = ""
            }
        }

        ListView {
            id: threadList
            anchors.top: searchBox.bottom
            anchors.topMargin: 8
            anchors.bottom: parent.bottom
            width: parent.width
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: page.shownThreads
            delegate: ThreadRow {
                width: threadList.width
                thread: modelData
            }
            Txt {
                anchors.centerIn: parent
                visible: page.ready && page.shownThreads.length === 0
                role: "body"
                muted: true
                text: page.query.length > 0 ? qsTr("No conversations match.") : qsTr("No conversations yet.")
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
            IconButton {
                id: backButton
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.back
                label: qsTr("Back to conversations (Esc)")
                onClicked: page.closeChat()
            }
            Column {
                anchors.left: backButton.right
                anchors.leftMargin: 8
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
                anchors.left: backButton.right
                anchors.leftMargin: 8
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
                previous: index > 0 ? page.messages[index - 1] : null
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
            // Long texts: how many SMS they take.
            Txt {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.bottom: parent.top
                anchors.bottomMargin: 4
                visible: input.text.length > 120
                role: "bodySmall"
                muted: true
                text: qsTr("%1 characters · %n text(s)", "", page.smsCount(input.text)).arg(input.text.length)
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
                hue: named && !row.thread.group ? page.hueOf(row.thread.title) : -1
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
        property var previous: null
        readonly property bool mine: message.outgoing
        readonly property real maxWidth: Math.min(width * 0.72, 520)
        readonly property bool newDay: !previous || new Date(previous.date).toDateString() !== new Date(message.date).toDateString()
        readonly property bool showSender: !!message.sender && (!previous || previous.sender !== message.sender || newDay)
        height: day.height + sender.height + content.height + meta.height + codeRow.height + 4

        HoverHandler { id: bubbleHover }

        // The day, above its first message.
        Txt {
            id: day
            width: parent.width
            height: bubble.newDay ? implicitHeight + 20 : 0
            visible: bubble.newDay
            topPadding: 8
            horizontalAlignment: Text.AlignHCenter
            role: "label"
            muted: true
            text: page.dayLabel(bubble.message.date)
        }
        // Who wrote it, in a group.
        Txt {
            id: sender
            anchors.top: day.bottom
            x: 30
            height: bubble.showSender ? implicitHeight + 2 : 0
            visible: bubble.showSender
            role: "bodySmall"
            muted: true
            text: bubble.message.sender || ""
        }

        Rectangle {
            id: content
            anchors.top: sender.bottom
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
                text: bubble.message.body.length > 0 ? page.linkified(bubble.message.body)
                    : qsTr("Attachment (open it on the phone)")
                readOnly: true
                selectByMouse: true
                wrapMode: TextEdit.Wrap
                textFormat: bubble.message.body.length > 0 ? TextEdit.RichText : TextEdit.PlainText
                onLinkActivated: (link) => Qt.openUrlExternally(link)
                HoverHandler { cursorShape: text.hoveredLink.length > 0 ? Qt.PointingHandCursor : Qt.IBeamCursor }
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
                // The day is in the separator above.
                : new Date(bubble.message.date).toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        }
        // Copy the text (shown while hovered).
        IconButton {
            anchors.verticalCenter: content.verticalCenter
            anchors.left: bubble.mine ? undefined : content.right
            anchors.right: bubble.mine ? content.left : undefined
            anchors.margins: 4
            width: 32; height: 32
            visible: bubble.message.body.length > 0
            opacity: bubbleHover.hovered || activeFocus ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
            iconPath: Icons.copy
            label: qsTr("Copy text")
            onClicked: Messages.copy(bubble.message.body)
        }
        // A one-time code, one click from the clipboard.
        Item {
            id: codeRow
            anchors.top: meta.bottom
            width: parent.width
            height: bubble.message.code ? codeButton.height + 6 : 0
            visible: !!bubble.message.code
            Button {
                id: codeButton
                x: 24
                y: 4
                variant: "tonal"
                size: "sm"
                iconPath: copied ? Icons.check : Icons.copy
                property bool copied: false
                text: copied ? qsTr("Copied") : qsTr("Copy code %1").arg(bubble.message.code || "")
                onClicked: {
                    Messages.copy(bubble.message.code)
                    copied = true
                    copiedTimer.restart()
                }
                Timer { id: copiedTimer; interval: 2000; onTriggered: codeButton.copied = false }
            }
        }
    }
}
