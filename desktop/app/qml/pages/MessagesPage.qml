// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import QtQuick.Window
import app.nectarlink

// Messages: the phone's SMS and mirrored MessagingStyle chat conversations on
// the left, the open one on the right, and a box to text, attach an MMS image,
// or reply inline from the PC (sent through the phone).
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
    readonly property var contacts: { try { return JSON.parse(PhoneCall.contacts) } catch (e) { return [] } }
    readonly property var availableApps: { try { return JSON.parse(Messages.availableApps) } catch (e) { return [] } }
    readonly property var openThread: threads.find(t => t.id === Messages.thread) || null
    readonly property bool ready: Messages.status === "ready"
    // The lists follow the phone's by changing only what differs, so a list
    // keeps its place (and the pictures its bubbles loaded) when one is read again.
    SyncModel { id: threadModel }
    SyncModel { id: messageModel }
    // What is selected: conversations in the list, or messages in the open one.
    Selection { id: threadSel }
    Selection { id: msgSel }
    readonly property var threadOrder: shownThreads.map(t => t.id)
    readonly property var selectedThreads: threadSel.inOrder(threadOrder)
    readonly property var selectedMessages: msgSel.inOrder(messageOrder)
    // Oldest first, as the messages are read.
    readonly property var messageOrder: messages.map(m => m.id)
    // The open conversation, newest first: the list runs from the bottom up, so it
    // opens at the newest and keeps its place when older messages come in above.
    // Each row knows the message before it, for its day and sender.
    readonly property var messageRows: {
        const out = []
        for (let i = messages.length - 1; i >= 0; i--) {
            const before = i > 0 ? messages[i - 1] : null
            out.push(Object.assign({}, messages[i], {
                hasPrev: before !== null,
                prevDate: before ? before.date : 0,
                prevSender: before ? (before.sender || "") : ""
            }))
        }
        return out
    }
    onShownThreadsChanged: {
        threadModel.sync(shownThreads)
        threadSel.prune(threadOrder)
    }
    onMessageRowsChanged: {
        messageModel.sync(messageRows)
        msgSel.prune(messageOrder)
    }
    // A conversation opens at its newest message, whatever was read before.
    function resetMessages() {
        messageModel.clear()
        messageModel.sync(messageRows)
        messageList.positionViewAtBeginning()
    }
    // Conversations matching the search box.
    readonly property string query: search.text.trim().toLocaleLowerCase()
    readonly property var shownThreads: query.length === 0 ? threads
        : threads.filter(t => (t.title + " " + t.addresses + " " + (t.appName || "") + " " + (t.lastSender || "") + " " + t.snippet).toLocaleLowerCase().indexOf(query) >= 0)

    // Contacts matching the "To" field when starting a new message.
    readonly property string recipientQuery: toField.text.trim().toLocaleLowerCase()
    readonly property var recipientMatches: {
        if (!composing || recipientQuery.length === 0) return []
        const out = []
        for (let i = 0; i < contacts.length && out.length < 6; i++) {
            const c = contacts[i]
            const nums = c.numbers || []
            for (let j = 0; j < nums.length && out.length < 6; j++) {
                const num = nums[j].number || ""
                const label = nums[j].label || ""
                // Don't show a suggestion if its number is already selected verbatim.
                if (num === toField.text.trim()) continue
                if ((c.name + " " + num + " " + label).toLocaleLowerCase().indexOf(recipientQuery) >= 0) {
                    out.push({
                        name: c.name,
                        number: num,
                        label: label,
                        photo: c.photo || ""
                    })
                }
            }
        }
        return out
    }

    FileDialog {
        id: imageFileDialog
        title: qsTr("Attach image or voice note")
        nameFilters: [
            qsTr("Media (*.png *.jpg *.jpeg *.webp *.gif *.bmp *.m4a *.mp3 *.aac *.ogg *.wav *.amr *.3gp)"),
            qsTr("Images (*.png *.jpg *.jpeg *.webp *.gif *.bmp)"),
            qsTr("Audio (*.m4a *.mp3 *.aac *.ogg *.wav *.amr *.3gp)")
        ]
        onAccepted: {
            if (selectedFile.toString().length > 0) {
                Messages.attachFile(selectedFile.toString())
                composer.focusText()
            }
        }
    }

    // Where to save the picture being looked at.
    FileDialog {
        id: saveImageDialog
        title: qsTr("Save picture")
        fileMode: FileDialog.SaveFile
        nameFilters: [qsTr("Pictures (*.jpg *.jpeg *.png *.gif *.webp)"), qsTr("All files (*)")]
        onAccepted: {
            if (selectedFile.toString().length > 0)
                Messages.saveImage(msgImagePreviewSheet.imageUrl, selectedFile.toString())
        }
    }

    // Back to the list: nothing open.
    function closeChat() {
        Messages.closeThread()
        composing = false
        toField.text = ""
    }

    // ---- What can be done with conversations and messages ----
    function markThreadsUnread(ids) {
        Messages.markThreadsUnread(JSON.stringify(ids))
        threadSel.clear()
    }
    // The text of the selected messages, oldest first.
    function textOf(ids) {
        return messages.filter(m => ids.indexOf(m.id) >= 0 && m.body.length > 0).map(m => m.body).join("\n")
    }
    // Deleting asks first.
    property var pendingDelete: ({ kind: "threads", ids: [] })
    function askDelete(kind, ids) {
        pendingDelete = { kind: kind, ids: ids }
        deleteSheet.open()
    }
    // `onPhone`: also delete on the phone, which the user chose for this once; never by default.
    function confirmDelete(onPhone) {
        deleteSheet.close()
        const ids = JSON.stringify(pendingDelete.ids)
        const phone = onPhone === true && deleteSheet.phoneToo
        if (pendingDelete.kind === "threads") {
            Messages.deleteThreads(ids, phone)
            threadSel.clear()
        } else {
            Messages.deleteMessages(ids, phone)
            msgSel.clear()
        }
    }

    // The menus: what applies to the item under the pointer, or to the whole
    // selection when it is part of it.
    property string menuKind: ""
    property var menuTargets: []
    function threadEntries(n, selecting) {
        const out = []
        if (!selecting)
            out.push({ key: "select", label: qsTr("Select"), icon: Icons.checkSquare })
        out.push({ key: "unread", icon: Icons.mail, label: n > 1 ? qsTr("Mark %n as unread", "", n) : qsTr("Mark as unread") })
        out.push({
            key: "delete", icon: Icons.trash, danger: true,
            label: n > 1 ? qsTr("Delete %n conversation(s)", "", n) : qsTr("Delete")
        })
        return out
    }
    function openThreadMenu(source, x, y, thread) {
        const inSelection = threadSel.has(thread.id)
        menuKind = "thread"
        menuTargets = inSelection ? threadSel.inOrder(threadOrder) : [thread.id]
        menu.openAt(source, x, y, threadEntries(menuTargets.length, inSelection))
    }
    function messageEntries(ids, selecting) {
        const n = ids.length
        const out = []
        if (!selecting)
            out.push({ key: "select", label: qsTr("Select"), icon: Icons.checkSquare })
        if (textOf(ids).length > 0)
            out.push({ key: "copy", label: n > 1 ? qsTr("Copy text of %n messages", "", n) : qsTr("Copy text"), icon: Icons.copy })
        out.push({ key: "unread", icon: Icons.mail, label: n > 1 ? qsTr("Mark %n messages as unread", "", n) : qsTr("Mark as unread") })
        out.push({
            key: "delete", icon: Icons.trash, danger: true,
            label: n > 1 ? qsTr("Delete %n messages", "", n) : qsTr("Delete message")
        })
        return out
    }
    function openMessageMenu(source, x, y, message) {
        const inSelection = msgSel.has(message.id)
        menuKind = "message"
        menuTargets = inSelection ? msgSel.inOrder(messageOrder) : [message.id]
        menu.openAt(source, x, y, messageEntries(menuTargets, inSelection))
    }
    // The open conversation's own menu (its "more" button).
    function openConversationMenu(source) {
        if (!page.openThread)
            return
        menuKind = "conversation"
        menuTargets = [page.openThread.id]
        menu.openAt(source, 0, source.height + 4, [
            { key: "select", label: qsTr("Select messages"), icon: Icons.checkSquare },
            { key: "unread", label: qsTr("Mark as unread"), icon: Icons.mail },
            { key: "delete", label: qsTr("Delete"), icon: Icons.trash, danger: true }
        ])
    }
    function menuChosen(key) {
        const ids = menuTargets
        if (menuKind === "thread") {
            if (key === "select") threadSel.toggle(ids[0])
            else if (key === "unread") markThreadsUnread(ids)
            else if (key === "delete") askDelete("threads", ids)
        } else if (menuKind === "message") {
            if (key === "select") msgSel.toggle(ids[0])
            else if (key === "copy") Messages.copy(textOf(ids))
            else if (key === "unread") { msgSel.clear(); Messages.markMessagesUnread(JSON.stringify(ids)) }
            else if (key === "delete") askDelete("messages", ids)
        } else if (menuKind === "conversation") {
            if (key === "select") msgSel.start()
            else if (key === "unread") markThreadsUnread(ids)
            else if (key === "delete") askDelete("threads", ids)
        }
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
        const locale = Qt.locale()
        if (date.getFullYear() !== today.getFullYear())
            return date.toLocaleDateString(locale, Locale.LongFormat)
        const pattern = locale.dateFormat(Locale.LongFormat)
            .replace(/\s*yyyy\s*(?:'年'|年)\s*|[,\s]+yyyy\b|\byyyy[,\s]+/g, "")
        return date.toLocaleDateString(locale, pattern)
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

    visible: active

    function applyComposeTarget() {
        if (Messages.composeTo.length > 0) {
            composing = true
            toField.text = Messages.composeTo
            // A first message that didn't go out comes back to the box.
            const unsentKey = deviceId + "|to:" + Messages.composeTo
            const unsent = Messages.draft(unsentKey)
            if (unsent.length > 0) {
                Messages.saveDraft(unsentKey, "")
                if (input.text.length === 0)
                    input.text = unsent
            }
            Qt.callLater(() => composer.focusText())
        } else if (Messages.thread.length > 0) {
            composing = false
        }
    }

    // The message a clicked notification is about, ringed for a moment.
    property string focusId: ""
    Timer {
        id: focusTimer
        interval: 2600
        onTriggered: page.focusId = ""
    }

    Connections {
        target: Messages
        function onComposeToChanged() { page.applyComposeTarget() }
        function onThreadChanged() {
            page.switchDraft()
            msgSel.clear()
            page.resetMessages()
            if (Messages.thread.length > 0) {
                page.composing = false
                Qt.callLater(() => composer.focusText())
            }
        }
        function onFocusMessageChanged() {
            page.focusId = Messages.focusMessage
            if (page.focusId.length > 0) {
                focusTimer.restart()
                messageList.showFocused()
            }
        }
    }

    function load() {
        if (active && deviceId.length > 0) {
            Messages.open(deviceId)
            PhoneCall.ensureContacts(deviceId)
            applyComposeTarget()
        }
    }
    // Leaving the page closes the open conversation.
    onActiveChanged: {
        if (active) {
            load()
        } else {
            closeChat()
            threadSel.clear()
        }
    }
    // Read only while it can be seen: not with the window closed to the tray or minimized.
    readonly property bool onScreen: active && Window.window !== null && Window.window.visible
        && Window.window.visibility !== Window.Minimized
    onOnScreenChanged: Messages.setOnScreen(onScreen)
    Component.onCompleted: {
        threadModel.sync(shownThreads)
        messageModel.sync(messageRows)
        load()
        switchDraft()
        Messages.setOnScreen(onScreen)
    }
    // What's typed stays for when you come back, even from another page.
    Component.onDestruction: {
        Messages.setOnScreen(false)
        if (draftKey.length > 0)
            Messages.saveDraft(draftKey, input.text)
        Messages.closeThread()
    }

    // Escape lets go of what a menu, a sheet or a selection has, then leaves the conversation.
    readonly property bool escapeFree: !menu.opened && !deleteSheet.opened && !msgImagePreviewSheet.opened
        && !threadSel.active && !msgSel.active
    Shortcut {
        sequence: "Esc"
        enabled: page.active && page.escapeFree && (!!page.openThread || page.composing)
        onActivated: page.closeChat()
    }
    Shortcut {
        sequence: "Esc"
        enabled: page.active && !menu.opened && !deleteSheet.opened && !msgImagePreviewSheet.opened
            && (threadSel.active || msgSel.active)
        onActivated: {
            if (msgSel.active)
                msgSel.clear()
            else
                threadSel.clear()
        }
    }
    Shortcut {
        sequences: [StandardKey.SelectAll]
        enabled: page.active && (threadSel.active || msgSel.active) && !input.activeFocus
            && !search.activeFocus && !toField.activeFocus
        onActivated: {
            if (msgSel.active)
                msgSel.selectAll(page.messageOrder)
            else
                threadSel.selectAll(page.threadOrder)
        }
    }
    Shortcut {
        sequences: [StandardKey.Find]
        enabled: page.active && page.ready
        onActivated: search.forceActiveFocus()
    }
    Shortcut {
        sequences: [StandardKey.New]
        enabled: page.active && page.ready
        onActivated: page.startNew()
    }
    Shortcut {
        sequences: [StandardKey.Paste]
        enabled: page.active && (!!page.openThread || page.composing) && !composer.isChat && !composer.groupSms && !input.activeFocus
        onActivated: {
            if (Messages.pasteClipboardImage()) {
                composer.focusText()
            }
        }
    }
    function startNew() {
        Messages.closeThread()
        composing = true
        toField.text = ""
        toField.forceActiveFocus()
    }
    onDeviceIdChanged: { composing = false; threadSel.clear(); msgSel.clear(); switchDraft(); load() }

    // What was typed in each conversation, so text written for one is never
    // sent to another and comes back when you return to it. Kept by the app,
    // not this page, so changing pages doesn't lose it.
    property string draftKey: ""
    function switchDraft() {
        const key = deviceId + "|" + Messages.thread
        if (key === draftKey)
            return
        if (draftKey.length > 0)
            Messages.saveDraft(draftKey, input.text)
        draftKey = key
        input.text = Messages.draft(key)
    }

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
            text: page.deviceId.length === 0 ? qsTr("Pair your phone to read and send texts.")
                : Messages.status === "offline" ? qsTr("Connect %1 to view messages.").arg(page.deviceName)
                : Messages.status === "off" ? qsTr("Enable Messages in Settings.")
                : Messages.status === "unsupported" ? qsTr("Grant SMS permission on %1.").arg(page.deviceName)
                : qsTr("Unable to load messages.")
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
        visible: Messages.status === "loading" && page.threads.length === 0 && !Messages.hasChatThreads
    }

    // ---- Conversations ----
    Item {
        id: list
        visible: page.threads.length > 0 || Messages.hasChatThreads || (page.ready && page.deviceId.length > 0)
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
                visible: !threadSel.active
                text: qsTr("Conversations")
                role: "title"
            }
            IconButton {
                id: newButton
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                visible: !threadSel.active
                iconPath: Icons.plus
                label: qsTr("New message (Ctrl+N)")
                tonal: page.composing
                enabled: page.ready
                onClicked: page.startNew()
            }
            IconButton {
                id: refreshButton
                anchors.right: newButton.left
                anchors.verticalCenter: parent.verticalCenter
                visible: !threadSel.active
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: Messages.status !== "loading"
                onClicked: Messages.refresh()
            }
            IconButton {
                anchors.right: refreshButton.left
                anchors.verticalCenter: parent.verticalCenter
                visible: !threadSel.active
                objectName: "selectThreads"
                iconPath: Icons.checkSquare
                label: qsTr("Select conversations (or Ctrl+click)")
                enabled: page.shownThreads.length > 0
                onClicked: threadSel.start()
            }
            // Selecting: how many, and what to do with them.
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                visible: threadSel.active
                spacing: 2
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.close
                    label: qsTr("Cancel selection (Esc)")
                    onClicked: threadSel.clear()
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(40, parent.width - 3 * 36 - 6)
                    leftPadding: 6
                    elide: Text.ElideRight
                    role: "body"
                    weight: 500
                    text: qsTr("%n selected", "", threadSel.count)
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    objectName: "unreadSelected"
                    enabled: threadSel.count > 0
                    iconPath: Icons.mail
                    label: qsTr("Mark as unread")
                    onClicked: page.markThreadsUnread(threadSel.inOrder(page.threadOrder))
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    objectName: "deleteSelected"
                    enabled: threadSel.count > 0
                    iconPath: Icons.trash
                    label: qsTr("Delete")
                    onClicked: page.askDelete("threads", threadSel.inOrder(page.threadOrder))
                }
            }
        }

        // Search: by name, number, app, or the latest text.
        Rectangle {
            id: searchBox
            anchors.top: listHeader.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            height: 38
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : (Theme.glass ? Theme.tileColor : Theme.surfaceContainerHighest)
            border.width: search.activeFocus ? 2 : (Theme.graphite || Theme.glass ? 1 : 0)
            border.color: search.activeFocus ? Theme.primary : (Theme.glass ? Theme.cardBorder : Theme.outlineVariant)
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

        // Calm app filter chips when MessagingStyle chat conversations are present.
        Flickable {
            id: appFilterBar
            anchors.top: searchBox.bottom
            anchors.topMargin: visible ? 8 : 0
            anchors.left: parent.left
            anchors.right: parent.right
            height: visible ? 34 : 0
            visible: Messages.hasChatThreads && page.availableApps.length > 2
            contentWidth: filterRow.implicitWidth + 32
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            // A mouse wheel only turns up and down: let it move the chips
            // sideways, so the ones past the edge can be reached.
            WheelHandler {
                target: null
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: (event) => {
                    const delta = event.pixelDelta.x !== 0 ? -event.pixelDelta.x
                        : event.pixelDelta.y !== 0 ? -event.pixelDelta.y
                        : -(event.angleDelta.y !== 0 ? event.angleDelta.y : event.angleDelta.x) / 120 * 60
                    const max = Math.max(0, appFilterBar.contentWidth - appFilterBar.width)
                    appFilterBar.contentX = Math.max(0, Math.min(max, appFilterBar.contentX + delta))
                }
            }

            Row {
                id: filterRow
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                Repeater {
                    model: page.availableApps
                    delegate: Chip {
                        required property var modelData
                        text: modelData.label
                        selected: Messages.appFilter === modelData.key
                        interactive: true
                        onClicked: Messages.filterByApp(modelData.key)
                    }
                }
            }
        }

        ScrollRail { flick: threadList }
        WheelScroll { flick: threadList }
        ListView {
            id: threadList
            objectName: "threadList"
            anchors.top: appFilterBar.bottom
            anchors.topMargin: 8
            anchors.bottom: parent.bottom
            width: parent.width
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: threadModel
            delegate: ThreadRow {
                required property string json
                width: threadList.width
                thread: JSON.parse(json)
            }
            Txt {
                objectName: "emptyThreads"
                anchors.centerIn: parent
                visible: (page.ready || Messages.hasChatThreads) && page.shownThreads.length === 0
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

        // Drag-and-drop image onto an open SMS conversation or new SMS.
        DropArea {
            id: mmsDropArea
            anchors.fill: parent
            enabled: (!!page.openThread || page.composing) && !composer.isChat && !composer.groupSms
            onDropped: (drop) => {
                if (drop.hasUrls && drop.urls.length > 0) {
                    if (Messages.attachFile(drop.urls[0].toString())) {
                        drop.acceptProposedAction()
                        composer.focusText()
                    }
                }
            }
        }

        Rectangle {
            anchors.fill: parent
            anchors.margins: 16
            z: 10
            visible: mmsDropArea.containsDrag
            radius: Theme.radiusLg
            color: Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b, 0.90)
            border.width: 2
            border.color: Theme.primary
            Column {
                anchors.centerIn: parent
                spacing: 8
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 32; height: 32
                    path: Icons.photo
                    color: Theme.primaryContainerContent
                }
                Txt {
                    anchors.horizontalCenter: parent.horizontalCenter
                    role: "title"
                    color: Theme.primaryContainerContent
                    text: qsTr("Drop image to attach (MMS)")
                }
                Txt {
                    anchors.horizontalCenter: parent.horizontalCenter
                    role: "bodySmall"
                    color: Theme.primaryContainerContent
                    text: qsTr("PNG, JPEG, WebP, GIF, or BMP up to 900 KB")
                }
            }
        }

        Txt {
            anchors.centerIn: parent
            visible: !page.openThread && !page.composing && list.visible
            role: "body"
            muted: true
            text: qsTr("Select a conversation")
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
                visible: !msgSel.active
                iconPath: Icons.back
                label: qsTr("Back to conversations (Esc)")
                onClicked: page.closeChat()
            }
            Column {
                anchors.left: backButton.right
                anchors.leftMargin: 8
                anchors.right: headerActions.visible ? headerActions.left : parent.right
                anchors.rightMargin: headerActions.visible ? 8 : 24
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                visible: !page.composing && !msgSel.active
                Row {
                    width: parent.width
                    spacing: 8
                    Txt {
                        id: headerTitleTxt
                        width: Math.min(implicitWidth, parent.width - (headerAppPill.visible ? headerAppPill.width + 8 : 0))
                        text: page.openThread ? page.openThread.title : ""
                        role: "title"
                        elide: Text.ElideRight
                    }
                    Rectangle {
                        id: headerAppPill
                        anchors.verticalCenter: headerTitleTxt.verticalCenter
                        visible: !!page.openThread && !!page.openThread.isChat
                        height: 20
                        width: headerAppPillLabel.implicitWidth + 14
                        radius: 10
                        color: Theme.secondaryContainer
                        Txt {
                            id: headerAppPillLabel
                            anchors.centerIn: parent
                            role: "label"
                            size: 11
                            color: Theme.secondaryContainerContent
                            text: page.openThread && page.openThread.appName ? page.openThread.appName : Messages.currentAppName
                        }
                    }
                }
                Txt {
                    width: parent.width
                    visible: page.openThread && !page.openThread.isChat && page.openThread.title !== page.openThread.addresses
                    text: !page.openThread ? "" : page.openThread.addresses
                    role: "bodySmall"
                    muted: true
                    elide: Text.ElideRight
                }
            }
            Row {
                id: headerActions
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                visible: !page.composing && !!page.openThread && !msgSel.active
                IconButton {
                    iconPath: Icons.checkSquare
                    label: qsTr("Select messages")
                    onClicked: msgSel.start()
                }
                IconButton {
                    id: callThreadButton
                    visible: !!page.openThread && !page.openThread.isChat && !page.openThread.group
                        && page.openThread.addresses.length > 0 && PhoneCall.canDial
                    enabled: !PhoneCall.dialing
                    iconPath: Icons.call
                    label: page.openThread ? qsTr("Call %1").arg(page.openThread.title) : qsTr("Call")
                    onClicked: if (page.openThread) PhoneCall.dial(page.openThread.addresses)
                }
                IconButton {
                    id: moreButton
                    objectName: "moreButton"
                    iconPath: Icons.more
                    label: qsTr("More")
                    onClicked: page.openConversationMenu(moreButton)
                }
            }
            // Selecting messages: how many, and what to do with them.
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                visible: msgSel.active
                spacing: 2
                IconButton {
                    objectName: "cancelMessageSelection"
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.close
                    label: qsTr("Cancel selection (Esc)")
                    onClicked: msgSel.clear()
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(60, parent.width - 4 * 36 - 8)
                    leftPadding: 8
                    elide: Text.ElideRight
                    role: "title"
                    text: qsTr("%n selected", "", msgSel.count)
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    objectName: "copySelectedMessages"
                    enabled: msgSel.count > 0 && page.textOf(msgSel.inOrder(page.messageOrder)).length > 0
                    iconPath: Icons.copy
                    label: qsTr("Copy text")
                    onClicked: {
                        Messages.copy(page.textOf(msgSel.inOrder(page.messageOrder)))
                        msgSel.clear()
                    }
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    objectName: "unreadSelectedMessages"
                    enabled: msgSel.count > 0
                    iconPath: Icons.mail
                    label: msgSel.count > 1 ? qsTr("Mark %n messages as unread", "", msgSel.count) : qsTr("Mark as unread")
                    onClicked: {
                        const ids = msgSel.inOrder(page.messageOrder)
                        msgSel.clear()
                        Messages.markMessagesUnread(JSON.stringify(ids))
                    }
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    objectName: "deleteSelectedMessages"
                    enabled: msgSel.count > 0
                    iconPath: Icons.trash
                    label: qsTr("Delete")
                    onClicked: page.askDelete("messages", msgSel.inOrder(page.messageOrder))
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
                    clip: true
                    Accessible.name: qsTr("Name or phone number")
                    Keys.onReturnPressed: {
                        if (page.recipientMatches.length > 0 && !/\d/.test(toField.text)) {
                            toField.text = page.recipientMatches[0].number
                        }
                        composer.focusText()
                    }
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: toField.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Name or phone number")
                    }
                }
            }
            Divider { anchors.bottom: parent.bottom; width: parent.width }
        }

        // Contact suggestions when typing a name or number in "To".
        Rectangle {
            id: contactSuggestions
            z: 2
            anchors.top: chatHeader.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            anchors.topMargin: 6
            visible: page.composing && page.recipientMatches.length > 0
            height: suggestionColumn.height + 12
            radius: Theme.radiusLg
            color: Theme.surfaceContainerHigh
            border.width: 1
            border.color: Theme.outlineVariant

            Column {
                id: suggestionColumn
                y: 6
                width: parent.width
                Repeater {
                    model: page.recipientMatches
                    delegate: Item {
                        id: srow
                        required property var modelData
                        width: suggestionColumn.width
                        height: 52
                        activeFocusOnTab: true
                        Accessible.role: Accessible.ListItem
                        Accessible.name: modelData.name + ", " + modelData.number
                        Accessible.onPressAction: choose()
                        function choose() {
                            toField.text = modelData.number
                            composer.focusText()
                        }
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                srow.choose()
                                event.accepted = true
                            }
                        }
                        Rectangle {
                            anchors.fill: parent
                            anchors.leftMargin: 6
                            anchors.rightMargin: 6
                            radius: Theme.radiusMd
                            color: sHover.hovered
                                ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.08)
                                : "transparent"
                            border.width: Theme.focusVisible(srow) ? 2 : 0
                            border.color: Theme.primary
                        }
                        Item {
                            id: sFace
                            x: 16
                            anchors.verticalCenter: parent.verticalCenter
                            width: 34; height: 34
                            RoundedImage {
                                anchors.fill: parent
                                radius: width / 2
                                visible: srow.modelData.photo.length > 0 && status === Image.Ready
                                source: srow.modelData.photo
                                sourceSize: Qt.size(68, 68)
                                fillMode: Image.PreserveAspectCrop
                            }
                            Avatar {
                                anchors.fill: parent
                                visible: srow.modelData.photo.length === 0
                                name: srow.modelData.name
                                hue: page.hueOf(srow.modelData.name)
                            }
                        }
                        Column {
                            anchors.left: sFace.right
                            anchors.leftMargin: 12
                            anchors.right: parent.right
                            anchors.rightMargin: 16
                            anchors.verticalCenter: parent.verticalCenter
                            Txt {
                                width: parent.width
                                text: srow.modelData.name
                                role: "body"
                                weight: 500
                                elide: Text.ElideRight
                            }
                            Txt {
                                width: parent.width
                                text: srow.modelData.label.length > 0
                                    ? srow.modelData.label + " · " + srow.modelData.number
                                    : srow.modelData.number
                                role: "bodySmall"
                                muted: true
                                elide: Text.ElideRight
                            }
                        }
                        HoverHandler { id: sHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: srow.choose() }
                    }
                }
            }
        }

        ScrollRail { flick: messageList }
        WheelScroll {
            flick: messageList
            onMoved: page.focusId = ""
        }
        ListView {
            id: messageList
            objectName: "messageList"
            anchors.top: chatHeader.bottom
            anchors.bottom: attachmentBar.visible ? attachmentBar.top : composer.top
            anchors.bottomMargin: 8
            width: parent.width
            visible: !!page.openThread
            clip: true
            spacing: 6
            cacheBuffer: 960
            // Newest at the bottom, listed first: a short conversation sits by the box,
            // a new message shows when the newest is, and older ones loaded above don't
            // move what's in view. Nothing has to scroll to the end, or keep a place.
            verticalLayoutDirection: ListView.BottomToTop
            boundsBehavior: Flickable.StopAtBounds
            model: messageModel
            function showFocused() {
                const index = messageModel.indexOfId(page.focusId)
                if (index >= 0)
                    positionViewAtIndex(index, ListView.Contain)
            }
            // Near the top, by scrolling or because the messages loaded don't fill the
            // view (some of them were deleted here).
            function loadOlderIfNeeded(short) {
                if (!Messages.more || Messages.loadingOlder || count === 0)
                    return
                if (!short && contentY - originY > 240)
                    return
                Messages.loadOlder()
            }
            function fill() {
                if (contentHeight < height)
                    loadOlderIfNeeded(true)
            }
            onContentYChanged: loadOlderIfNeeded(false)
            onCountChanged: Qt.callLater(fill)
            onMovementStarted: page.focusId = ""
            header: Item { width: messageList.width; height: 8 }
            // At the top: older messages loading.
            footer: Item {
                width: messageList.width
                height: Messages.loadingOlder ? 56 : 16
                Spinner { anchors.centerIn: parent; visible: Messages.loadingOlder }
            }
            Connections {
                target: Messages
                function onLoadingOlderChanged() { Qt.callLater(messageList.fill) }
            }
            delegate: Bubble {
                required property string json
                width: messageList.width
                message: JSON.parse(json)
            }
            Txt {
                anchors.centerIn: parent
                width: Math.min(parent.width - 48, 420)
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                visible: page.openThread && page.messages.length === 0 && (page.ready || Messages.currentIsChat)
                role: "body"
                muted: true
                text: Messages.currentIsChat
                    ? qsTr("No mirrored messages yet")
                    : qsTr("No messages")
            }
        }

        // Staged MMS image attachment preview card.
        Rectangle {
            id: attachmentBar
            visible: (!!page.openThread || page.composing) && !composer.isChat && Messages.hasAttachment
            anchors.left: composer.left
            anchors.right: composer.right
            anchors.bottom: composer.top
            anchors.bottomMargin: 8
            height: 64
            radius: Theme.radiusMd
            color: Theme.surfaceContainerHigh
            border.width: 1
            border.color: Messages.attachmentError.length > 0 ? Theme.error : Theme.outlineVariant

            RoundedImage {
                id: attThumb
                x: 8
                anchors.verticalCenter: parent.verticalCenter
                width: 48; height: 48
                radius: Theme.radiusSm
                visible: Messages.attachmentPreview.length > 0
                source: Messages.attachmentPreview
                sourceSize: Qt.size(96, 96)
                fillMode: Image.PreserveAspectCrop
            }
            Column {
                anchors.left: attThumb.visible ? attThumb.right : parent.left
                anchors.leftMargin: 12
                anchors.right: removeAttBtn.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                Txt {
                    width: parent.width
                    role: "bodySmall"
                    weight: 600
                    elide: Text.ElideRight
                    text: Messages.attachmentName
                }
                Txt {
                    width: parent.width
                    role: "caption"
                    color: Messages.attachmentError.length > 0 ? Theme.error : Theme.surfaceContentVariant
                    elide: Text.ElideRight
                    text: Messages.attachmentError.length > 0
                        ? Messages.attachmentError
                        : qsTr("%1 · Sent as MMS").arg(Messages.attachmentSize)
                }
            }
            IconButton {
                id: removeAttBtn
                anchors.right: parent.right
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                width: 32; height: 32
                iconPath: Icons.close
                label: qsTr("Remove attachment")
                onClicked: Messages.clearAttachment()
            }
        }

        // Writing a text or inline chat reply.
        Rectangle {
            id: composer
            visible: !!page.openThread || page.composing
            readonly property bool isChat: Messages.currentIsChat && !page.composing
            readonly property bool groupSms: !isChat && !!page.openThread && page.openThread.group
            readonly property bool chatReplyDisabled: isChat && !Messages.currentCanReply
            readonly property bool inputDisabled: groupSms || chatReplyDisabled
            readonly property bool validAttachment: !isChat && Messages.hasAttachment && Messages.attachmentError.length === 0
            readonly property bool hasAttachmentError: !isChat && Messages.hasAttachment && Messages.attachmentError.length > 0
            readonly property bool canSend: !Messages.sending
                && !inputDisabled
                && !hasAttachmentError
                && (isChat ? Messages.currentCanReply : page.ready)
                && (input.text.trim().length > 0 || validAttachment)
                && (!page.composing || toField.text.trim().length > 0)
            function focusText() { input.forceActiveFocus() }
            function send() {
                if (!canSend)
                    return
                if (page.composing) {
                    // A name typed in To is its contact's number, as Return makes it.
                    if (!/\d/.test(toField.text) && page.recipientMatches.length > 0)
                        toField.text = page.recipientMatches[0].number
                    Messages.sendTo(toField.text.trim(), input.text)
                }
                else
                    Messages.send(input.text)
                // Your own message is where you look next, wherever you had scrolled to.
                messageList.positionViewAtBeginning()
                input.text = ""
                Messages.saveDraft(page.draftKey, "")
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

            IconButton {
                id: attachButton
                anchors.left: parent.left
                anchors.leftMargin: 6
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                visible: !composer.isChat && !composer.groupSms
                enabled: page.ready && !Messages.sending
                iconPath: Icons.photo
                label: qsTr("Attach image (MMS)")
                onClicked: imageFileDialog.open()
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: {
                        if (Messages.pasteClipboardImage())
                            composer.focusText()
                    }
                }
            }

            // Which SIM it goes out on, when the phone has more than one (#474).
            // A click moves to the next; the phone's label says which is which.
            Chip {
                id: simChip
                readonly property var sims: { try { return JSON.parse(Messages.sims || "[]") } catch (e) { return [] } }
                readonly property int current: Math.max(0, sims.findIndex(s => s.id === Messages.sim))
                readonly property string fullLabel: sims.length > 0 ? sims[current].label : ""
                visible: sims.length > 1 && !composer.isChat && !composer.groupSms
                anchors.left: attachButton.visible ? attachButton.right : parent.left
                anchors.leftMargin: 4
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 9
                interactive: !Messages.sending
                // "SIM 1 · Airtel" shows as "SIM 1"; a label without a slot as it is.
                text: fullLabel.split(" · ")[0]
                Accessible.name: qsTr("Sending from %1. Switch SIM").arg(fullLabel)
                onClicked: Messages.chooseSim(sims[(current + 1) % sims.length].id)
            }

            Flickable {
                id: inputScroll
                anchors.left: simChip.visible ? simChip.right : (attachButton.visible ? attachButton.right : parent.left)
                anchors.leftMargin: simChip.visible || attachButton.visible ? 4 : 18
                anchors.right: sendButton.left
                anchors.rightMargin: 8
                anchors.top: parent.top
                // A single line sits in the middle of the box.
                anchors.topMargin: Math.max(11, (composer.height - input.contentHeight) / 2)
                anchors.bottom: parent.bottom
                anchors.bottomMargin: Math.max(11, (composer.height - input.contentHeight) / 2)
                contentHeight: input.contentHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                TextEdit {
                    id: input
                    width: inputScroll.width
                    wrapMode: TextEdit.Wrap
                    readOnly: composer.inputDisabled
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    Accessible.name: composer.isChat
                        ? qsTr("Reply in %1").arg(Messages.currentAppName)
                        : qsTr("Text message")
                    // Enter sends; Shift+Enter starts a new line; Ctrl+V pastes clipboard image when available.
                    Keys.onPressed: (event) => {
                        if (!composer.isChat && !composer.groupSms
                            && ((event.key === Qt.Key_V && (event.modifiers & Qt.ControlModifier))
                                || event.matches(StandardKey.Paste))) {
                            if (Messages.pasteClipboardImage()) {
                                event.accepted = true
                                return
                            }
                        }
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
                        elide: Text.ElideRight
                        width: input.width
                        text: composer.chatReplyDisabled
                            ? qsTr("Replies are available while the notification is active on your phone")
                            : composer.isChat
                                ? qsTr("Reply in %1…").arg(Messages.currentAppName)
                                : composer.groupSms
                                    ? qsTr("Group texts can't be sent from the PC yet")
                                    : Messages.hasAttachment
                                        ? qsTr("Add a caption (optional)…")
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
                anchors.bottom: attachmentBar.visible ? attachmentBar.top : parent.top
                anchors.bottomMargin: 4
                visible: !composer.isChat && input.text.length > 120
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
        // Chosen among others to delete, mark unread...
        readonly property bool picked: threadSel.has(thread.id)
        readonly property bool isChat: !!thread.isChat
        // Only the phone's own SMS app can mark texts read, so the one being
        // read here would otherwise keep its unread look.
        readonly property int unread: selected ? 0 : thread.unread
        height: 72
        activeFocusOnTab: true
        Accessible.role: Accessible.ListItem
        Accessible.name: thread.title + (isChat && thread.appName ? " (" + thread.appName + ")" : "") + ". " + thread.snippet
        Accessible.onPressAction: open()
        function open() {
            page.composing = false
            Messages.openThread(thread.id)
        }
        // A click: Ctrl (Cmd) toggles it, Shift selects up to it, and while
        // selecting any click does; otherwise it opens.
        function activate(modifiers) {
            const ctrl = (modifiers & Qt.ControlModifier) !== 0
            const shift = (modifiers & Qt.ShiftModifier) !== 0
            if (!threadSel.click(thread.id, page.threadOrder, ctrl, shift))
                open()
        }
        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                activate(event.modifiers)
                event.accepted = true
            }
        }

        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            radius: Theme.radiusLg
            color: row.picked ? Theme.primaryContainer
                 : row.selected ? Theme.secondaryContainer
                 : rowHover.hovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                 : "transparent"
            border.width: Theme.focusVisible(row) ? 2 : 0
            border.color: Theme.primary
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
                emphasized: row.unread > 0
            }
            Rectangle {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.rightMargin: -2
                anchors.bottomMargin: -2
                width: 18; height: 18
                radius: 9
                visible: row.isChat && !!row.thread.appIcon && row.thread.appIcon.length > 0 && appBadgeImg.status === Image.Ready
                color: Theme.surface
                RoundedImage {
                    id: appBadgeImg
                    anchors.centerIn: parent
                    width: 14; height: 14
                    radius: 7
                    source: row.thread.appIcon || ""
                    sourceSize: Qt.size(28, 28)
                    fillMode: Image.PreserveAspectCrop
                }
            }
        }
        // Chosen: a check over the picture; selecting: a ring to choose it by.
        Rectangle {
            x: face.x - 4
            y: face.y - 4
            width: 22; height: 22
            radius: 11
            visible: threadSel.active
            color: row.picked ? Theme.primary : Theme.surface
            border.width: 2
            border.color: row.picked ? Theme.primary : Theme.outline
            Icon {
                anchors.centerIn: parent
                width: 12; height: 12
                visible: row.picked
                path: Icons.check
                color: Theme.primaryContent
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
                    width: Math.min(implicitWidth, parent.width - when.width - (appTag.visible ? appTag.width + 12 : 8))
                    text: row.thread.title
                    role: "body"
                    weight: row.unread > 0 ? 650 : 500
                    elide: Text.ElideRight
                }
                Rectangle {
                    id: appTag
                    anchors.left: name.right
                    anchors.leftMargin: 6
                    anchors.verticalCenter: name.verticalCenter
                    visible: row.isChat && !!row.thread.appName && row.thread.appName.length > 0
                    height: 18
                    width: appTagTxt.implicitWidth + 12
                    radius: 9
                    color: row.selected ? Theme.surfaceContainerHigh : Theme.surfaceContainerHighest
                    Txt {
                        id: appTagTxt
                        anchors.centerIn: parent
                        role: "label"
                        size: 10
                        muted: true
                        text: row.thread.appName || ""
                    }
                }
                Txt {
                    id: when
                    anchors.right: parent.right
                    anchors.baseline: name.baseline
                    text: page.shortTime(row.thread.date)
                    role: "bodySmall"
                    color: row.unread > 0 ? Theme.primary : Theme.surfaceContentVariant
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
                    text: {
                        const s = row.thread.snippet || ""
                        const sender = row.thread.lastSender || ""
                        if (row.isChat && row.thread.group && sender.length > 0 && s.indexOf(sender + ":") !== 0)
                            return sender + ": " + s
                        return s
                    }
                    role: "bodySmall"
                    muted: row.unread === 0
                    maximumLineCount: 1
                    elide: Text.ElideRight
                }
                Rectangle {
                    id: badge
                    anchors.right: parent.right
                    anchors.verticalCenter: snippet.verticalCenter
                    visible: row.unread > 0
                    width: Math.max(20, count.implicitWidth + 10)
                    height: 20
                    radius: 10
                    color: Theme.primary
                    Txt {
                        id: count
                        anchors.centerIn: parent
                        text: row.unread
                        role: "label"
                        size: 11
                        color: Theme.primaryContent
                    }
                }
            }
        }
        HoverHandler { id: rowHover; cursorShape: Qt.PointingHandCursor }
        // Click, or right-click (press and hold on a touch screen) for what can be done with it.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: (mouse) => {
                if (mouse.button === Qt.RightButton)
                    page.openThreadMenu(row, mouse.x, mouse.y, row.thread)
                else
                    row.activate(mouse.modifiers)
            }
            onPressAndHold: (mouse) => {
                if (mouse.source !== Qt.MouseEventNotSynthesized)
                    page.openThreadMenu(row, mouse.x, mouse.y, row.thread)
            }
        }
    }

    // A message: right-aligned when sent from the phone.
    component Bubble: Item {
        id: bubble
        property var message
        // The message before it (the page adds what is needed of it to each row).
        readonly property var previous: message.hasPrev ? ({ date: message.prevDate, sender: message.prevSender }) : null
        readonly property bool mine: message.outgoing
        // Chosen among others to copy, delete...
        readonly property bool picked: msgSel.has(message.id)
        readonly property real maxWidth: Math.min(width * 0.72, 520)
        readonly property bool hasUrl: message.body.indexOf("http://") >= 0 || message.body.indexOf("https://") >= 0
        readonly property bool newDay: {
            if (!previous)
                return true
            // By the calendar alone: 23:55 and 00:02 are two days.
            const d1 = new Date(previous.date)
            const d2 = new Date(message.date)
            return d1.getFullYear() !== d2.getFullYear() || d1.getMonth() !== d2.getMonth() || d1.getDate() !== d2.getDate()
        }
        readonly property bool showSender: !!message.sender && (!previous || previous.sender !== message.sender || newDay)
        height: day.height + sender.height + content.height + meta.height + codeRow.height + 4

        HoverHandler { id: bubbleHover }

        // Chosen: tinted from edge to edge, with a check in the margin.
        Rectangle {
            anchors.fill: parent
            anchors.topMargin: day.height
            visible: bubble.picked
            color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.14)
        }

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
            text: bubble.newDay ? page.dayLabel(bubble.message.date) : ""
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
            readonly property var audioList: bubble.message.audios || []
            width: Math.max(pictures.width, audiosCol.width, Math.min(text.implicitWidth, bubble.maxWidth - 28) + 28)
            height: pictures.height + audiosCol.height + (text.visible ? text.height + 18 : 0)
            radius: 18
            color: bubble.mine ? Theme.primaryContainer : Theme.surfaceContainerHigh
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.outlineVariant
            Column {
                id: pictures
                width: bubble.message.images.length > 0 ? Math.min(260, bubble.maxWidth) : 0
                // By count, so a picture arriving only changes its own source: a new list
                // of the same pictures would build every one of them again.
                Repeater {
                    model: bubble.message.images.length
                    delegate: Item {
                        id: picItem
                        required property int index
                        readonly property string modelData: bubble.message.images[index] || ""
                        width: pictures.width
                        // A frame of one size, loaded or not: a bubble that changes height as its
                        // picture arrives moves everything above it. The picture is cropped to it
                        // and shown whole when opened.
                        height: Math.round(width * 0.7)
                        activeFocusOnTab: modelData.length > 0
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("Open image attachment")
                        Accessible.onPressAction: if (picItem.modelData.length > 0) msgImagePreviewSheet.openFor(picItem.modelData)
                        Keys.onPressed: (event) => {
                            if (picItem.modelData.length > 0 && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                                msgImagePreviewSheet.openFor(picItem.modelData)
                                event.accepted = true
                            }
                        }
                        RoundedImage {
                            id: picture
                            anchors.fill: parent
                            radius: 18
                            source: parent.modelData
                            fillMode: Image.PreserveAspectCrop
                            sourceSize.width: 520
                        }
                        Rectangle {
                            anchors.fill: parent
                            radius: 18
                            color: "transparent"
                            border.width: 2
                            border.color: Theme.primary
                            visible: Theme.focusVisible(picItem)
                        }
                        Spinner { anchors.centerIn: parent; visible: parent.modelData.length === 0 }
                        // Not while selecting: a click then chooses the message.
                        TapHandler {
                            enabled: !msgSel.active
                            acceptedModifiers: Qt.NoModifier
                            onTapped: if (parent.modelData.length > 0) msgImagePreviewSheet.openFor(parent.modelData)
                        }
                        HoverHandler { cursorShape: parent.modelData.length > 0 ? Qt.PointingHandCursor : Qt.ArrowCursor }
                    }
                }
            }
            Column {
                id: audiosCol
                anchors.top: pictures.bottom
                width: content.audioList.length > 0 ? Math.min(240, bubble.maxWidth) : 0
                spacing: 4
                Repeater {
                    model: content.audioList.length
                    delegate: Rectangle {
                        id: audioPill
                        required property int index
                        readonly property var modelData: content.audioList[index]
                        width: audiosCol.width
                        height: 44
                        radius: 14
                        color: "transparent"
                        Row {
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 10
                            Rectangle {
                                width: 28; height: 28; radius: 14
                                anchors.verticalCenter: parent.verticalCenter
                                color: bubble.mine ? Theme.primary : Theme.secondaryContainer
                                Icon {
                                    anchors.centerIn: parent
                                    width: 14; height: 14
                                    path: Icons.speaker
                                    color: bubble.mine ? Theme.primaryContent : Theme.secondaryContainerContent
                                }
                            }
                            Column {
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    role: "bodySmall"
                                    weight: 600
                                    color: bubble.mine ? Theme.primaryContainerContent : Theme.surfaceContent
                                    text: qsTr("Voice note")
                                }
                                Txt {
                                    role: "caption"
                                    color: bubble.mine ? Theme.primaryContainerContent : Theme.surfaceContentVariant
                                    opacity: 0.8
                                    text: (audioPill.modelData.size || "") + (audioPill.modelData.url && audioPill.modelData.url.length > 0 ? qsTr(" · Click to play") : qsTr(" · Loading…"))
                                }
                            }
                        }
                        HoverHandler { cursorShape: audioPill.modelData.url && audioPill.modelData.url.length > 0 ? Qt.PointingHandCursor : Qt.ArrowCursor }
                        TapHandler {
                            onTapped: {
                                if (audioPill.modelData.url && audioPill.modelData.url.length > 0)
                                    Qt.openUrlExternally(audioPill.modelData.url)
                            }
                        }
                    }
                }
            }
            TextEdit {
                id: text
                anchors.top: audiosCol.bottom
                anchors.topMargin: 9
                x: 14
                width: Math.min(implicitWidth, bubble.maxWidth - 28)
                visible: bubble.message.body.length > 0 || bubble.message.attachments > 0
                text: bubble.message.body.length > 0
                    ? (bubble.hasUrl ? page.linkified(bubble.message.body) : bubble.message.body)
                    : qsTr("Attachment (open it on the phone)")
                readOnly: true
                selectByMouse: !msgSel.active
                wrapMode: TextEdit.Wrap
                textFormat: bubble.hasUrl ? TextEdit.RichText : TextEdit.PlainText
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
        // A ring to choose it by while selecting, a check when chosen.
        Rectangle {
            x: 3
            anchors.verticalCenter: content.verticalCenter
            width: 18; height: 18
            radius: 9
            visible: msgSel.active
            color: bubble.picked ? Theme.primary : "transparent"
            border.width: 2
            border.color: bubble.picked ? Theme.primary : Theme.outline
            Icon {
                anchors.centerIn: parent
                width: 10; height: 10
                visible: bubble.picked
                path: Icons.check
                color: Theme.primaryContent
            }
        }
        // Clicks that select, above the text so that it still takes the rest: Ctrl
        // (Cmd) toggles, Shift selects up to it, and any click does while selecting.
        // Right-click, or press and hold on a touch screen, opens the menu.
        Item {
            id: pointer
            anchors.fill: parent
            anchors.topMargin: day.height
            z: 5
            function pick(modifiers) {
                msgSel.click(bubble.message.id, page.messageOrder,
                    (modifiers & Qt.ControlModifier) !== 0, (modifiers & Qt.ShiftModifier) !== 0)
            }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: (point) => page.openMessageMenu(pointer, point.position.x, point.position.y, bubble.message)
            }
            TapHandler {
                acceptedDevices: PointerDevice.TouchScreen
                onLongPressed: page.openMessageMenu(pointer, point.position.x, point.position.y, bubble.message)
            }
            TapHandler {
                acceptedButtons: Qt.LeftButton
                acceptedModifiers: Qt.ControlModifier
                onTapped: pointer.pick(Qt.ControlModifier)
            }
            TapHandler {
                acceptedButtons: Qt.LeftButton
                acceptedModifiers: Qt.ShiftModifier
                onTapped: pointer.pick(Qt.ShiftModifier)
            }
            TapHandler {
                enabled: msgSel.active
                acceptedButtons: Qt.LeftButton
                acceptedModifiers: Qt.NoModifier
                onTapped: pointer.pick(Qt.NoModifier)
            }
        }
        // Ringed for a moment when a clicked notification brought it here.
        Rectangle {
            anchors.fill: content
            anchors.margins: -3
            radius: content.radius + 3
            color: "transparent"
            border.width: 2
            border.color: Theme.primary
            opacity: page.focusId === bubble.message.id ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
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

    // Inline lightbox preview for MMS image attachments.
    Sheet {
        id: msgImagePreviewSheet
        objectName: "imageSheet"
        property string imageUrl: ""
        cardWidth: 680
        function openFor(url) {
            imageUrl = url || ""
            if (imageUrl.length > 0)
                open()
        }
        Column {
            width: parent.width
            spacing: 14
            Item {
                width: parent.width
                height: Math.max(msgPreviewTitle.height, closeMsgPreviewBtn.height)
                Txt {
                    id: msgPreviewTitle
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    role: "headline"
                    text: qsTr("Image attachment")
                }
                IconButton {
                    id: closeMsgPreviewBtn
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.close
                    label: qsTr("Close")
                    onClicked: msgImagePreviewSheet.close()
                }
            }
            Rectangle {
                width: parent.width
                height: Math.min(480, Math.max(200, msgPreviewImg.status === Image.Ready
                    ? width * msgPreviewImg.implicitHeight / Math.max(1, msgPreviewImg.implicitWidth)
                    : 320))
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                clip: true
                Image {
                    id: msgPreviewImg
                    anchors.fill: parent
                    anchors.margins: 8
                    source: msgImagePreviewSheet.imageUrl
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    mipmap: true
                }
            }
            // Right to left: Done is the last, at the right.
            Flow {
                width: parent.width
                layoutDirection: Qt.RightToLeft
                spacing: 8
                Button {
                    text: qsTr("Done")
                    onClicked: msgImagePreviewSheet.close()
                }
                Button {
                    variant: "tonal"
                    iconPath: Icons.photo
                    text: qsTr("Open externally")
                    onClicked: Qt.openUrlExternally(msgImagePreviewSheet.imageUrl)
                }
                // The system's share sheet: on Windows and on a Mac.
                Button {
                    objectName: "shareImage"
                    visible: Qt.platform.os === "windows" || Qt.platform.os === "osx"
                    variant: "tonal"
                    iconPath: Icons.share
                    text: qsTr("Share")
                    onClicked: Messages.shareImage(msgImagePreviewSheet.imageUrl)
                }
                Button {
                    objectName: "saveImage"
                    variant: "tonal"
                    iconPath: Icons.download
                    text: qsTr("Save as…")
                    onClicked: {
                        saveImageDialog.currentFile = Messages.imageSaveSuggestion(msgImagePreviewSheet.imageUrl)
                        saveImageDialog.open()
                    }
                }
                Button {
                    objectName: "copyImage"
                    variant: "tonal"
                    iconPath: Icons.copy
                    text: qsTr("Copy image")
                    onClicked: Messages.copyImage(msgImagePreviewSheet.imageUrl)
                }
            }
        }
    }

    // Deleting asks first, and says what it does: on this PC only, or (asked each time, when the
    // phone can) on the phone too, which can't be undone.
    Sheet {
        id: deleteSheet
        objectName: "deleteSheet"
        cardWidth: 420
        readonly property int count: page.pendingDelete.ids.length
        readonly property bool threads: page.pendingDelete.kind === "threads"
        // Chats read from notifications aren't on the phone's messages.
        readonly property bool phoneToo: Messages.phoneCanDelete
            && (threads ? page.pendingDelete.ids.some(id => id.indexOf("chat:") !== 0) : !Messages.currentIsChat)
        Column {
            width: parent.width
            spacing: 14
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "headline"
                text: deleteSheet.threads ? qsTr("Delete %n conversation(s)?", "", deleteSheet.count)
                    : qsTr("Delete %n message(s)?", "", deleteSheet.count)
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: deleteSheet.phoneToo ? qsTr("Choose where to delete. Deleting on the phone can't be undone.")
                    : deleteSheet.count > 1 ? qsTr("Removes them from this PC. They stay on your phone.")
                    : qsTr("Removes it from this PC. It stays on your phone.")
            }
            // The phone can delete too: two clear choices.
            Column {
                visible: deleteSheet.phoneToo
                width: parent.width
                spacing: 6
                Button {
                    objectName: "confirmDeleteOnPhone"
                    width: parent.width
                    variant: "danger"
                    text: qsTr("Delete here and on the phone")
                    iconPath: Icons.trash
                    onClicked: page.confirmDelete(true)
                }
                Txt {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: qsTr("Also deletes it from your phone's messages. This can't be undone.")
                }
                Item { width: 1; height: 6 }
                Button {
                    objectName: "confirmDeleteHere"
                    width: parent.width
                    variant: "tonal"
                    text: qsTr("Remove from this PC only")
                    onClicked: page.confirmDelete(false)
                }
                Txt {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: qsTr("It stays on your phone.")
                }
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                Button {
                    objectName: "cancelDelete"
                    variant: "text"
                    text: qsTr("Cancel")
                    onClicked: deleteSheet.close()
                }
                Button {
                    objectName: "confirmDelete"
                    visible: !deleteSheet.phoneToo
                    text: qsTr("Delete")
                    iconPath: Icons.trash
                    onClicked: page.confirmDelete(false)
                }
            }
        }
    }

    ContextMenu {
        id: menu
        objectName: "contextMenu"
        onChosen: (key) => page.menuChosen(key)
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: 10
        radius: Theme.radiusLg
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: opacity > 0
        opacity: AppController.highlightTarget === "messages:main" ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    }
}
