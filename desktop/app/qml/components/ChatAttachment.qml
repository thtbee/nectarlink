// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A file, photo or recording in the chat with a device (#698), on the side
// of whoever sent it: a thumbnail when it's an image on this PC, else an
// icon, with its name and size. Clicking it opens it (or shows it in its
// folder); Open, Show in folder and Remove from chat sit beside it while
// it's hovered or one of them has focus.
Item {
    id: attachment
    property string title
    // "file", "photo" or "recording".
    property string kind: "file"
    // The size in words ("" when it isn't known).
    property string size
    property string thumbUrl
    property bool mine: false
    // Unix seconds.
    property real timestamp: 0
    // The day, above the first item of a day ("" for none).
    property string dayLabel: ""
    property bool canOpen: false
    property bool canShowInFolder: false
    signal openRequested
    signal showInFolderRequested
    signal removeRequested

    readonly property real maxWidth: Math.min(width * 0.72, 420)
    readonly property bool hasThumb: thumbUrl.length > 0 && thumb.status === Image.Ready
    // Its file isn't on this PC (any more).
    readonly property bool gone: !canOpen && !canShowInFolder
    readonly property string sizeText: gone
        ? (size.length > 0 ? qsTr("%1 · Moved or deleted").arg(size) : qsTr("Moved or deleted"))
        : size
    readonly property bool showActions: hover.hovered || bubble.activeFocus || openButton.activeFocus
                                        || folderButton.activeFocus || removeButton.activeFocus
    readonly property string timeText: new Date(timestamp * 1000).toLocaleTimeString(Qt.locale(), Locale.ShortFormat)

    implicitHeight: day.height + bubble.height + meta.height + 7
    height: implicitHeight

    function iconFor(kind, title) {
        if (kind === "photo") return Icons.photo
        if (kind === "recording") return Icons.mic
        const t = (title || "").toLowerCase()
        if (/\.(jpg|jpeg|png|webp|gif|bmp|heic|svg)$/.test(t)) return Icons.photo
        if (/\.(mp4|mkv|mov|webm|avi)$/.test(t)) return Icons.video
        if (/\.(mp3|m4a|wav|flac|ogg|opus|aac)$/.test(t)) return Icons.music
        return Icons.file
    }

    // A click: opens it, or shows it in its folder when it can't be opened.
    function activate() {
        if (canOpen)
            openRequested()
        else if (canShowInFolder)
            showInFolderRequested()
    }

    HoverHandler { id: hover }

    Item {
        id: day
        width: parent.width
        height: attachment.dayLabel.length > 0 ? dayText.implicitHeight + 20 : 0
        visible: attachment.dayLabel.length > 0
        Txt {
            id: dayText
            y: 8
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "label"
            muted: true
            text: attachment.dayLabel
        }
    }

    Rectangle {
        id: bubble
        objectName: "bubble"
        anchors.top: day.bottom
        anchors.right: attachment.mine ? parent.right : undefined
        anchors.left: attachment.mine ? undefined : parent.left
        anchors.leftMargin: 24
        anchors.rightMargin: 24
        width: Math.min(Math.max(nameText.implicitWidth, sizeLabel.implicitWidth) + leading.width + 34,
                        attachment.maxWidth)
        height: leading.height + 16
        radius: 18
        color: attachment.mine ? Theme.primaryContainer : Theme.surfaceContainerHigh
        border.width: Theme.focusVisible(bubble) ? 2 : (Theme.graphite ? 1 : 0)
        border.color: Theme.focusVisible(bubble) ? Theme.primary : Theme.outlineVariant
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: attachment.title
        Accessible.description: attachment.sizeText.length > 0
            ? attachment.sizeText + ", " + attachment.timeText : attachment.timeText
        Accessible.onPressAction: attachment.activate()
        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                attachment.activate()
                event.accepted = true
            }
        }

        HoverHandler { cursorShape: attachment.gone ? Qt.ArrowCursor : Qt.PointingHandCursor }
        TapHandler { onTapped: attachment.activate() }

        // The image itself, or what kind of file it is.
        Item {
            id: leading
            x: 8
            anchors.verticalCenter: parent.verticalCenter
            width: 48; height: 48

            RoundedImage {
                id: thumb
                objectName: "thumb"
                anchors.fill: parent
                visible: attachment.hasThumb
                radius: 12
                source: attachment.thumbUrl
                sourceSize: Qt.size(96, 96)
                fillMode: Image.PreserveAspectCrop
            }
            Rectangle {
                objectName: "fileIcon"
                anchors.fill: parent
                visible: !attachment.hasThumb
                radius: 12
                color: attachment.mine ? Theme.surfaceContainerLow : Theme.surfaceContainerHighest
                Icon {
                    anchors.centerIn: parent
                    width: 20; height: 20
                    path: attachment.iconFor(attachment.kind, attachment.title)
                    color: Theme.primary
                }
            }
        }

        Column {
            anchors.left: leading.right
            anchors.leftMargin: 10
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Txt {
                id: nameText
                objectName: "name"
                width: parent.width
                role: "body"
                weight: 600
                elide: Text.ElideMiddle
                maximumLineCount: 1
                color: attachment.mine ? Theme.primaryContainerContent : Theme.surfaceContent
                text: attachment.title
            }
            Txt {
                id: sizeLabel
                objectName: "size"
                width: parent.width
                visible: text.length > 0
                role: "bodySmall"
                elide: Text.ElideRight
                color: attachment.mine ? Theme.primaryContainerContent : Theme.surfaceContentVariant
                opacity: attachment.mine ? 0.8 : 1
                text: attachment.sizeText
            }
        }
    }

    Txt {
        id: meta
        anchors.top: bubble.bottom
        anchors.topMargin: 3
        anchors.right: attachment.mine ? bubble.right : undefined
        anchors.left: attachment.mine ? undefined : bubble.left
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        role: "bodySmall"
        size: 11
        color: Theme.surfaceContentVariant
        text: attachment.timeText
    }

    Row {
        objectName: "actions"
        anchors.verticalCenter: bubble.verticalCenter
        anchors.right: attachment.mine ? bubble.left : undefined
        anchors.left: attachment.mine ? undefined : bubble.right
        anchors.margins: 4
        spacing: 2
        opacity: attachment.showActions ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        IconButton {
            id: openButton
            objectName: "openButton"
            width: 32; height: 32
            visible: attachment.canOpen
            iconPath: Icons.openExternal
            label: qsTr("Open")
            onClicked: attachment.openRequested()
        }
        IconButton {
            id: folderButton
            objectName: "folderButton"
            width: 32; height: 32
            visible: attachment.canShowInFolder
            iconPath: Icons.folder
            label: qsTr("Show in folder")
            onClicked: attachment.showInFolderRequested()
        }
        IconButton {
            id: removeButton
            objectName: "removeButton"
            width: 32; height: 32
            iconPath: Icons.close
            label: qsTr("Remove from chat")
            onClicked: attachment.removeRequested()
        }
    }
}
