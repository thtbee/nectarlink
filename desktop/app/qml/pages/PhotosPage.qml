// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Photos: the phone's gallery on the PC. Albums on the left, a virtualized
// GridView of thumbnails grouped by day on the right, multi-select, Save (to
// a chosen folder or Downloads\Nectarlink), Copy to clipboard, Open in the
// Windows Photos app, and an in-app viewer with arrow-key navigation.
Item {
    id: page
    property bool active: false
    property string deviceId
    property string deviceName

    readonly property var albums: { try { return JSON.parse(Photos.albums) } catch (e) { return [] } }
    readonly property var photosFeature: AppController.capsRevision >= 0 && page.deviceId.length > 0
        ? AppController.featureState(page.deviceId, "files.recent_photos") : ({})

    // Multi-select state: map of itemId -> true, and ordered array of selected IDs.
    property var selectedMap: ({})
    property var selectedIds: []
    readonly property int selectedCount: selectedIds.length
    property bool selectMode: false
    property int lastClickedIndex: -1

    // In-app viewer index (-1 when closed).
    property int viewerIndex: -1
    readonly property var viewedItem: {
        if (Photos.revision < 0 || viewerIndex < 0 || viewerIndex >= Photos.count)
            return null
        try { return JSON.parse(Photos.itemAt(viewerIndex)) } catch (e) { return null }
    }

    // Whether the single selected item is a video (Copy is for still images).
    readonly property bool singleSelectedIsVideo: {
        if (selectedCount !== 1 || Photos.revision < 0) return false
        const id = selectedIds[0]
        return id.indexOf("video:") === 0
    }

    readonly property string currentAlbumName: {
        if (Photos.album.length === 0) return qsTr("All photos")
        for (let i = 0; i < albums.length; i++) {
            if (albums[i].id === Photos.album) return albums[i].name
        }
        return qsTr("Album")
    }

    readonly property int totalPhotosCount: {
        let sum = 0
        for (let i = 0; i < albums.length; i++)
            sum += (albums[i].count || 0)
        return sum > 0 ? sum : Photos.count
    }

    function sameDay(a, b) {
        if (!a || !b || Math.abs(a - b) >= 86400000) return false
        const da = new Date(a)
        const db = new Date(b)
        return da.getDate() === db.getDate()
            && da.getMonth() === db.getMonth()
            && da.getFullYear() === db.getFullYear()
    }

    function dayLabel(ms) {
        if (!ms || ms <= 0) return ""
        const date = new Date(ms)
        const today = new Date()
        const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
        if (date.toDateString() === today.toDateString()) return qsTr("Today")
        if (date.toDateString() === yesterday.toDateString()) return qsTr("Yesterday")
        return date.toLocaleDateString(
            Qt.locale(),
            date.getFullYear() === today.getFullYear() ? "dddd, d MMMM" : Locale.LongFormat
        )
    }

    function shortDayLabel(ms) {
        if (!ms || ms <= 0) return ""
        const date = new Date(ms)
        const today = new Date()
        const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
        if (date.toDateString() === today.toDateString()) return qsTr("Today")
        if (date.toDateString() === yesterday.toDateString()) return qsTr("Yesterday")
        return date.toLocaleDateString(
            Qt.locale(),
            date.getFullYear() === today.getFullYear() ? "d MMM" : "d MMM yyyy"
        )
    }

    function formatVideoDuration(ms) {
        if (!ms || ms <= 0) return "0:00"
        const totalSec = Math.max(1, Math.round(ms / 1000))
        const h = Math.floor(totalSec / 3600)
        const m = Math.floor((totalSec % 3600) / 60)
        const s = totalSec % 60
        if (h > 0)
            return h + ":" + String(m).padStart(2, "0") + ":" + String(s).padStart(2, "0")
        return m + ":" + String(s).padStart(2, "0")
    }

    function formatBytes(bytes) {
        if (!bytes || bytes <= 0) return ""
        if (bytes < 1024) return qsTr("%1 B").arg(Math.round(bytes))
        const units = [qsTr("%1 KB"), qsTr("%1 MB"), qsTr("%1 GB"), qsTr("%1 TB")]
        let value = bytes / 1024
        let unit = 0
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++ }
        return units[unit].arg(unit === 0 ? Math.round(value) : (value < 10 ? value.toFixed(1) : Math.round(value)))
    }

    function isSelected(id) {
        return !!selectedMap[id]
    }

    function toggleSelect(id, index, shiftHeld) {
        const next = Object.assign({}, selectedMap)
        if (shiftHeld && lastClickedIndex >= 0 && index >= 0 && Photos.count > 0) {
            const from = Math.min(lastClickedIndex, index)
            const to = Math.max(lastClickedIndex, index)
            for (let r = from; r <= to; r++) {
                const rowId = Photos.idAt(r)
                if (rowId.length > 0) next[rowId] = true
            }
        } else if (next[id]) {
            delete next[id]
        } else {
            next[id] = true
        }
        if (index >= 0) lastClickedIndex = index
        selectedMap = next
        selectedIds = Object.keys(next)
    }

    function selectAllLoaded() {
        const next = {}
        const n = Math.min(Photos.count, 500)
        for (let r = 0; r < n; r++) {
            const rowId = Photos.idAt(r)
            if (rowId.length > 0) next[rowId] = true
        }
        selectedMap = next
        selectedIds = Object.keys(next)
        selectMode = true
    }

    function clearSelection() {
        selectedMap = ({})
        selectedIds = []
        selectMode = false
        lastClickedIndex = -1
    }

    function openViewer(index) {
        if (index < 0 || index >= Photos.count) return
        viewerIndex = index
        try {
            const item = JSON.parse(Photos.itemAt(index))
            if (item && !item.isVideo) {
                Photos.ensureFull(item.id)
            }
        } catch (e) {}
        if (index + 6 >= Photos.count && Photos.more) {
            Photos.loadOlder()
        }
    }

    function stepViewer(delta) {
        const next = viewerIndex + delta
        if (next >= 0 && next < Photos.count) {
            openViewer(next)
        }
    }

    function load() {
        if (active && deviceId.length > 0)
            Photos.open(deviceId)
    }

    onActiveChanged: if (active) load()
    Component.onCompleted: load()
    onDeviceIdChanged: {
        clearSelection()
        viewerIndex = -1
        load()
    }

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    // Keyboard shortcuts for viewer navigation and selection.
    Shortcut {
        sequence: "Left"
        enabled: page.active && page.viewerIndex > 0
        onActivated: page.stepViewer(-1)
    }
    Shortcut {
        sequence: "Right"
        enabled: page.active && page.viewerIndex >= 0 && page.viewerIndex + 1 < Photos.count
        onActivated: page.stepViewer(1)
    }
    Shortcut {
        sequence: "Esc"
        enabled: page.active && (page.viewerIndex >= 0 || page.selectedCount > 0 || page.selectMode)
        onActivated: {
            if (page.viewerIndex >= 0)
                page.viewerIndex = -1
            else
                page.clearSelection()
        }
    }
    Shortcut {
        sequence: StandardKey.Copy
        enabled: page.active && (
            (page.viewerIndex >= 0 && page.viewedItem !== null && !page.viewedItem.isVideo)
            || (page.selectedCount === 1 && !page.singleSelectedIsVideo)
        )
        onActivated: {
            if (page.viewerIndex >= 0 && page.viewedItem)
                Photos.copyItem(page.viewedItem.id)
            else if (page.selectedCount === 1)
                Photos.copyItem(page.selectedIds[0])
        }
    }

    FolderDialog {
        id: defaultFolderPicker
        title: qsTr("Choose default folder for saved photos")
        onAccepted: Photos.chooseSaveFolder(selectedFolder.toString())
    }

    FolderDialog {
        id: saveToFolderPicker
        property var pendingIds: []
        title: qsTr("Save to folder")
        onAccepted: {
            if (pendingIds.length > 0) {
                Photos.saveItems(pendingIds, selectedFolder.toString())
                if (page.viewerIndex < 0)
                    page.clearSelection()
            }
        }
    }

    readonly property bool phoneOffline: page.deviceId.length === 0
        || (Photos.status === "offline" && Photos.count === 0 && page.albums.length === 0)

    // ---- No phone paired, or phone offline with nothing cached ----
    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.contentPadding * 2, 420)
        spacing: 12
        visible: page.phoneOffline
        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 40; height: 40
            stroke: 1.5
            path: Icons.photo
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: page.deviceId.length === 0 ? qsTr("Your phone's photos on your PC")
                : qsTr("%1 isn't connected").arg(page.deviceName)
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: page.deviceId.length === 0
                ? qsTr("Pair your phone to browse its camera roll, screenshots, and albums right from here.")
                : qsTr("Your photos and albums show up here when it's back.")
        }
    }

    // ---- Left pane: Albums list ----
    Item {
        id: albumPane
        visible: !page.phoneOffline
        width: page.width < 760 ? 200 : 248
        height: parent.height

        Item {
            id: albumHeader
            width: parent.width
            height: 56
            Txt {
                anchors.left: parent.left
                anchors.leftMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                role: "label"
                muted: true
                text: qsTr("Albums")
            }
            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: Photos.status !== "loading"
                onClicked: Photos.refresh()
            }
        }

        Flickable {
            anchors.top: albumHeader.bottom
            anchors.bottom: parent.bottom
            width: parent.width
            contentHeight: albumCol.height + 16
            boundsBehavior: Flickable.StopAtBounds
            clip: true

            Column {
                id: albumCol
                width: parent.width
                spacing: 2

                AlbumRow {
                    width: albumCol.width
                    albumId: ""
                    title: qsTr("All photos")
                    count: page.totalPhotosCount
                    thumbUrl: page.albums.length > 0 ? (page.albums[0].thumb || "") : ""
                    selected: Photos.album === ""
                    onClicked: {
                        page.clearSelection()
                        page.viewerIndex = -1
                        Photos.selectAlbum("")
                    }
                }

                Repeater {
                    model: page.albums
                    delegate: AlbumRow {
                        required property var modelData
                        width: albumCol.width
                        albumId: modelData.id
                        title: modelData.name
                        count: modelData.count || 0
                        thumbUrl: modelData.thumb || ""
                        selected: Photos.album === modelData.id
                        onClicked: {
                            page.clearSelection()
                            page.viewerIndex = -1
                            Photos.selectAlbum(modelData.id)
                        }
                    }
                }
            }
        }

        Divider { anchors.right: parent.right; width: 1; height: parent.height }
    }

    // ---- Right pane: Toolbar + Virtualized GridView ----
    Item {
        id: gridPane
        visible: !page.phoneOffline
        anchors.left: albumPane.right
        anchors.right: parent.right
        height: parent.height
        readonly property bool compactBar: width < 540

        // Top toolbar: album info OR multi-select actions.
        Item {
            id: toolbar
            width: parent.width
            height: 56

            // Normal mode header (no selection active).
            Row {
                id: normalLeftRow
                anchors.left: parent.left
                anchors.leftMargin: 20
                anchors.right: normalRightRow.visible ? normalRightRow.left : parent.right
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                spacing: 10
                visible: page.selectedCount === 0 && !page.selectMode
                clip: true

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    role: "title"
                    size: 16
                    elide: Text.ElideRight
                    text: page.currentAlbumName
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: Photos.count > 0
                    role: "caption"
                    muted: true
                    text: Photos.more
                        ? qsTr("%1+ items").arg(Photos.count)
                        : (Photos.count === 1 ? qsTr("1 item") : qsTr("%1 items").arg(Photos.count))
                }
            }

            Row {
                id: normalRightRow
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: page.selectedCount === 0 && !page.selectMode

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: AppController.continuityCameraBusy
                    role: "caption"
                    muted: true
                    text: AppController.continuityCameraStatus
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: AppController.continuityCameraBusy
                    variant: "outline"
                    size: "sm"
                    iconPath: Icons.close
                    text: qsTr("Cancel")
                    onClicked: AppController.cancelContinuityCamera()
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !AppController.continuityCameraBusy
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.camera
                    text: gridPane.compactBar ? qsTr("Photo") : qsTr("Take photo")
                    onClicked: AppController.startContinuityCamera("photo")
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !AppController.continuityCameraBusy
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.clipboardList
                    text: gridPane.compactBar ? qsTr("Scan") : qsTr("Scan document")
                    onClicked: AppController.startContinuityCamera("scan")
                }

                Chip {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: Photos.count > 0
                    iconPath: Icons.folder
                    interactive: true
                    text: {
                        const f = Photos.saveFolder || ""
                        if (!gridPane.compactBar) return f
                        const parts = f.split(/[\\/]/)
                        return parts.length > 0 && parts[parts.length - 1].length > 0
                            ? parts[parts.length - 1] : f
                    }
                    onClicked: defaultFolderPicker.open()
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: Photos.count > 0
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.check
                    text: qsTr("Select")
                    onClicked: page.selectMode = true
                }
            }

            // Selection mode toolbar.
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: page.selectedCount > 0 || page.selectMode

                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.close
                    label: qsTr("Cancel selection")
                    onClicked: page.clearSelection()
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    role: "title"
                    size: 15
                    text: page.selectedCount > 0
                        ? qsTr("%1 selected").arg(page.selectedCount)
                        : (gridPane.compactBar ? qsTr("Select items") : qsTr("Click items to select"))
                }
                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !gridPane.compactBar || page.selectedCount === 0
                    variant: "text"
                    size: "sm"
                    text: qsTr("Select all")
                    onClicked: page.selectAllLoaded()
                }
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: page.selectedCount > 0 || page.selectMode

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: page.selectedCount === 1 && !page.singleSelectedIsVideo
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.copy
                    text: qsTr("Copy")
                    enabled: !Photos.saving
                    onClicked: Photos.copyItem(page.selectedIds[0])
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: page.selectedCount === 1
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.openExternal
                    text: qsTr("Open")
                    enabled: !Photos.saving
                    onClicked: Photos.openItem(page.selectedIds[0])
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    variant: "fill"
                    size: "sm"
                    iconPath: Icons.download
                    busy: Photos.saving
                    enabled: page.selectedCount > 0 && !Photos.saving
                    text: page.selectedCount > 1
                        ? qsTr("Save (%1)").arg(page.selectedCount)
                        : qsTr("Save")
                    onClicked: {
                        Photos.saveItems(page.selectedIds, "")
                        page.clearSelection()
                    }
                }

                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    tonal: true
                    iconPath: Icons.folder
                    label: qsTr("Save to folder…")
                    enabled: page.selectedCount > 0 && !Photos.saving
                    onClicked: {
                        saveToFolderPicker.pendingIds = page.selectedIds.slice()
                        saveToFolderPicker.open()
                    }
                }
            }

            Divider { anchors.bottom: parent.bottom; width: parent.width }
        }

        // Content area below toolbar.
        Item {
            id: gridContainer
            anchors.top: toolbar.bottom
            anchors.bottom: parent.bottom
            width: parent.width

            Spinner {
                anchors.centerIn: parent
                visible: Photos.status === "loading" && Photos.count === 0
            }

            // Permission / toggle / error state.
            Column {
                anchors.centerIn: parent
                width: Math.min(parent.width - 48, 400)
                spacing: 10
                visible: Photos.count === 0 && Photos.status !== "loading" && Photos.status !== "ready"
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 34; height: 34
                    stroke: 1.5
                    path: Icons.photo
                    color: Theme.surfaceContentVariant
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "title"
                    size: 16
                    text: Photos.status === "off" ? qsTr("Photos are off for %1").arg(page.deviceName)
                        : Photos.status === "unsupported" ? qsTr("Allow photos on %1").arg(page.deviceName)
                        : Photos.status === "offline" ? qsTr("%1 isn't connected").arg(page.deviceName)
                        : qsTr("Couldn't load photos")
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: Photos.status === "off"
                        ? qsTr("Turn on Photos for this phone, here in Settings and in the Nectarlink app on the phone.")
                        : Photos.status === "unsupported"
                          ? qsTr("In the Nectarlink app on your phone, tap Allow on “Photos and videos”.")
                          : Photos.status === "offline"
                            ? qsTr("Your photos and albums show up here when it's back.")
                            : qsTr("Something went wrong reading the photo library from the phone.")
                }
                LockChip {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: label.length > 0
                    feature: page.photosFeature
                }
                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: Photos.status === "failed"
                    variant: "tonal"
                    size: "sm"
                    text: qsTr("Try again")
                    onClicked: Photos.refresh()
                }
            }

            // Empty album / library state.
            Column {
                anchors.centerIn: parent
                width: Math.min(parent.width - 48, 360)
                spacing: 8
                visible: Photos.status === "ready" && Photos.count === 0
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 32; height: 32
                    stroke: 1.5
                    path: Icons.photo
                    color: Theme.surfaceContentVariant
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    role: "title"
                    size: 15
                    text: qsTr("No photos or videos")
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: qsTr("Photos and videos on your phone will appear here.")
                }
            }

            // Virtualized thumbnail grid grouped by day.
            GridView {
                id: grid
                anchors.fill: parent
                anchors.leftMargin: 16
                anchors.rightMargin: 8
                topMargin: 44
                bottomMargin: 24
                visible: Photos.count > 0
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                cacheBuffer: height
                readonly property int columns: Math.max(2, Math.floor((width - 8) / 164))
                cellWidth: Math.floor((width - 8) / columns)
                cellHeight: cellWidth
                model: Photos

                onAtYEndChanged: {
                    if (atYEnd && Photos.more && count > 0)
                        Photos.loadOlder()
                }

                // Current top-visible item's day label, shown in the sticky day banner.
                readonly property int topVisibleRow: Math.max(
                    0,
                    Math.floor(Math.max(0, contentY + 20) / Math.max(1, cellHeight)) * columns
                )
                readonly property string stickyDay: {
                    if (Photos.revision < 0 || Photos.count === 0) return ""
                    const row = Math.min(topVisibleRow, Photos.count - 1)
                    const ms = Photos.dateAt(row)
                    return ms > 0 ? page.dayLabel(ms) : ""
                }

                delegate: PhotoTile {
                    width: grid.cellWidth
                    height: grid.cellHeight
                }

                footer: Item {
                    width: grid.width
                    height: Photos.loadingOlder ? 52 : 0
                    Spinner {
                        anchors.centerIn: parent
                        visible: Photos.loadingOlder
                    }
                }
            }

            // Sticky day group banner at the top of the grid.
            Rectangle {
                anchors.top: parent.top
                anchors.topMargin: 8
                anchors.left: parent.left
                anchors.leftMargin: 20
                visible: Photos.count > 0 && grid.stickyDay.length > 0
                height: 28
                width: stickyDayTxt.implicitWidth + 24
                radius: Theme.pill(height)
                color: Qt.rgba(Theme.surfaceContainerHigh.r, Theme.surfaceContainerHigh.g, Theme.surfaceContainerHigh.b, 0.92)
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                Txt {
                    id: stickyDayTxt
                    anchors.centerIn: parent
                    role: "label"
                    size: 12
                    text: grid.stickyDay
                }
            }
        }
    }

    // ---- In-app Photo & Video Viewer Overlay ----
    Rectangle {
        id: viewer
        anchors.fill: parent
        visible: page.viewerIndex >= 0 && page.viewedItem !== null
        color: Theme.surface
        z: 10

        // Absorb background clicks.
        TapHandler { onTapped: {} }

        // Viewer top bar.
        Item {
            id: viewerBar
            width: parent.width
            height: 60

            IconButton {
                id: closeViewerBtn
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.back
                label: qsTr("Back to gallery")
                onClicked: page.viewerIndex = -1
            }

            Column {
                anchors.left: closeViewerBtn.right
                anchors.leftMargin: 12
                anchors.right: viewerActions.left
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                Txt {
                    width: parent.width
                    role: "title"
                    size: 15
                    elide: Text.ElideMiddle
                    text: page.viewedItem ? page.viewedItem.name : ""
                }
                Txt {
                    width: parent.width
                    role: "caption"
                    muted: true
                    elide: Text.ElideRight
                    text: {
                        if (!page.viewedItem) return ""
                        const parts = []
                        parts.push((page.viewerIndex + 1) + " / " + Photos.count)
                        if (page.viewedItem.date > 0) {
                            const d = new Date(page.viewedItem.date)
                            parts.push(page.dayLabel(page.viewedItem.date) + " " + d.toLocaleTimeString(Qt.locale(), Locale.ShortFormat))
                        }
                        if (page.viewedItem.width > 0 && page.viewedItem.height > 0)
                            parts.push(page.viewedItem.width + " × " + page.viewedItem.height)
                        if (page.viewedItem.size > 0)
                            parts.push(page.formatBytes(page.viewedItem.size))
                        return parts.join(" · ")
                    }
                }
            }

            Row {
                id: viewerActions
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: page.viewedItem !== null && !page.viewedItem.isVideo
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.copy
                    text: qsTr("Copy")
                    enabled: !Photos.saving
                    onClicked: if (page.viewedItem) Photos.copyItem(page.viewedItem.id)
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    variant: "tonal"
                    size: "sm"
                    iconPath: page.viewedItem && page.viewedItem.isVideo ? Icons.play : Icons.openExternal
                    text: page.viewedItem && page.viewedItem.isVideo ? qsTr("Play video") : qsTr("Open in Photos")
                    enabled: !Photos.saving
                    onClicked: if (page.viewedItem) Photos.openItem(page.viewedItem.id)
                }

                Button {
                    anchors.verticalCenter: parent.verticalCenter
                    variant: "fill"
                    size: "sm"
                    iconPath: Icons.download
                    busy: Photos.saving
                    enabled: !Photos.saving
                    text: qsTr("Save")
                    onClicked: if (page.viewedItem) Photos.saveItems([page.viewedItem.id], "")
                }

                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    tonal: true
                    iconPath: Icons.folder
                    label: qsTr("Save to folder…")
                    enabled: !Photos.saving
                    onClicked: {
                        if (page.viewedItem) {
                            saveToFolderPicker.pendingIds = [page.viewedItem.id]
                            saveToFolderPicker.open()
                        }
                    }
                }
            }

            Divider { anchors.bottom: parent.bottom; width: parent.width }
        }

        // Main photo/video stage.
        Item {
            anchors.top: viewerBar.bottom
            anchors.bottom: parent.bottom
            width: parent.width

            Image {
                id: stageImage
                anchors.fill: parent
                anchors.margins: 32
                asynchronous: true
                fillMode: Image.PreserveAspectFit
                smooth: true
                mipmap: true
                source: {
                    if (!page.viewedItem) return ""
                    if (page.viewedItem.fullUrl && page.viewedItem.fullUrl.length > 0)
                        return page.viewedItem.fullUrl
                    return page.viewedItem.thumb || ""
                }
            }

            // Loading badge while full-resolution photo is downloading in background.
            Rectangle {
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 20
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.viewedItem !== null
                    && !page.viewedItem.isVideo
                    && (!page.viewedItem.fullUrl || page.viewedItem.fullUrl.length === 0)
                    && Photos.busyItem === page.viewedItem.id
                height: 32
                width: loadingRow.implicitWidth + 24
                radius: Theme.pill(height)
                color: Theme.surfaceContainerHigh
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                Row {
                    id: loadingRow
                    anchors.centerIn: parent
                    spacing: 8
                    Spinner { width: 14; height: 14; anchors.verticalCenter: parent.verticalCenter }
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        role: "caption"
                        text: qsTr("Loading full resolution…")
                    }
                }
            }

            // Video play button overlay in viewer.
            Rectangle {
                id: playOverlayBtn
                anchors.centerIn: parent
                visible: page.viewedItem !== null && !!page.viewedItem.isVideo
                width: 76; height: 76
                radius: 38
                color: Qt.rgba(0, 0, 0, 0.68)
                border.width: 2
                border.color: "#ffffff"
                scale: playTap.pressed ? Theme.pressScale : 1
                Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
                activeFocusOnTab: true
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Play video")
                Accessible.onPressAction: if (page.viewedItem) Photos.openItem(page.viewedItem.id)
                Keys.onPressed: (event) => {
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                        if (page.viewedItem)
                            Photos.openItem(page.viewedItem.id)
                        event.accepted = true
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -3
                    radius: parent.radius + 3
                    color: "transparent"
                    border.width: 2
                    border.color: Theme.primary
                    visible: Theme.focusVisible(playOverlayBtn)
                }
                Icon {
                    anchors.centerIn: parent
                    width: 30; height: 30
                    path: Icons.play
                    color: "#ffffff"
                }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
                TapHandler {
                    id: playTap
                    onTapped: if (page.viewedItem) Photos.openItem(page.viewedItem.id)
                }
            }

            // Previous photo button (Left arrow).
            IconButton {
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 44; height: 44
                tonal: true
                visible: page.viewerIndex > 0
                iconPath: Icons.back
                label: qsTr("Previous photo")
                onClicked: page.stepViewer(-1)
            }

            // Next photo button (Right arrow).
            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 44; height: 44
                tonal: true
                visible: page.viewerIndex + 1 < Photos.count
                iconPath: Icons.chevronRight
                label: qsTr("Next photo")
                onClicked: page.stepViewer(1)
            }
        }
    }

    // ---- Album row component ----
    component AlbumRow: Item {
        id: albumRow
        property string albumId
        property string title
        property int count: 0
        property string thumbUrl
        property bool selected: false
        signal clicked

        height: 54
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: albumRow.title
        Accessible.onPressAction: albumRow.clicked()
        Keys.onReturnPressed: albumRow.clicked()
        Keys.onEnterPressed: albumRow.clicked()
        Keys.onSpacePressed: albumRow.clicked()

        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 10
            anchors.rightMargin: 10
            anchors.topMargin: 2
            anchors.bottomMargin: 2
            radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
            color: albumRow.selected
                ? Theme.secondaryContainer
                : albumHover.hovered
                  ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                  : "transparent"
            border.width: Theme.focusVisible(albumRow) ? 2 : (Theme.graphite && albumRow.selected ? 1 : 0)
            border.color: Theme.focusVisible(albumRow) ? Theme.primary : Theme.surfaceContent

            Item {
                id: coverBox
                x: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 36; height: 36
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusSm
                    color: Theme.surfaceContainerHigh
                    Icon {
                        anchors.centerIn: parent
                        width: 16; height: 16
                        path: Icons.photo
                        color: Theme.surfaceContentVariant
                        visible: coverImg.status !== Image.Ready
                    }
                    Image {
                        id: coverImg
                        anchors.fill: parent
                        asynchronous: true
                        fillMode: Image.PreserveAspectCrop
                        sourceSize: Qt.size(72, 72)
                        source: albumRow.thumbUrl
                        visible: status === Image.Ready
                    }
                }
            }

            Column {
                anchors.left: coverBox.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 1
                Txt {
                    width: parent.width
                    role: "body"
                    elide: Text.ElideRight
                    color: albumRow.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                    text: albumRow.title
                }
                Txt {
                    width: parent.width
                    role: "caption"
                    muted: !albumRow.selected
                    color: albumRow.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                    text: qsTr("%1").arg(albumRow.count)
                }
            }
        }

        HoverHandler { id: albumHover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: albumRow.clicked() }
    }

    // ---- Virtualized GridView cell component ----
    component PhotoTile: Item {
        id: tile
        required property int index
        required property string itemId
        required property string name
        required property real date
        required property real prevDate
        required property real size
        required property int itemWidth
        required property int itemHeight
        required property int duration
        required property bool isVideo
        required property string album
        required property string thumb
        required property string fullUrl

        property string requestedThumbId: ""
        readonly property bool selected: page.selectedCount >= 0 && page.isSelected(itemId)
        readonly property bool firstOfDay: index === 0 || !page.sameDay(date, prevDate)

        function activateTile() {
            if (page.selectMode || page.selectedCount > 0)
                page.toggleSelect(tile.itemId, tile.index, false)
            else
                page.openViewer(tile.index)
        }

        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: tile.name
        Accessible.onPressAction: tile.activateTile()
        Keys.onReturnPressed: tile.activateTile()
        Keys.onEnterPressed: tile.activateTile()
        Keys.onSpacePressed: page.toggleSelect(tile.itemId, tile.index, false)

        function checkThumb() {
            if (thumb.length === 0 && itemId.length > 0) {
                if (requestedThumbId !== itemId) {
                    if (requestedThumbId.length > 0)
                        Photos.dropThumb(requestedThumbId)
                    requestedThumbId = itemId
                    Photos.needThumb(itemId)
                }
            } else if (thumb.length > 0) {
                requestedThumbId = ""
            }
        }

        Component.onCompleted: checkThumb()
        onItemIdChanged: checkThumb()
        onThumbChanged: if (thumb.length > 0) requestedThumbId = ""
        Component.onDestruction: {
            if (requestedThumbId.length > 0 && thumb.length === 0)
                Photos.dropThumb(requestedThumbId)
        }

        Rectangle {
            id: cellCard
            anchors.fill: parent
            anchors.margins: 4
            radius: Theme.radiusSm
            color: Theme.surfaceContainerHigh
            border.width: tile.selected ? 3 : (Theme.focusVisible(tile) ? 2 : (Theme.graphite ? 1 : 0))
            border.color: (tile.selected || Theme.focusVisible(tile)) ? Theme.primary : Theme.outlineVariant
            clip: true

            Icon {
                anchors.centerIn: parent
                width: 24; height: 24
                path: tile.isVideo ? Icons.video : Icons.photo
                color: Theme.surfaceContentVariant
                visible: thumbImg.status !== Image.Ready
            }

            Image {
                id: thumbImg
                anchors.fill: parent
                anchors.margins: tile.selected ? 3 : 0
                asynchronous: true
                fillMode: Image.PreserveAspectCrop
                sourceSize: Qt.size(256, 256)
                source: tile.thumb
                opacity: status === Image.Ready ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
            }

            // Subtle dark scrim when hovered or selected.
            Rectangle {
                anchors.fill: parent
                color: tile.selected
                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.18)
                    : (tileHover.hovered ? Qt.rgba(0, 0, 0, 0.14) : "transparent")
            }

            // Day group pill on the first tile of each subsequent day.
            Rectangle {
                anchors.top: parent.top
                anchors.topMargin: 6
                anchors.left: parent.left
                anchors.leftMargin: 6
                visible: tile.firstOfDay && tile.index > 0 && tile.date > 0
                height: 22
                width: Math.min(parent.width - 40, dayBadgeTxt.implicitWidth + 14)
                radius: 11
                color: Qt.rgba(0, 0, 0, 0.68)
                Txt {
                    id: dayBadgeTxt
                    anchors.centerIn: parent
                    width: parent.width - 12
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    role: "caption"
                    size: 11
                    color: "#ffffff"
                    text: (tile.firstOfDay && tile.index > 0 && tile.date > 0)
                        ? page.shortDayLabel(tile.date) : ""
                }
            }

            // Play badge in center for video items.
            Rectangle {
                anchors.centerIn: parent
                visible: tile.isVideo
                width: 36; height: 36
                radius: 18
                color: Qt.rgba(0, 0, 0, 0.62)
                border.width: 1.5
                border.color: "#ffffff"
                Icon {
                    anchors.centerIn: parent
                    width: 16; height: 16
                    path: Icons.play
                    color: "#ffffff"
                }
            }

            // Video duration badge in bottom-right.
            Rectangle {
                anchors.right: parent.right
                anchors.rightMargin: 6
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 6
                visible: tile.isVideo
                height: 20
                width: durTxt.implicitWidth + 12
                radius: 10
                color: Qt.rgba(0, 0, 0, 0.72)
                Txt {
                    id: durTxt
                    anchors.centerIn: parent
                    role: "caption"
                    size: 11
                    color: "#ffffff"
                    text: tile.isVideo ? page.formatVideoDuration(tile.duration) : ""
                }
            }

            // Selection checkbox circle in top-right.
            Rectangle {
                id: checkCircle
                z: 2
                anchors.top: parent.top
                anchors.topMargin: 6
                anchors.right: parent.right
                anchors.rightMargin: 6
                width: 24; height: 24
                radius: 12
                visible: tile.selected || tileHover.hovered || page.selectMode || page.selectedCount > 0
                color: tile.selected ? Theme.primary : Qt.rgba(0, 0, 0, 0.48)
                border.width: tile.selected ? 0 : 1.5
                border.color: "#ffffff"
                Accessible.role: Accessible.CheckBox
                Accessible.name: qsTr("Select photo")
                Accessible.checkable: true
                Accessible.checked: tile.selected
                Accessible.onPressAction: page.toggleSelect(tile.itemId, tile.index, false)
                Accessible.onToggleAction: page.toggleSelect(tile.itemId, tile.index, false)
                Icon {
                    anchors.centerIn: parent
                    visible: tile.selected
                    width: 14; height: 14
                    path: Icons.check
                    color: Theme.primaryContent
                }
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -6
                    onClicked: (mouse) => page.toggleSelect(tile.itemId, tile.index,
                                                            (mouse.modifiers & Qt.ShiftModifier) !== 0)
                }
            }

            HoverHandler { id: tileHover; cursorShape: Qt.PointingHandCursor }
            // Under the selection circle (its z is higher). A MouseArea, as
            // a TapHandler's event point doesn't carry the keyboard modifiers.
            MouseArea {
                anchors.fill: parent
                onClicked: (mouse) => {
                    const mods = mouse.modifiers
                    const shift = (mods & Qt.ShiftModifier) !== 0
                    const ctrl = (mods & Qt.ControlModifier) !== 0
                    if (page.selectMode || page.selectedCount > 0 || shift || ctrl) {
                        page.toggleSelect(tile.itemId, tile.index, shift)
                    } else {
                        page.openViewer(tile.index)
                    }
                }
            }
        }
    }
}
