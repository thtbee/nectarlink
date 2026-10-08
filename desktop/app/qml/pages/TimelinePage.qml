// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Timeline: searchable, paged local history of everything that moved between
// this PC and paired devices (files, folders, clips, links, saved photos,
// voice recordings, and screen mirroring / webcam sessions).
Item {
    id: page
    property bool active: false
    property bool confirmClearAll: false

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    readonly property var kindOptions: [
        { key: "", label: qsTr("All"), icon: "" },
        { key: "file", label: qsTr("Files"), icon: Icons.folder },
        { key: "clip", label: qsTr("Clips"), icon: Icons.clipboard },
        { key: "link", label: qsTr("Links"), icon: Icons.globe },
        { key: "photo", label: qsTr("Photos"), icon: Icons.photo },
        { key: "recording", label: qsTr("Recordings"), icon: Icons.mic },
        { key: "session", label: qsTr("Sessions"), icon: Icons.mirror }
    ]

    function dayKey(unixSecs) {
        if (!unixSecs || unixSecs <= 0)
            return ""
        return new Date(unixSecs * 1000).toDateString()
    }

    function dayLabel(unixSecs) {
        const date = new Date(unixSecs * 1000)
        const today = new Date()
        const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
        if (date.toDateString() === today.toDateString())
            return qsTr("Today")
        if (date.toDateString() === yesterday.toDateString())
            return qsTr("Yesterday")
        return date.toLocaleDateString(
            Qt.locale(),
            date.getFullYear() === today.getFullYear() ? "dddd, d MMMM" : Locale.LongFormat
        )
    }

    function timeOfDay(unixSecs) {
        return new Date(unixSecs * 1000).toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
    }

    function iconForKind(kind) {
        switch (kind) {
        case "file": return Icons.folder
        case "clip": return Icons.clipboard
        case "link": return Icons.globe
        case "photo": return Icons.photo
        case "recording": return Icons.mic
        case "session": return Icons.mirror
        default: return Icons.history
        }
    }

    Column {
        id: headerCol
        width: Math.min(860, parent.width - Theme.contentPadding * 2)
        x: Math.max(Theme.contentPadding, (parent.width - width) / 2)
        y: Theme.contentPadding
        spacing: 12

        // Search bar + Clear all button
        Item {
            width: parent.width
            height: 40

            Rectangle {
                id: searchBox
                anchors.left: parent.left
                anchors.right: clearBtn.visible ? clearBtn.left : parent.right
                anchors.rightMargin: clearBtn.visible ? 10 : 0
                height: 40
                radius: Theme.graphite ? Theme.radiusSm : height / 2
                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                border.width: searchInput.activeFocus ? 2 : 1
                border.color: searchInput.activeFocus ? Theme.primary : Theme.outlineVariant

                Icon {
                    id: searchIcon
                    x: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16; height: 16
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: searchInput
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 10
                    anchors.right: clearSearchBtn.visible ? clearSearchBtn.left : parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    clip: true
                    text: TimelineModel.searchQuery
                    onTextEdited: TimelineModel.searchQuery = text
                }
                Txt {
                    anchors.left: searchInput.left
                    anchors.verticalCenter: parent.verticalCenter
                    visible: searchInput.text.length === 0
                    role: "body"
                    muted: true
                    text: qsTr("Search files, clips, links, photos, recordings…")
                }
                IconButton {
                    id: clearSearchBtn
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28; height: 28
                    visible: searchInput.text.length > 0
                    iconPath: Icons.close
                    label: qsTr("Clear search")
                    onClicked: {
                        searchInput.text = ""
                        TimelineModel.searchQuery = ""
                    }
                }
            }

            Button {
                id: clearBtn
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                visible: TimelineModel.count > 0
                    && TimelineModel.searchQuery.length === 0
                    && TimelineModel.kindFilter.length === 0
                    && TimelineModel.deviceFilter.length === 0
                variant: "outline"
                size: "sm"
                iconPath: Icons.trash
                text: qsTr("Clear all")
                onClicked: page.confirmClearAll = true
            }
        }

        // Filter chips (by kind, and by device when multiple devices are paired)
        Flow {
            width: parent.width
            spacing: 8

            Repeater {
                model: page.kindOptions
                delegate: Chip {
                    required property var modelData
                    text: modelData.label
                    iconPath: modelData.icon
                    selected: TimelineModel.kindFilter === modelData.key
                    TapHandler { onTapped: TimelineModel.kindFilter = modelData.key }
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                }
            }

            Rectangle {
                visible: DeviceList.count > 1
                width: 1
                height: 22
                y: 4
                color: Theme.outlineVariant
            }

            Chip {
                visible: DeviceList.count > 1
                text: qsTr("All devices")
                selected: TimelineModel.deviceFilter === ""
                TapHandler { onTapped: TimelineModel.deviceFilter = "" }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
            }

            Repeater {
                model: DeviceList.count > 1 ? DeviceList : null
                delegate: Chip {
                    required property string deviceId
                    required property string name
                    text: name
                    iconPath: Icons.phone
                    selected: TimelineModel.deviceFilter === deviceId
                    TapHandler {
                        onTapped: TimelineModel.deviceFilter =
                            (TimelineModel.deviceFilter === deviceId ? "" : deviceId)
                    }
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                }
            }
        }

        // Confirmation bar for Clear all
        Rectangle {
            width: parent.width
            height: confirmRow.height + 20
            visible: page.confirmClearAll && TimelineModel.count > 0
            radius: Theme.radiusMd
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
            border.width: 1
            border.color: Theme.outlineVariant

            Item {
                id: confirmRow
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - 28
                height: Math.max(confirmLabel.height, confirmBtns.height)

                Txt {
                    id: confirmLabel
                    anchors.left: parent.left
                    anchors.right: confirmBtns.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    text: qsTr("Clear all timeline entries on this PC? Saved files, photos and recordings on disk are kept.")
                }
                Row {
                    id: confirmBtns
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8
                    Button {
                        variant: "text"
                        size: "sm"
                        text: qsTr("Cancel")
                        onClicked: page.confirmClearAll = false
                    }
                    Button {
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.trash
                        text: qsTr("Clear timeline")
                        onClicked: {
                            page.confirmClearAll = false
                            TimelineModel.clearAll()
                        }
                    }
                }
            }
        }
    }

    // Empty state
    Column {
        anchors.centerIn: parent
        anchors.verticalCenterOffset: 30
        width: Math.min(420, parent.width - 48)
        spacing: 10
        visible: TimelineModel.count === 0

        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 48; height: 48; radius: 24
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.outlineVariant
            Icon {
                anchors.centerIn: parent
                width: 22; height: 22
                path: Icons.history
                color: Theme.surfaceContentVariant
            }
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: (TimelineModel.searchQuery.length > 0
                   || TimelineModel.kindFilter.length > 0
                   || TimelineModel.deviceFilter.length > 0)
                  ? qsTr("No matching timeline items")
                  : qsTr("Nothing in the timeline yet")
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "bodySmall"
            muted: true
            wrapMode: Text.WordWrap
            text: (TimelineModel.searchQuery.length > 0
                   || TimelineModel.kindFilter.length > 0
                   || TimelineModel.deviceFilter.length > 0)
                  ? qsTr("Try clearing the search or filter to see everything shared with your devices.")
                  : qsTr("Files, folders, clipboard items, links, saved photos, voice recordings, and screen mirroring or webcam sessions appear here automatically.")
        }
    }

    // Paged timeline list
    ListView {
        id: listView
        anchors.top: headerCol.bottom
        anchors.topMargin: 12
        anchors.bottom: parent.bottom
        width: headerCol.width
        x: headerCol.x
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: 8
        model: TimelineModel
        visible: TimelineModel.count > 0

        onContentYChanged: {
            if (TimelineModel.hasMore && contentHeight > height && contentY + height >= contentHeight - 260)
                TimelineModel.loadMore()
        }

        delegate: Column {
            id: rowRoot
            required property int index
            required property string entryId
            required property string kind
            required property string direction
            required property bool incoming
            required property string deviceId
            required property string deviceName
            required property real timestamp
            required property real prevTimestamp
            required property string title
            required property string subtitle
            required property real sizeBytes
            required property int durationSecs
            required property int itemCount
            required property string thumbUrl
            required property bool canOpen
            required property bool canShowInFolder
            required property bool canCopy
            required property bool canSendAgain

            readonly property bool showDay: index === 0 || page.dayKey(timestamp) !== page.dayKey(prevTimestamp)

            width: listView.width
            spacing: 6

            Txt {
                visible: rowRoot.showDay
                topPadding: rowRoot.index === 0 ? 2 : 10
                bottomPadding: 2
                role: "label"
                muted: true
                text: rowRoot.showDay ? page.dayLabel(rowRoot.timestamp) : ""
            }

            Rectangle {
                id: itemCard
                width: parent.width
                height: Math.max(62, textCol.implicitHeight + 24)
                radius: Theme.radiusMd
                color: Theme.cardColor
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.cardBorder

                // Leading thumbnail or kind icon
                Item {
                    id: leading
                    x: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 40; height: 40

                    RoundedImage {
                        id: thumbImg
                        anchors.fill: parent
                        visible: rowRoot.thumbUrl.length > 0 && status === Image.Ready
                        radius: Theme.radiusSm
                        source: rowRoot.thumbUrl
                        fillMode: Image.PreserveAspectCrop
                    }

                    Rectangle {
                        anchors.fill: parent
                        visible: !thumbImg.visible
                        radius: Theme.radiusSm
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: Theme.graphite ? 1 : 0
                        border.color: Theme.outlineVariant
                        Icon {
                            anchors.centerIn: parent
                            width: 18; height: 18
                            path: page.iconForKind(rowRoot.kind)
                            color: Theme.primary
                        }
                    }
                }

                // Title & subtitle
                Column {
                    id: textCol
                    anchors.left: leading.right
                    anchors.leftMargin: 12
                    anchors.right: actionsRow.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2

                    Row {
                        width: parent.width
                        spacing: 8
                        Txt {
                            width: Math.min(implicitWidth, parent.width - timeText.implicitWidth - 10)
                            role: "body"
                            weight: 600
                            elide: Text.ElideRight
                            text: rowRoot.title
                        }
                        Txt {
                            id: timeText
                            anchors.verticalCenter: parent.verticalCenter
                            role: "caption"
                            muted: true
                            text: page.timeOfDay(rowRoot.timestamp)
                        }
                    }

                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        elide: Text.ElideRight
                        text: rowRoot.subtitle
                    }
                }

                // Actions: Open / Show in folder / Copy / Send again / Remove
                Row {
                    id: actionsRow
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6

                    Button {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: rowRoot.canOpen
                        variant: "tonal"
                        size: "sm"
                        iconPath: rowRoot.kind === "link" ? Icons.openExternal : ""
                        text: qsTr("Open")
                        onClicked: TimelineModel.openEntry(rowRoot.entryId)
                    }
                    Button {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: rowRoot.canShowInFolder
                        variant: "text"
                        size: "sm"
                        iconPath: Icons.folder
                        text: qsTr("Show in folder")
                        onClicked: TimelineModel.revealEntry(rowRoot.entryId)
                    }
                    Button {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: rowRoot.canCopy
                        variant: rowRoot.canOpen ? "text" : "tonal"
                        size: "sm"
                        iconPath: Icons.copy
                        text: qsTr("Copy")
                        onClicked: TimelineModel.copyEntry(rowRoot.entryId)
                    }
                    Button {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: rowRoot.canSendAgain
                        variant: "text"
                        size: "sm"
                        iconPath: Icons.send
                        text: qsTr("Send again")
                        onClicked: TimelineModel.sendAgain(rowRoot.entryId, "")
                    }
                    IconButton {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 30; height: 30
                        iconPath: Icons.close
                        label: qsTr("Remove from timeline")
                        onClicked: TimelineModel.removeEntry(rowRoot.entryId)
                    }
                }
            }
        }

        footer: Item {
            width: listView.width
            height: TimelineModel.hasMore ? 56 : 24
            Button {
                anchors.centerIn: parent
                visible: TimelineModel.hasMore
                variant: "tonal"
                size: "sm"
                text: qsTr("Load more")
                onClicked: TimelineModel.loadMore()
            }
        }
    }
}
