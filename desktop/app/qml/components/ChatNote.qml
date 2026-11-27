// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A note in the chat with a device (#698): this PC's on the right, the
// device's on the left, its time under it. Copy, Edit and Delete sit beside
// it while it's hovered or one of them has focus.
Item {
    id: note
    property string text
    property bool mine: false
    // Unix seconds.
    property real timestamp: 0
    // The day, above the first note of a day ("" for none).
    property string dayLabel: ""
    property bool canEdit: false
    // The note being changed in the box below.
    property bool editing: false
    signal copyRequested
    signal editRequested
    signal deleteRequested

    readonly property real maxWidth: Math.min(width * 0.72, 560)
    readonly property bool hasUrl: text.indexOf("http://") >= 0 || text.indexOf("https://") >= 0
    readonly property bool showActions: hover.hovered || copyButton.activeFocus || editButton.activeFocus
                                        || deleteButton.activeFocus
    readonly property string timeText: new Date(timestamp * 1000).toLocaleTimeString(Qt.locale(), Locale.ShortFormat)

    implicitHeight: day.height + bubble.height + meta.height + 7
    height: implicitHeight

    // Text as HTML with its web links clickable.
    function linkified(text) {
        const escaped = text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\n/g, "<br>")
        return escaped.replace(/\b(https?:\/\/[^\s<]+[^\s<.,;:!?)\]'"])/g, '<a href="$1">$1</a>')
    }

    HoverHandler { id: hover }

    Item {
        id: day
        width: parent.width
        height: note.dayLabel.length > 0 ? dayText.implicitHeight + 20 : 0
        visible: note.dayLabel.length > 0
        Txt {
            id: dayText
            y: 8
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "label"
            muted: true
            text: note.dayLabel
        }
    }

    Rectangle {
        id: bubble
        objectName: "bubble"
        anchors.top: day.bottom
        anchors.right: note.mine ? parent.right : undefined
        anchors.left: note.mine ? undefined : parent.left
        anchors.leftMargin: 24
        anchors.rightMargin: 24
        width: Math.min(body.implicitWidth, note.maxWidth - 28) + 28
        height: body.height + 18
        radius: 18
        color: note.mine ? Theme.primaryContainer : Theme.surfaceContainerHigh
        border.width: note.editing ? 2 : (Theme.graphite ? 1 : 0)
        border.color: note.editing ? Theme.primary : Theme.outlineVariant

        TextEdit {
            id: body
            x: 14
            y: 9
            width: Math.min(implicitWidth, note.maxWidth - 28)
            text: note.hasUrl ? note.linkified(note.text) : note.text
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.Wrap
            textFormat: note.hasUrl ? TextEdit.RichText : TextEdit.PlainText
            onLinkActivated: (link) => Qt.openUrlExternally(link)
            HoverHandler { cursorShape: body.hoveredLink.length > 0 ? Qt.PointingHandCursor : Qt.IBeamCursor }
            font.family: Theme.fontUi
            font.pixelSize: 14
            color: note.mine ? Theme.primaryContainerContent : Theme.surfaceContent
            selectionColor: Theme.primary
            selectedTextColor: Theme.primaryContent
            Accessible.name: note.text
            Accessible.description: note.timeText
        }
    }

    Txt {
        id: meta
        anchors.top: bubble.bottom
        anchors.topMargin: 3
        anchors.right: note.mine ? bubble.right : undefined
        anchors.left: note.mine ? undefined : bubble.left
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        role: "bodySmall"
        size: 11
        color: Theme.surfaceContentVariant
        text: note.timeText
    }

    Row {
        objectName: "actions"
        anchors.verticalCenter: bubble.verticalCenter
        anchors.right: note.mine ? bubble.left : undefined
        anchors.left: note.mine ? undefined : bubble.right
        anchors.margins: 4
        spacing: 2
        opacity: note.showActions ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        IconButton {
            id: copyButton
            objectName: "copyButton"
            width: 32; height: 32
            iconPath: Icons.copy
            label: qsTr("Copy text")
            onClicked: note.copyRequested()
        }
        IconButton {
            id: editButton
            objectName: "editButton"
            width: 32; height: 32
            visible: note.canEdit
            iconPath: Icons.edit
            label: qsTr("Edit")
            onClicked: note.editRequested()
        }
        IconButton {
            id: deleteButton
            objectName: "deleteButton"
            width: 32; height: 32
            iconPath: Icons.trash
            label: qsTr("Delete")
            onClicked: note.deleteRequested()
        }
    }
}
