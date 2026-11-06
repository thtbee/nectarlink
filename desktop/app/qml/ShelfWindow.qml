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

    title: qsTr("NectarLink Shelf")
    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
    color: "transparent"
    visible: true

    readonly property int availW: Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920
    readonly property int availH: Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : 1080

    // The work area (the screen minus the taskbar, wherever the taskbar is). Qt only knows
    // its size, so Windows reports the position; refreshed when the work area changes.
    property var workArea: AppController.workArea()
    readonly property int workAreaKey: Screen.desktopAvailableWidth * 100000 + Screen.desktopAvailableHeight
    onWorkAreaKeyChanged: workArea = AppController.workArea()
    readonly property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real areaX: workArea && workArea.width ? workArea.x / dpr : Screen.virtualX
    readonly property real areaY: workArea && workArea.height ? workArea.y / dpr : Screen.virtualY
    readonly property real areaW: workArea && workArea.width ? workArea.width / dpr : availW
    readonly property real areaH: workArea && workArea.height ? workArea.height / dpr : availH

    width: 348
    height: Math.min(620, Math.max(420, availH - 48))
    x: areaX + areaW - width - 12
    y: areaY + Math.round((areaH - height) / 2)

    property bool justDropped: false
    property bool undoPending: false
    property bool undoCancelled: false
    property var pendingUrls: []
    property string pendingText: ""
    property string pendingLabel: ""
    /// An image clip waiting out the undo; it's sent from clipboard history.
    property string pendingClipId: ""
    property real undoProgress: 0.0
    property double undoStartedAt: 0

    readonly property string targetPhoneName: shelfData.hasDevice && shelfData.deviceName.length > 0
        ? shelfData.deviceName
        : qsTr("phone")

    function queueSendToPhone(urls, text, label, clipId) {
        const cleanUrls = urls ? urls : []
        const cleanText = text ? String(text) : ""
        const cleanClip = clipId ? String(clipId) : ""
        if (cleanUrls.length === 0 && cleanText.trim().length === 0 && cleanClip.length === 0)
            return
        if (!shelfData.hasDevice) {
            // Nothing to send to: the app says no phone is paired yet.
            AppController.dropToShelf(cleanUrls, cleanText)
            return
        }
        if (undoPending) {
            commitPendingSend()
        }
        pendingUrls = cleanUrls
        pendingText = cleanText
        pendingLabel = label ? String(label) : ""
        pendingClipId = cleanClip
        undoCancelled = false
        justDropped = false
        undoPending = true
        undoProgress = 1.0
        undoStartedAt = Date.now()
        if (Theme.reduceMotion) {
            undoStepTimer.restart()
        } else {
            undoProgressAnim.restart()
        }
        undoSendTimer.restart()
    }

    function commitPendingSend() {
        if (!undoPending)
            return
        undoSendTimer.stop()
        undoProgressAnim.stop()
        undoStepTimer.stop()
        const urls = pendingUrls
        const text = pendingText
        const clipId = pendingClipId
        undoPending = false
        pendingUrls = []
        pendingText = ""
        pendingLabel = ""
        pendingClipId = ""
        undoProgress = 0.0
        if (clipId.length > 0) {
            AppController.sendClipboardHistory(shelfData.deviceId, clipId)
            justDropped = true
            dropConfirmTimer.restart()
        } else if (urls.length > 0 || text.trim().length > 0) {
            AppController.dropToShelf(urls, text)
            justDropped = true
            dropConfirmTimer.restart()
        }
    }

    function cancelPendingSend() {
        if (!undoPending)
            return
        undoSendTimer.stop()
        undoProgressAnim.stop()
        undoStepTimer.stop()
        undoPending = false
        pendingUrls = []
        pendingText = ""
        pendingLabel = ""
        pendingClipId = ""
        undoProgress = 0.0
        justDropped = false
        undoCancelled = true
        dropConfirmTimer.restart()
    }

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
        const locale = Qt.locale()
        if (b < 1024)
            return qsTr("%1 B").arg(b)
        if (b < 1024 * 1024)
            return qsTr("%1 KB").arg(locale.toString(b / 1024, "f", 1))
        if (b < 1024 * 1024 * 1024)
            return qsTr("%1 MB").arg(locale.toString(b / (1024 * 1024), "f", 1))
        return qsTr("%1 GB").arg(locale.toString(b / (1024 * 1024 * 1024), "f", 2))
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
        } else if (undoPending) {
            commitPendingSend()
        }
    }

    // No keyboard shortcuts: the window never takes focus
    // (WindowDoesNotAcceptFocus), so window shortcuts could never fire, and
    // app-wide ones would take Ctrl+Z from the app you're working in.

    Timer {
        id: undoSendTimer
        interval: 3000
        repeat: false
        onTriggered: shelfWin.commitPendingSend()
    }

    NumberAnimation {
        id: undoProgressAnim
        target: shelfWin
        property: "undoProgress"
        from: 1.0
        to: 0.0
        duration: 3000
        easing.type: Easing.Linear
    }

    Timer {
        id: undoStepTimer
        interval: 250
        repeat: true
        onTriggered: shelfWin.undoProgress = Math.max(0, 1 - (Date.now() - shelfWin.undoStartedAt) / undoSendTimer.interval)
    }

    Timer {
        id: dropConfirmTimer
        interval: 1800
        onTriggered: {
            shelfWin.justDropped = false
            shelfWin.undoCancelled = false
        }
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

            // Whole-window drop target so dropping anywhere on the Shelf queues a send with 3s Undo.
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
                        shelfWin.queueSendToPhone(urls, text, "")
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

                    // Wayland ignores where we ask to put the window, so let it be dragged.
                    DragHandler {
                        target: null
                        enabled: Qt.platform.pluginName === "wayland"
                        onActiveChanged: if (active) shelfWin.startSystemMove()
                    }

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
                                path: Icons.shelf
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

                // ---- Drop Zone / Undo Banner ----
                Rectangle {
                    id: dropBanner
                    width: parent.width
                    height: 52
                    radius: Theme.radiusMd
                    clip: true
                    color: shelfDropArea.containsDrag || shelfWin.undoPending
                        ? Theme.primaryContainer
                        : (shelfWin.justDropped ? Theme.secondaryContainer : Theme.surfaceContainerLow)
                    border.width: 1
                    border.color: shelfDropArea.containsDrag || shelfWin.undoPending ? Theme.primary : Theme.outlineVariant

                    Accessible.role: Accessible.StaticText
                    Accessible.name: shelfWin.undoPending
                        ? undoBannerText.text
                        : dropLabel.text

                    Behavior on color {
                        enabled: !Theme.reduceMotion
                        ColorAnimation { duration: Theme.fadeFast }
                    }

                    // Normal drop / confirmation state
                    Row {
                        anchors.centerIn: parent
                        spacing: 8
                        visible: !shelfWin.undoPending

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
                                ? qsTr("Release to send to %1").arg(shelfWin.targetPhoneName)
                                : (shelfWin.justDropped
                                    ? (shelfWin.shelfData.online
                                        ? qsTr("Sent to %1").arg(shelfWin.targetPhoneName)
                                        : qsTr("Queued for %1").arg(shelfWin.targetPhoneName))
                                    : (shelfWin.undoCancelled
                                        ? qsTr("Send cancelled")
                                        : qsTr("Drop or flick items here to send")))
                        }
                    }

                    // 3-second "Sending to <phone>… [Undo]" bar
                    Item {
                        anchors.fill: parent
                        anchors.leftMargin: 12
                        anchors.rightMargin: 10
                        visible: shelfWin.undoPending

                        Row {
                            anchors.left: parent.left
                            anchors.right: undoBtn.left
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8

                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 15
                                height: 15
                                path: Icons.send
                                color: Theme.primaryContainerContent
                            }

                            Txt {
                                id: undoBannerText
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 23
                                role: "bodySmall"
                                weight: 600
                                elide: Text.ElideRight
                                color: Theme.primaryContainerContent
                                text: shelfWin.pendingLabel.length > 0
                                    ? qsTr("Sending %1 to %2…").arg(shelfWin.pendingLabel).arg(shelfWin.targetPhoneName)
                                    : qsTr("Sending to %1…").arg(shelfWin.targetPhoneName)
                            }
                        }

                        Button {
                            id: undoBtn
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            size: "sm"
                            text: qsTr("Undo")
                            onClicked: shelfWin.cancelPendingSend()
                        }
                    }

                    // Countdown progress bar along the bottom edge of the Undo pill
                    Rectangle {
                        anchors.left: parent.left
                        anchors.bottom: parent.bottom
                        height: 3
                        width: parent.width * Math.max(0, Math.min(1, shelfWin.undoProgress))
                        color: Theme.primary
                        visible: shelfWin.undoPending
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
                                    // Only a file on this PC can be sent: a photo still on the phone
                                    // has just a thumbnail here, which isn't the picture.
                                    readonly property string sendUrl: hasItem && itemData.localPath ? itemData.localPath : ""

                                    property real flickOffsetX: 0
                                    property real flickOffsetY: 0

                                    width: Math.floor((cardsColumn.width - 8) / 2)
                                    height: 152
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainer
                                    border.width: mediaDragArea.flicking ? 1.5 : (Theme.graphite ? 1 : 0)
                                    border.color: mediaDragArea.flicking ? Theme.primary : Theme.outlineVariant
                                    z: (flickOffsetY !== 0 || mediaDragArea.flicking) ? 10 : 0

                                    transform: Translate {
                                        x: mediaCard.flickOffsetX
                                        y: mediaCard.flickOffsetY
                                    }

                                    Behavior on flickOffsetX {
                                        enabled: !Theme.reduceMotion && !mediaDragArea.pressed
                                        SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                                    }

                                    Behavior on flickOffsetY {
                                        enabled: !Theme.reduceMotion && !mediaDragArea.pressed && !mediaFlickAnim.running
                                        SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                                    }

                                    SequentialAnimation {
                                        id: mediaFlickAnim
                                        NumberAnimation {
                                            target: mediaCard
                                            property: "flickOffsetY"
                                            to: -54
                                            duration: Theme.reduceMotion ? 0 : 140
                                            easing.type: Easing.OutBack
                                        }
                                        SpringAnimation {
                                            target: mediaCard
                                            property: "flickOffsetY"
                                            to: 0
                                            spring: 4.5
                                            damping: 0.35
                                            epsilon: 0.25
                                        }
                                    }

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
                                            height: mediaCard.hasItem ? 86 : 116

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
                                                property real lastY: 0
                                                property double lastTime: 0
                                                property real velY: 0
                                                property bool dragged: false
                                                property bool flicking: false

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
                                                    lastY = mouse.y
                                                    lastTime = Date.now()
                                                    velY = 0
                                                    dragged = false
                                                    flicking = false
                                                }

                                                onPositionChanged: (mouse) => {
                                                    if (!pressed || dragged || !mediaCard.hasItem)
                                                        return
                                                    const dx = mouse.x - pressX
                                                    const dy = mouse.y - pressY
                                                    const now = Date.now()
                                                    const dt = Math.max(1, now - lastTime)
                                                    velY = ((mouse.y - lastY) * 1000.0) / dt
                                                    lastY = mouse.y
                                                    lastTime = now

                                                    if (dy < -8 && Math.abs(dy) >= Math.abs(dx) * 0.75) {
                                                        flicking = true
                                                        mediaCard.flickOffsetX = dx * 0.2
                                                        mediaCard.flickOffsetY = Math.max(-68, dy * 0.65)
                                                    } else if (!flicking && (dx * dx + dy * dy > 64)) {
                                                        dragged = true
                                                        mediaCard.flickOffsetX = 0
                                                        mediaCard.flickOffsetY = 0
                                                        AppController.startShelfDrag(
                                                            modelData.slotKind,
                                                            mediaCard.targetRef,
                                                            mediaCard.itemData.name
                                                        )
                                                    }
                                                }

                                                onReleased: (mouse) => {
                                                    if (!flicking)
                                                        return
                                                    const dy = mouse.y - pressY
                                                    const shouldSend = (dy < -24 && velY < -280) || (dy < -48)
                                                    flicking = false
                                                    dragged = true
                                                    mediaCard.flickOffsetX = 0
                                                    if (shouldSend && mediaCard.sendUrl.length > 0) {
                                                        mediaFlickAnim.restart()
                                                        shelfWin.queueSendToPhone([mediaCard.sendUrl], "", mediaCard.itemData.name)
                                                    } else {
                                                        mediaCard.flickOffsetY = 0
                                                    }
                                                }

                                                onCanceled: {
                                                    flicking = false
                                                    mediaCard.flickOffsetX = 0
                                                    mediaCard.flickOffsetY = 0
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
                                            visible: mediaCard.hasItem

                                            Txt {
                                                anchors.left: parent.left
                                                anchors.right: mediaActions.left
                                                anchors.rightMargin: 4
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "caption"
                                                size: 11
                                                visible: mediaCard.hasItem
                                                text: mediaCard.hasItem ? mediaCard.itemData.name : ""
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

                            property real flickOffsetX: 0
                            property real flickOffsetY: 0

                            width: parent.width
                            implicitHeight: clipColumn.implicitHeight + 16
                            radius: Theme.radiusMd
                            color: Theme.surfaceContainer
                            border.width: clipDragArea.flicking ? 1.5 : (Theme.graphite ? 1 : 0)
                            border.color: clipDragArea.flicking ? Theme.primary : Theme.outlineVariant
                            z: (flickOffsetY !== 0 || clipDragArea.flicking) ? 10 : 0

                            transform: Translate {
                                x: clipCard.flickOffsetX
                                y: clipCard.flickOffsetY
                            }

                            Behavior on flickOffsetX {
                                enabled: !Theme.reduceMotion && !clipDragArea.pressed
                                SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                            }

                            Behavior on flickOffsetY {
                                enabled: !Theme.reduceMotion && !clipDragArea.pressed && !clipFlickAnim.running
                                SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                            }

                            SequentialAnimation {
                                id: clipFlickAnim
                                NumberAnimation {
                                    target: clipCard
                                    property: "flickOffsetY"
                                    to: -54
                                    duration: Theme.reduceMotion ? 0 : 140
                                    easing.type: Easing.OutBack
                                }
                                SpringAnimation {
                                    target: clipCard
                                    property: "flickOffsetY"
                                    to: 0
                                    spring: 4.5
                                    damping: 0.35
                                    epsilon: 0.25
                                }
                            }

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

                                    Row {
                                        anchors.right: parent.right
                                        anchors.verticalCenter: parent.verticalCenter
                                        spacing: 2
                                        visible: clipCard.hasClip

                                        IconButton {
                                            width: 24
                                            height: 24
                                            visible: clipCard.hasClip
                                                && ((clipCard.clipData.text || "").length > 0
                                                    || (clipCard.clipData.kind === "image" && clipCard.clipId.length > 0))
                                            iconPath: Icons.send
                                            label: qsTr("Send clip to phone")
                                            onClicked: {
                                                clipFlickAnim.restart()
                                                if (clipCard.clipData.kind === "image")
                                                    shelfWin.queueSendToPhone([], "", qsTr("the copied image"), clipCard.clipId)
                                                else
                                                    shelfWin.queueSendToPhone([], clipCard.clipData.text || "", qsTr("the copied text"))
                                            }
                                        }

                                        IconButton {
                                            width: 24
                                            height: 24
                                            iconPath: Icons.copy
                                            label: qsTr("Copy clip")
                                            onClicked: AppController.activateShelfItem("clip", "copy", clipCard.clipId)
                                        }
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
                                        id: clipDragArea
                                        anchors.fill: parent
                                        enabled: clipCard.hasClip
                                        hoverEnabled: true
                                        cursorShape: enabled ? Qt.OpenHandCursor : Qt.ArrowCursor

                                        property real pressX: 0
                                        property real pressY: 0
                                        property real lastY: 0
                                        property double lastTime: 0
                                        property real velY: 0
                                        property bool dragged: false
                                        property bool flicking: false

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
                                            lastY = mouse.y
                                            lastTime = Date.now()
                                            velY = 0
                                            dragged = false
                                            flicking = false
                                        }

                                        onPositionChanged: (mouse) => {
                                            if (!pressed || dragged || !clipCard.hasClip)
                                                return
                                            const dx = mouse.x - pressX
                                            const dy = mouse.y - pressY
                                            const now = Date.now()
                                            const dt = Math.max(1, now - lastTime)
                                            velY = ((mouse.y - lastY) * 1000.0) / dt
                                            lastY = mouse.y
                                            lastTime = now

                                            if (dy < -8 && Math.abs(dy) >= Math.abs(dx) * 0.75) {
                                                flicking = true
                                                clipCard.flickOffsetX = dx * 0.2
                                                clipCard.flickOffsetY = Math.max(-68, dy * 0.65)
                                            } else if (!flicking && (dx * dx + dy * dy > 64)) {
                                                dragged = true
                                                clipCard.flickOffsetX = 0
                                                clipCard.flickOffsetY = 0
                                                AppController.startShelfDrag(
                                                    "clip",
                                                    clipCard.clipId,
                                                    clipCard.clipData.text || ""
                                                )
                                            }
                                        }

                                        onReleased: (mouse) => {
                                            if (!flicking)
                                                return
                                            const dy = mouse.y - pressY
                                            const shouldSend = (dy < -24 && velY < -280) || (dy < -48)
                                            flicking = false
                                            dragged = true
                                            clipCard.flickOffsetX = 0
                                            const isImage = clipCard.clipData.kind === "image"
                                            if (shouldSend && isImage && clipCard.clipId.length > 0) {
                                                clipFlickAnim.restart()
                                                shelfWin.queueSendToPhone([], "", qsTr("the copied image"), clipCard.clipId)
                                            } else if (shouldSend && (clipCard.clipData.text || "").length > 0) {
                                                clipFlickAnim.restart()
                                                shelfWin.queueSendToPhone([], clipCard.clipData.text || "", qsTr("the copied text"))
                                            } else {
                                                clipCard.flickOffsetY = 0
                                            }
                                        }

                                        onCanceled: {
                                            flicking = false
                                            clipCard.flickOffsetX = 0
                                            clipCard.flickOffsetY = 0
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
                                        id: fileRowCard
                                        width: filesColumn.width
                                        height: 44
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHigh
                                        border.width: fileRowDrag.flicking ? 1.5 : 0
                                        border.color: Theme.primary
                                        z: (flickOffsetY !== 0 || fileRowDrag.flicking) ? 10 : 0

                                        property real flickOffsetX: 0
                                        property real flickOffsetY: 0

                                        transform: Translate {
                                            x: fileRowCard.flickOffsetX
                                            y: fileRowCard.flickOffsetY
                                        }

                                        Behavior on flickOffsetX {
                                            enabled: !Theme.reduceMotion && !fileRowDrag.pressed
                                            SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                                        }

                                        Behavior on flickOffsetY {
                                            enabled: !Theme.reduceMotion && !fileRowDrag.pressed && !fileFlickAnim.running
                                            SpringAnimation { spring: 4.5; damping: 0.35; epsilon: 0.25 }
                                        }

                                        SequentialAnimation {
                                            id: fileFlickAnim
                                            NumberAnimation {
                                                target: fileRowCard
                                                property: "flickOffsetY"
                                                to: -54
                                                duration: Theme.reduceMotion ? 0 : 140
                                                easing.type: Easing.OutBack
                                            }
                                            SpringAnimation {
                                                target: fileRowCard
                                                property: "flickOffsetY"
                                                to: 0
                                                spring: 4.5
                                                damping: 0.35
                                                epsilon: 0.25
                                            }
                                        }

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
                                            property real lastY: 0
                                            property double lastTime: 0
                                            property real velY: 0
                                            property bool dragged: false
                                            property bool flicking: false

                                            Accessible.role: Accessible.Button
                                            Accessible.name: modelData.name
                                            Accessible.onPressAction: AppController.activateShelfItem("file", "open", modelData.path)

                                            onPressed: (mouse) => {
                                                pressX = mouse.x
                                                pressY = mouse.y
                                                lastY = mouse.y
                                                lastTime = Date.now()
                                                velY = 0
                                                dragged = false
                                                flicking = false
                                            }

                                            onPositionChanged: (mouse) => {
                                                if (!pressed || dragged)
                                                    return
                                                const dx = mouse.x - pressX
                                                const dy = mouse.y - pressY
                                                const now = Date.now()
                                                const dt = Math.max(1, now - lastTime)
                                                velY = ((mouse.y - lastY) * 1000.0) / dt
                                                lastY = mouse.y
                                                lastTime = now

                                                if (dy < -8 && Math.abs(dy) >= Math.abs(dx) * 0.75) {
                                                    flicking = true
                                                    fileRowCard.flickOffsetX = dx * 0.2
                                                    fileRowCard.flickOffsetY = Math.max(-68, dy * 0.65)
                                                } else if (!flicking && (dx * dx + dy * dy > 64)) {
                                                    dragged = true
                                                    fileRowCard.flickOffsetX = 0
                                                    fileRowCard.flickOffsetY = 0
                                                    AppController.startShelfDrag("file", modelData.path, modelData.name)
                                                }
                                            }

                                            onReleased: (mouse) => {
                                                if (!flicking)
                                                    return
                                                const dy = mouse.y - pressY
                                                const shouldSend = (dy < -24 && velY < -280) || (dy < -48)
                                                flicking = false
                                                dragged = true
                                                fileRowCard.flickOffsetX = 0
                                                if (shouldSend && modelData.path && modelData.path.length > 0) {
                                                    fileFlickAnim.restart()
                                                    shelfWin.queueSendToPhone([modelData.path], "", modelData.name)
                                                } else {
                                                    fileRowCard.flickOffsetY = 0
                                                }
                                            }

                                            onCanceled: {
                                                flicking = false
                                                fileRowCard.flickOffsetX = 0
                                                fileRowCard.flickOffsetY = 0
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
                                                iconPath: Icons.send
                                                label: qsTr("Send to phone")
                                                onClicked: {
                                                    fileFlickAnim.restart()
                                                    shelfWin.queueSendToPhone([modelData.path], "", modelData.name)
                                                }
                                            }

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
                            text: qsTr("Flick any card up to send to phone, drag out to use, or drop files in")
                        }
                    }
                }
            }
        }
    }
}
