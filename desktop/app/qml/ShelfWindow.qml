// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// Slide-out edge panel ("The Shelf") showing the latest photo, latest
// screenshot, current clipboard item, and recent received files. Dragging
// anything into the panel sends it to the connected phone; dragging any card
// out starts a native OS drag so it can be dropped into Explorer, browsers,
// or editors. Styled with WS_EX_NOACTIVATE so it never steals focus from the
// active application, and only instantiated while AppController.shelfOpen is
// true so nothing runs while hidden.
Window {
    id: shelfWin

    title: "Nectarlink Shelf"
    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
    color: "transparent"
    visible: true

    readonly property int availW: Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920
    readonly property int availH: Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : 1080

    width: 348
    height: Math.min(620, Math.max(420, availH - 48))
    x: Screen.virtualX + availW - width - 12
    y: Screen.virtualY + Math.round((availH - height) / 2)

    property bool justDropped: false

    readonly property var shelfData: {
        if (!AppController.shelfItems || AppController.shelfItems === "") {
            return ({
                hasDevice: false,
                deviceName: "",
                latestPhoto: null,
                latestScreenshot: null,
                currentClip: null,
                recentFiles: []
            })
        }
        try {
            return JSON.parse(AppController.shelfItems)
        } catch (e) {
            return ({
                hasDevice: false,
                deviceName: "",
                latestPhoto: null,
                latestScreenshot: null,
                currentClip: null,
                recentFiles: []
            })
        }
    }

    function formatBytes(bytes) {
        const b = Number(bytes) || 0
        if (b < 1024)
            return qsTr("%1 B").arg(b)
        if (b < 1024 * 1024)
            return qsTr("%1 KB").arg((b / 1024).toFixed(1))
        if (b < 1024 * 1024 * 1024)
            return qsTr("%1 MB").arg((b / (1024 * 1024)).toFixed(1))
        return qsTr("%1 GB").arg((b / (1024 * 1024 * 1024)).toFixed(2))
    }

    Component.onCompleted: {
        Qt.callLater(() => AppController.styleShelfWindow())
        if (!Theme.reduceMotion) {
            slideAnim.restart()
        }
    }

    onVisibleChanged: {
        if (visible) {
            Qt.callLater(() => AppController.styleShelfWindow())
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: AppController.closeShelf()
    }

    Timer {
        id: dropConfirmTimer
        interval: 1800
        onTriggered: shelfWin.justDropped = false
    }

    Item {
        id: rootItem
        anchors.fill: parent

        transform: Translate {
            id: slideTransform
            x: 0
        }

        NumberAnimation {
            id: slideAnim
            target: slideTransform
            property: "x"
            from: 24
            to: 0
            duration: Theme.reduceMotion ? 0 : 180
            easing.type: Easing.OutCubic
        }

        Rectangle {
            id: panel
            anchors.fill: parent
            radius: Theme.graphite ? Theme.radiusMd : Theme.radiusXl
            color: Theme.surface
            border.width: 1
            border.color: shelfDropArea.containsDrag ? Theme.primary : Theme.outlineVariant

            Behavior on border.color {
                enabled: !Theme.reduceMotion
                ColorAnimation { duration: Theme.fadeFast }
            }

            // Whole-window drop target so dropping anywhere on the Shelf sends to phone.
            DropArea {
                id: shelfDropArea
                anchors.fill: parent
                onDropped: (drop) => {
                    const urls = []
                    if (drop.urls) {
                        for (let i = 0; i < drop.urls.length; i++) {
                            urls.push(drop.urls[i].toString())
                        }
                    }
                    const text = drop.text ? String(drop.text) : ""
                    if (urls.length > 0 || text.trim().length > 0) {
                        AppController.dropToShelf(urls, text)
                        shelfWin.justDropped = true
                        dropConfirmTimer.restart()
                    }
                    drop.acceptProposedAction()
                }
            }

            Column {
                id: mainColumn
                anchors.fill: parent
                anchors.margins: 14
                spacing: 10

                // ---- Header ----
                Item {
                    width: parent.width
                    height: 36

                    Row {
                        anchors.left: parent.left
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8

                        Rectangle {
                            width: 28
                            height: 28
                            radius: Theme.pill(height)
                            color: Theme.primaryContainer
                            anchors.verticalCenter: parent.verticalCenter

                            Icon {
                                anchors.centerIn: parent
                                width: 15
                                height: 15
                                path: Icons.sparkle
                                color: Theme.primaryContainerContent
                            }
                        }

                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 0

                            Txt {
                                role: "title"
                                size: 15
                                weight: 650
                                text: qsTr("Shelf")
                            }

                            Txt {
                                role: "caption"
                                size: 11
                                muted: true
                                text: shelfWin.shelfData.hasDevice
                                    ? shelfWin.shelfData.deviceName
                                    : qsTr("No phone connected")
                            }
                        }
                    }

                    Row {
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 2

                        IconButton {
                            iconPath: Icons.refresh
                            label: qsTr("Refresh Shelf")
                            onClicked: AppController.refreshShelf()
                        }

                        IconButton {
                            iconPath: Icons.close
                            label: qsTr("Close Shelf")
                            onClicked: AppController.closeShelf()
                        }
                    }
                }

                // ---- Drop Zone Banner ----
                Rectangle {
                    id: dropBanner
                    width: parent.width
                    height: 52
                    radius: Theme.radiusMd
                    color: shelfDropArea.containsDrag
                        ? Theme.primaryContainer
                        : (shelfWin.justDropped ? Theme.secondaryContainer : Theme.surfaceContainerLow)
                    border.width: 1
                    border.color: shelfDropArea.containsDrag ? Theme.primary : Theme.outlineVariant

                    Accessible.role: Accessible.StaticText
                    Accessible.name: dropLabel.text

                    Behavior on color {
                        enabled: !Theme.reduceMotion
                        ColorAnimation { duration: Theme.fadeFast }
                    }

                    Row {
                        anchors.centerIn: parent
                        spacing: 8

                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 16
                            height: 16
                            path: shelfWin.justDropped ? Icons.check : Icons.send
                            color: shelfDropArea.containsDrag
                                ? Theme.primaryContainerContent
                                : (shelfWin.justDropped ? Theme.secondaryContainerContent : Theme.primary)
                        }

                        Txt {
                            id: dropLabel
                            anchors.verticalCenter: parent.verticalCenter
                            role: "bodySmall"
                            weight: 600
                            color: shelfDropArea.containsDrag
                                ? Theme.primaryContainerContent
                                : (shelfWin.justDropped ? Theme.secondaryContainerContent : Theme.surfaceContent)
                            text: shelfDropArea.containsDrag
                                ? qsTr("Release to send to phone")
                                : (shelfWin.justDropped
                                    ? qsTr("Sent to phone")
                                    : qsTr("Drop files, photos, or text here to send"))
                        }
                    }
                }

                // ---- Scrollable Shelf Content ----
                Flickable {
                    id: contentFlick
                    width: parent.width
                    height: parent.height - 36 - 52 - 20
                    contentWidth: width
                    contentHeight: cardsColumn.implicitHeight
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    Column {
                        id: cardsColumn
                        width: parent.width
                        spacing: 10

                        // ---- Camera Roll Row: Latest Photo & Latest Screenshot ----
                        Row {
                            width: parent.width
                            spacing: 8

                            Repeater {
                                model: [
                                    {
                                        slotKind: "photo",
                                        title: qsTr("Latest photo"),
                                        emptyText: qsTr("No recent photo"),
                                        icon: Icons.photo,
                                        item: shelfWin.shelfData.latestPhoto
                                    },
                                    {
                                        slotKind: "screenshot",
                                        title: qsTr("Latest screenshot"),
                                        emptyText: qsTr("No recent screenshot"),
                                        icon: Icons.camera,
                                        item: shelfWin.shelfData.latestScreenshot
                                    }
                                ]

                                delegate: Rectangle {
                                    id: mediaCard
                                    readonly property var itemData: modelData.item
                                    readonly property bool hasItem: itemData !== null && itemData !== undefined
                                    readonly property string thumbSource: hasItem ? (itemData.thumbUrl || itemData.thumbnailDataUrl || "") : ""
                                    readonly property string targetRef: hasItem ? (itemData.localPath || itemData.id || "") : ""

                                    width: Math.floor((cardsColumn.width - 8) / 2)
                                    height: 152
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainer
                                    border.width: Theme.graphite ? 1 : 0
                                    border.color: Theme.outlineVariant

                                    Column {
                                        anchors.fill: parent
                                        anchors.margins: 8
                                        spacing: 6

                                        // Card header
                                        Row {
                                            width: parent.width
                                            spacing: 5

                                            Icon {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: 13
                                                height: 13
                                                path: modelData.icon
                                                color: Theme.surfaceContentVariant
                                            }

                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: parent.width - 18
                                                role: "caption"
                                                weight: 600
                                                muted: true
                                                text: modelData.title
                                            }
                                        }

                                        // Thumbnail or empty state
                                        Item {
                                            width: parent.width
                                            height: 86

                                            Rectangle {
                                                anchors.fill: parent
                                                radius: Theme.radiusSm
                                                color: Theme.surfaceContainerHigh

                                                Txt {
                                                    anchors.centerIn: parent
                                                    width: parent.width - 12
                                                    horizontalAlignment: Text.AlignHCenter
                                                    wrapMode: Text.Wrap
                                                    role: "caption"
                                                    muted: true
                                                    visible: !mediaCard.hasItem
                                                    text: modelData.emptyText
                                                }
                                            }

                                            RoundedImage {
                                                anchors.fill: parent
                                                radius: Theme.radiusSm
                                                visible: mediaCard.hasItem && mediaCard.thumbSource !== ""
                                                source: mediaCard.thumbSource
                                            }

                                            MouseArea {
                                                id: mediaDragArea
                                                anchors.fill: parent
                                                enabled: mediaCard.hasItem
                                                hoverEnabled: true
                                                cursorShape: enabled ? Qt.OpenHandCursor : Qt.ArrowCursor

                                                property real pressX: 0
                                                property real pressY: 0
                                                property bool dragged: false

                                                Accessible.role: Accessible.Button
                                                Accessible.name: mediaCard.hasItem
                                                    ? (modelData.title + ": " + mediaCard.itemData.name)
                                                    : modelData.emptyText
                                                Accessible.onPressAction: {
                                                    if (mediaCard.hasItem)
                                                        AppController.activateShelfItem(modelData.slotKind, "open", mediaCard.targetRef)
                                                }

                                                onPressed: (mouse) => {
                                                    pressX = mouse.x
                                                    pressY = mouse.y
                                                    dragged = false
                                                }

                                                onPositionChanged: (mouse) => {
                                                    if (!pressed || dragged || !mediaCard.hasItem)
                                                        return
                                                    const dx = mouse.x - pressX
                                                    const dy = mouse.y - pressY
                                                    if (dx * dx + dy * dy > 36) {
                                                        dragged = true
                                                        AppController.startShelfDrag(
                                                            modelData.slotKind,
                                                            mediaCard.targetRef,
                                                            mediaCard.itemData.name
                                                        )
                                                    }
                                                }

                                                onClicked: {
                                                    if (!dragged && mediaCard.hasItem) {
                                                        AppController.activateShelfItem(
                                                            modelData.slotKind,
                                                            "open",
                                                            mediaCard.targetRef
                                                        )
                                                    }
                                                }
                                            }
                                        }

                                        // Bottom bar with filename + quick actions
                                        Item {
                                            width: parent.width
                                            height: 24

                                            Txt {
                                                anchors.left: parent.left
                                                anchors.right: mediaActions.left
                                                anchors.rightMargin: 4
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "caption"
                                                size: 11
                                                muted: !mediaCard.hasItem
                                                text: mediaCard.hasItem ? mediaCard.itemData.name : "—"
                                            }

                                            Row {
                                                id: mediaActions
                                                anchors.right: parent.right
                                                anchors.verticalCenter: parent.verticalCenter
                                                spacing: 2
                                                visible: mediaCard.hasItem

                                                IconButton {
                                                    width: 24
                                                    height: 24
                                                    iconPath: Icons.copy
                                                    label: qsTr("Copy")
                                                    onClicked: AppController.activateShelfItem(
                                                        modelData.slotKind,
                                                        "copy",
                                                        mediaCard.targetRef
                                                    )
                                                }

                                                IconButton {
                                                    width: 24
                                                    height: 24
                                                    iconPath: Icons.openExternal
                                                    label: qsTr("Open")
                                                    onClicked: AppController.activateShelfItem(
                                                        modelData.slotKind,
                                                        "open",
                                                        mediaCard.targetRef
                                                    )
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // ---- Current Clip Card ----
                        Rectangle {
                            id: clipCard
                            readonly property var clipData: shelfWin.shelfData.currentClip
                            readonly property bool hasClip: clipData !== null && clipData !== undefined
                            readonly property string clipId: hasClip ? (clipData.id || "") : ""

                            width: parent.width
                            implicitHeight: clipColumn.implicitHeight + 16
                            radius: Theme.radiusMd
                            color: Theme.surfaceContainer
                            border.width: Theme.graphite ? 1 : 0
                            border.color: Theme.outlineVariant

                            Column {
                                id: clipColumn
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: 8
                                spacing: 6

                                Item {
                                    width: parent.width
                                    height: 24

                                    Row {
                                        anchors.left: parent.left
                                        anchors.verticalCenter: parent.verticalCenter
                                        spacing: 6

                                        Icon {
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: 14
                                            height: 14
                                            path: Icons.clipboard
                                            color: Theme.surfaceContentVariant
                                        }

                                        Txt {
                                            anchors.verticalCenter: parent.verticalCenter
                                            role: "caption"
                                            weight: 600
                                            muted: true
                                            text: qsTr("Current clip")
                                        }
                                    }

                                    IconButton {
                                        anchors.right: parent.right
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 24
                                        height: 24
                                        visible: clipCard.hasClip
                                        iconPath: Icons.copy
                                        label: qsTr("Copy clip")
                                        onClicked: AppController.activateShelfItem("clip", "copy", clipCard.clipId)
                                    }
                                }

                                Rectangle {
                                    width: parent.width
                                    height: clipCard.hasClip && clipCard.clipData.kind === "image" ? 76 : 54
                                    radius: Theme.radiusSm
                                    color: Theme.surfaceContainerHigh

                                    RoundedImage {
                                        anchors.fill: parent
                                        anchors.margins: 4
                                        radius: Theme.radiusXs
                                        fillMode: Image.PreserveAspectFit
                                        visible: clipCard.hasClip && clipCard.clipData.kind === "image"
                                        source: clipCard.hasClip && clipCard.clipData.imageDataUrl
                                            ? clipCard.clipData.imageDataUrl : ""
                                    }

                                    Txt {
                                        anchors.fill: parent
                                        anchors.margins: 8
                                        visible: !clipCard.hasClip || clipCard.clipData.kind !== "image"
                                        role: clipCard.hasClip ? "code" : "caption"
                                        size: 12
                                        muted: !clipCard.hasClip
                                        wrapMode: Text.Wrap
                                        maximumLineCount: 2
                                        elide: Text.ElideRight
                                        text: clipCard.hasClip
                                            ? clipCard.clipData.text
                                            : qsTr("Clipboard is empty")
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        enabled: clipCard.hasClip
                                        hoverEnabled: true
                                        cursorShape: enabled ? Qt.OpenHandCursor : Qt.ArrowCursor

                                        property real pressX: 0
                                        property real pressY: 0
                                        property bool dragged: false

                                        Accessible.role: Accessible.Button
                                        Accessible.name: clipCard.hasClip
                                            ? qsTr("Current clip: %1").arg(clipCard.clipData.text)
                                            : qsTr("Clipboard is empty")
                                        Accessible.onPressAction: {
                                            if (clipCard.hasClip)
                                                AppController.activateShelfItem("clip", "copy", clipCard.clipId)
                                        }

                                        onPressed: (mouse) => {
                                            pressX = mouse.x
                                            pressY = mouse.y
                                            dragged = false
                                        }

                                        onPositionChanged: (mouse) => {
                                            if (!pressed || dragged || !clipCard.hasClip)
                                                return
                                            const dx = mouse.x - pressX
                                            const dy = mouse.y - pressY
                                            if (dx * dx + dy * dy > 36) {
                                                dragged = true
                                                AppController.startShelfDrag(
                                                    "clip",
                                                    clipCard.clipId,
                                                    clipCard.clipData.text || ""
                                                )
                                            }
                                        }

                                        onClicked: {
                                            if (!dragged && clipCard.hasClip) {
                                                AppController.activateShelfItem("clip", "copy", clipCard.clipId)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // ---- Recent Received Files Card ----
                        Rectangle {
                            id: filesCard
                            readonly property var filesList: shelfWin.shelfData.recentFiles || []

                            width: parent.width
                            implicitHeight: filesColumn.implicitHeight + 16
                            radius: Theme.radiusMd
                            color: Theme.surfaceContainer
                            border.width: Theme.graphite ? 1 : 0
                            border.color: Theme.outlineVariant

                            Column {
                                id: filesColumn
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: 8
                                spacing: 6

                                Row {
                                    width: parent.width
                                    spacing: 6

                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 14
                                        height: 14
                                        path: Icons.download
                                        color: Theme.surfaceContentVariant
                                    }

                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        role: "caption"
                                        weight: 600
                                        muted: true
                                        text: qsTr("Recent received files")
                                    }
                                }

                                Rectangle {
                                    width: parent.width
                                    height: 40
                                    radius: Theme.radiusSm
                                    color: Theme.surfaceContainerHigh
                                    visible: filesCard.filesList.length === 0

                                    Txt {
                                        anchors.centerIn: parent
                                        role: "caption"
                                        muted: true
                                        text: qsTr("No received files yet")
                                    }
                                }

                                Repeater {
                                    model: filesCard.filesList

                                    delegate: Rectangle {
                                        width: filesColumn.width
                                        height: 44
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHigh

                                        MouseArea {
                                            id: fileRowDrag
                                            anchors.left: parent.left
                                            anchors.top: parent.top
                                            anchors.bottom: parent.bottom
                                            anchors.right: fileActions.left
                                            hoverEnabled: true
                                            cursorShape: Qt.OpenHandCursor

                                            property real pressX: 0
                                            property real pressY: 0
                                            property bool dragged: false

                                            Accessible.role: Accessible.Button
                                            Accessible.name: modelData.name
                                            Accessible.onPressAction: AppController.activateShelfItem("file", "open", modelData.path)

                                            onPressed: (mouse) => {
                                                pressX = mouse.x
                                                pressY = mouse.y
                                                dragged = false
                                            }

                                            onPositionChanged: (mouse) => {
                                                if (!pressed || dragged)
                                                    return
                                                const dx = mouse.x - pressX
                                                const dy = mouse.y - pressY
                                                if (dx * dx + dy * dy > 36) {
                                                    dragged = true
                                                    AppController.startShelfDrag("file", modelData.path, modelData.name)
                                                }
                                            }

                                            onClicked: {
                                                if (!dragged)
                                                    AppController.activateShelfItem("file", "open", modelData.path)
                                            }

                                            Row {
                                                anchors.fill: parent
                                                anchors.leftMargin: 8
                                                anchors.rightMargin: 4
                                                spacing: 8

                                                Icon {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    width: 16
                                                    height: 16
                                                    path: Icons.folder
                                                    color: Theme.primary
                                                }

                                                Column {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    width: parent.width - 28
                                                    spacing: 1

                                                    Txt {
                                                        width: parent.width
                                                        role: "bodySmall"
                                                        size: 12
                                                        weight: 600
                                                        text: modelData.name
                                                    }

                                                    Txt {
                                                        width: parent.width
                                                        role: "caption"
                                                        size: 10
                                                        muted: true
                                                        text: shelfWin.formatBytes(modelData.size)
                                                    }
                                                }
                                            }
                                        }

                                        Row {
                                            id: fileActions
                                            anchors.right: parent.right
                                            anchors.rightMargin: 4
                                            anchors.verticalCenter: parent.verticalCenter
                                            spacing: 2

                                            IconButton {
                                                width: 28
                                                height: 28
                                                iconPath: Icons.openExternal
                                                label: qsTr("Open file")
                                                onClicked: AppController.activateShelfItem("file", "open", modelData.path)
                                            }

                                            IconButton {
                                                width: 28
                                                height: 28
                                                iconPath: Icons.folder
                                                label: qsTr("Show in folder")
                                                onClicked: AppController.activateShelfItem("file", "reveal", modelData.path)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Txt {
                            width: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            role: "caption"
                            size: 11
                            muted: true
                            text: qsTr("Drag any card out to use, or drop files in to send")
                        }
                    }
                }
            }
        }
    }
}
