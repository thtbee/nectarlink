// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import app.nectarlink

// Chat: notes between this PC and the device shown on Home (#698), oldest
// at the top. The device's on the left, this PC's on the right, and a box
// to write one below. Opened from Home, or by clicking a note's notification.
Item {
    id: page
    property bool active: true
    property string deviceId
    property string deviceName
    visible: active

    readonly property string name: ChatModel.deviceName.length > 0 ? ChatModel.deviceName : deviceName
    readonly property bool canWrite: ChatModel.online && ChatModel.supported
    // Why nothing can be written now ("" when it can).
    readonly property string reason: !ChatModel.online ? qsTr("%1 isn't connected").arg(page.name)
        : !ChatModel.supported ? qsTr("Update NectarLink on %1 to chat").arg(page.name)
        : ""
    // The note of this PC's being changed in the box ("" for none).
    property string editingId: ""
    // A note's text is at most this long.
    readonly property int maxChars: 8192
    readonly property int typedChars: Array.from(input.text).length
    readonly property bool tooLong: typedChars > maxChars

    // In front: a note that comes in needs no notification.
    readonly property bool inFront: active && Window.window !== null && Window.window.visible && Window.window.active
    onInFrontChanged: ChatModel.setInFront(inFront)
    onDeviceIdChanged: {
        stopEditing()
        input.text = ""
        ChatModel.device = deviceId
        noteList.followLatest = true
    }
    Component.onCompleted: {
        ChatModel.device = deviceId
        ChatModel.setInFront(inFront)
        input.forceActiveFocus()
    }
    Component.onDestruction: {
        ChatModel.setInFront(false)
        ChatModel.device = ""
    }

    Connections {
        target: ChatModel
        // Not sent: back in the box, unless something else was typed since.
        function onSendFailed(text) {
            if (input.text.length === 0 && page.editingId.length === 0)
                input.text = text
        }
    }

    function dayKey(unixSecs) {
        return unixSecs > 0 ? new Date(unixSecs * 1000).toDateString() : ""
    }

    // "Today", "Yesterday", or the date, for separators.
    function dayLabel(unixSecs) {
        const date = new Date(unixSecs * 1000)
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

    function startEditing(noteId, text) {
        editingId = noteId
        input.text = text
        input.cursorPosition = input.length
        input.forceActiveFocus()
    }
    function stopEditing() {
        if (editingId.length === 0)
            return
        editingId = ""
        input.text = ""
    }

    // Enter: sends the note, or saves the one being changed.
    function submit() {
        const text = input.text
        if (text.trim().length === 0 || tooLong || !canWrite)
            return
        if (editingId.length > 0) {
            ChatModel.edit(editingId, text)
            editingId = ""
            input.text = ""
            return
        }
        if (ChatModel.sending)
            return
        ChatModel.send(text)
        input.text = ""
        // Your own note is where you look next.
        noteList.followLatest = true
    }

    Item {
        id: column
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(parent.width - Theme.contentPadding * 2 + 32, 860)
        height: parent.height

        // ---- No notes yet ----
        Column {
            anchors.centerIn: noteList
            width: Math.min(parent.width - 48, 420)
            spacing: 12
            visible: ChatModel.count === 0
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
                text: qsTr("No notes yet")
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("What you write here shows on %1, and the other way round.").arg(page.name)
            }
        }

        ScrollRail { flick: noteList }
        WheelScroll {
            id: wheelScroll
            flick: noteList
            onMoved: noteList.followLatest = false
            onSettled: noteList.noteEnd()
        }
        ListView {
            id: noteList
            anchors.top: parent.top
            anchors.bottom: transfersCard.visible ? transfersCard.top
                : editBar.visible ? editBar.top : composer.top
            anchors.bottomMargin: 8
            width: parent.width
            clip: true
            spacing: 6
            cacheBuffer: 960
            // A short chat sits at the bottom, by the box.
            topMargin: Math.max(16, height - contentHeight - bottomMargin)
            bottomMargin: 8
            boundsBehavior: Flickable.StopAtBounds
            model: ChatModel
            // Stays at the latest note until the reader scrolls away from it.
            property bool followLatest: true
            function noteEnd() {
                followLatest = originY + contentHeight + bottomMargin - contentY - height <= 8
            }
            function keepAtEnd() {
                if (followLatest)
                    positionViewAtEnd()
            }
            onCountChanged: Qt.callLater(keepAtEnd)
            onContentHeightChanged: if (followLatest) Qt.callLater(keepAtEnd)
            onHeightChanged: if (followLatest) Qt.callLater(keepAtEnd)
            onMovementStarted: followLatest = false
            onMovementEnded: noteEnd()
            onFlickEnded: noteEnd()
            // Near the top, by the reader's own scrolling: older notes.
            onContentYChanged: {
                const readerMoving = moving || dragging || flicking || wheelScroll.active
                if (readerMoving && ChatModel.hasMore && count > 0 && !followLatest
                    && contentY - (originY - topMargin) < 160)
                    ChatModel.loadMore()
            }
            // A note, or a file, photo or recording sent between the two.
            delegate: Item {
                id: entry
                required property int index
                required property string entryId
                required property string kind
                required property string noteId
                required property string text
                required property bool incoming
                required property real timestamp
                required property real prevTimestamp
                required property string title
                required property string size
                required property string thumbUrl
                required property bool canOpen
                required property bool canShowInFolder
                readonly property string dayLabel: index === 0 || page.dayKey(timestamp) !== page.dayKey(prevTimestamp)
                    ? page.dayLabel(timestamp) : ""
                width: noteList.width
                height: shown.item ? shown.item.height : 0

                Loader {
                    id: shown
                    width: parent.width
                    sourceComponent: entry.kind === "note" ? noteComponent : attachmentComponent
                }
                Component {
                    id: noteComponent
                    ChatNote {
                        width: entry.width
                        text: entry.text
                        timestamp: entry.timestamp
                        mine: !entry.incoming
                        // Changing a note is for the device that wrote it, while the other has it too.
                        canEdit: !entry.incoming && page.canWrite
                        editing: page.editingId === entry.noteId
                        dayLabel: entry.dayLabel
                        onCopyRequested: Messages.copy(entry.text)
                        onEditRequested: page.startEditing(entry.noteId, entry.text)
                        onDeleteRequested: deleteSheet.openFor(entry.noteId)
                    }
                }
                Component {
                    id: attachmentComponent
                    ChatAttachment {
                        width: entry.width
                        kind: entry.kind
                        title: entry.title
                        size: entry.size
                        thumbUrl: entry.thumbUrl
                        timestamp: entry.timestamp
                        mine: !entry.incoming
                        canOpen: entry.canOpen
                        canShowInFolder: entry.canShowInFolder
                        dayLabel: entry.dayLabel
                        onOpenRequested: ChatModel.openAttachment(entry.entryId)
                        onShowInFolderRequested: ChatModel.showAttachmentInFolder(entry.entryId)
                        onRemoveRequested: ChatModel.removeAttachment(entry.entryId)
                    }
                }
            }
        }

        // Files on their way to or from the device, or that didn't make it;
        // each joins the chat once it's through.
        Rectangle {
            id: transfersCard
            objectName: "transfersCard"
            visible: transfersColumn.height > 0
            anchors.left: composer.left
            anchors.right: composer.right
            anchors.bottom: editBar.visible ? editBar.top : composer.top
            anchors.bottomMargin: 8
            height: transfersColumn.height
            radius: Theme.radiusMd
            color: Theme.cardColor
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.cardBorder
            Column {
                id: transfersColumn
                x: 12
                width: parent.width - 24
                Repeater {
                    model: TransferList
                    delegate: TransferItem {
                        required property string deviceId
                        width: transfersColumn.width
                        visible: deviceId === page.deviceId && (active || status === "failed")
                    }
                }
            }
        }

        // Changing a note: says so, and how to stop.
        Item {
            id: editBar
            visible: page.editingId.length > 0
            anchors.left: composer.left
            anchors.right: composer.right
            anchors.bottom: composer.top
            anchors.bottomMargin: 4
            height: 34
            Icon {
                id: editIcon
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 14; height: 14
                path: Icons.edit
                color: Theme.primary
            }
            Txt {
                anchors.left: editIcon.right
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                role: "bodySmall"
                muted: true
                text: qsTr("Editing note")
            }
            IconButton {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: 30; height: 30
                iconPath: Icons.close
                label: qsTr("Cancel editing")
                onClicked: {
                    page.stopEditing()
                    input.forceActiveFocus()
                }
            }
        }

        // Writing a note.
        Rectangle {
            id: composer
            readonly property bool canSend: page.canWrite && !page.tooLong && input.text.trim().length > 0
                && (page.editingId.length > 0 || !ChatModel.sending)
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 16
            height: Math.min(Math.max(input.contentHeight + 22, 48), 160)
            radius: Theme.graphite ? Theme.radiusSm : 24
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: input.activeFocus ? 2 : 1
            border.color: input.activeFocus ? Theme.primary : Theme.outlineVariant

            // Files go whether or not notes can: they wait for the device.
            IconButton {
                id: attachButton
                objectName: "attachButton"
                anchors.left: parent.left
                anchors.leftMargin: 6
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                iconPath: Icons.attach
                label: qsTr("Send files")
                enabled: page.deviceId.length > 0
                onClicked: filePicker.open()
            }
            Flickable {
                id: inputScroll
                opacity: page.canWrite ? 1 : 0.7
                anchors.left: attachButton.right
                anchors.leftMargin: 6
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
                    readOnly: !page.canWrite
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    Accessible.name: page.editingId.length > 0 ? qsTr("Editing note") : qsTr("Note to %1").arg(page.name)
                    Accessible.description: page.reason
                    // Enter sends; Shift+Enter starts a new line; Esc stops changing a note.
                    Keys.onPressed: (event) => {
                        if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && !(event.modifiers & Qt.ShiftModifier)) {
                            page.submit()
                            event.accepted = true
                        } else if (event.key === Qt.Key_Escape && page.editingId.length > 0) {
                            page.stopEditing()
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
                        text: page.reason.length > 0 ? page.reason : qsTr("Note to %1").arg(page.name)
                    }
                }
            }
            IconButton {
                id: sendButton
                anchors.right: parent.right
                anchors.rightMargin: 6
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                iconPath: page.editingId.length > 0 ? Icons.check : Icons.send
                label: page.editingId.length > 0 ? qsTr("Save") : qsTr("Send")
                tonal: composer.canSend
                enabled: composer.canSend
                onClicked: page.submit()
            }
            Spinner {
                anchors.centerIn: sendButton
                visible: ChatModel.sending && page.editingId.length === 0
            }
            // Close to the limit: how close.
            Txt {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.bottom: parent.top
                anchors.bottomMargin: 4
                visible: page.typedChars > page.maxChars - 200
                role: "bodySmall"
                color: page.tooLong ? Theme.error : Theme.surfaceContentVariant
                text: Qt.locale().toString(page.typedChars) + " / " + Qt.locale().toString(page.maxChars)
            }
        }
    }

    // Files picked or dropped go to the device, as Send files on Home does;
    // each shows in the chat once it's through.
    function sendFiles(urls) {
        if (urls.length === 0 || deviceId.length === 0)
            return
        TransferList.send(deviceId, urls)
        noteList.followLatest = true
    }

    FileDialog {
        id: filePicker
        title: qsTr("Send files to %1").arg(page.name)
        fileMode: FileDialog.OpenFiles
        onAccepted: page.sendFiles(selectedFiles.map(url => url.toString()))
    }

    DropArea {
        id: drop
        anchors.fill: parent
        enabled: page.deviceId.length > 0
        onEntered: (drag) => drag.accepted = drag.hasUrls
        onDropped: (drag) => {
            if (drag.hasUrls) {
                page.sendFiles(drag.urls.map(url => url.toString()))
                drag.accept(Qt.CopyAction)
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: Theme.contentPadding / 2
        radius: Theme.radiusXl
        visible: opacity > 0
        opacity: drop.containsDrag ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        color: Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b, 0.92)
        border.width: 2
        border.color: Theme.primary
        Column {
            anchors.centerIn: parent
            spacing: 8
            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 40; height: 40
                stroke: 1.6
                path: Icons.attach
                color: Theme.primaryContainerContent
            }
            Txt {
                anchors.horizontalCenter: parent.horizontalCenter
                role: "headline"
                color: Theme.primaryContainerContent
                text: qsTr("Drop to send to %1").arg(page.name)
            }
        }
    }

    // Deleting a note: it goes from both devices.
    Sheet {
        id: deleteSheet
        property string noteId: ""
        cardWidth: 400
        function openFor(id) {
            noteId = id
            open()
        }
        Column {
            width: parent.width
            spacing: 18
            Txt {
                width: parent.width
                role: "headline"
                wrapMode: Text.WordWrap
                text: qsTr("Delete note?")
            }
            Txt {
                width: parent.width
                role: "body"
                muted: true
                wrapMode: Text.WordWrap
                text: ChatModel.online
                    ? qsTr("It's deleted on this PC and on %1.").arg(page.name)
                    : qsTr("It's deleted on this PC. %1 isn't connected, so it keeps its copy.").arg(page.name)
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                Button {
                    variant: "text"
                    text: qsTr("Cancel")
                    onClicked: deleteSheet.close()
                }
                Button {
                    iconPath: Icons.trash
                    text: qsTr("Delete")
                    onClicked: {
                        if (page.editingId === deleteSheet.noteId)
                            page.stopEditing()
                        ChatModel.remove(deleteSheet.noteId)
                        deleteSheet.close()
                        input.forceActiveFocus()
                    }
                }
            }
        }
    }
}
