// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Deck editor: configure pages of one-tap control tiles for paired phones,
// preview live PC states (volume, mic mute, media), and test actions locally.
Item {
    id: page
    property bool active: false
    onActiveChanged: DeckController.setPageActive(active)
    Component.onCompleted: if (active) DeckController.setPageActive(true)
    Component.onDestruction: DeckController.setPageActive(false)

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    readonly property var pagesList: {
        try {
            const parsed = JSON.parse(DeckController.pagesJson)
            return Array.isArray(parsed) && parsed.length > 0 ? parsed : [{ id: "main", name: qsTr("Main"), tiles: [] }]
        } catch (e) {
            return [{ id: "main", name: qsTr("Main"), tiles: [] }]
        }
    }
    property int currentPageIndex: 0
    readonly property var currentPage: pagesList[Math.min(currentPageIndex, Math.max(0, pagesList.length - 1))] || { id: "main", name: qsTr("Main"), tiles: [] }
    readonly property var currentTiles: currentPage.tiles || []
    property int draggedTileIndex: -1

    onPagesListChanged: {
        if (currentPageIndex >= pagesList.length)
            currentPageIndex = Math.max(0, pagesList.length - 1)
    }

    function iconPathFor(name, kind) {
        if (kind === "media_play_pause" && DeckController.playing)
            return Icons.pause
        if (kind === "mic_mute" && DeckController.micState === 1)
            return Icons.micOff
        if ((kind === "volume_mute" || kind === "volume_up" || kind === "volume_down") && DeckController.muted)
            return Icons.soundOff
        switch (name) {
        case "play": return Icons.play
        case "pause": return Icons.pause
        case "skip_next":
        case "next": return Icons.skipNext
        case "skip_previous":
        case "prev": return Icons.skipPrevious
        case "volume_up": return Icons.volumeUp
        case "volume_down": return Icons.volumeDown
        case "volume_off":
        case "volume_mute": return Icons.soundOff
        case "mic": return Icons.mic
        case "mic_off": return Icons.micOff
        case "lock": return Icons.lock
        case "desktop": return Icons.desktop
        case "switch_window":
        case "window": return Icons.window
        case "screenshot":
        case "camera": return Icons.camera
        case "shortcut":
        case "keyboard": return Icons.keyboard
        case "globe": return Icons.globe
        case "text": return Icons.text
        case "app": return Icons.apps
        case "terminal": return Icons.terminal
        case "sparkle": return Icons.sparkle
        case "bolt": return Icons.bolt
        case "star": return Icons.star
        case "music": return Icons.music
        case "folder": return Icons.folder
        case "video": return Icons.video
        case "bell": return Icons.bell
        default: return Icons.deck
        }
    }

    function isTileHighlighted(kind) {
        if (kind === "media_play_pause") return DeckController.playing
        if (kind === "volume_mute") return DeckController.muted
        if (kind === "mic_mute") return DeckController.micState === 1
        return false
    }

    readonly property var iconChoices: [
        "play", "pause", "skip_next", "skip_previous",
        "volume_up", "volume_down", "volume_off", "mic",
        "mic_off", "lock", "desktop", "switch_window",
        "screenshot", "shortcut", "globe", "text",
        "app", "terminal", "sparkle", "star"
    ]

    readonly property var actionChoices: [
        { id: "media_play_pause", group: qsTr("Media"), label: qsTr("Play / pause") },
        { id: "media_previous",   group: qsTr("Media"), label: qsTr("Previous track") },
        { id: "media_next",       group: qsTr("Media"), label: qsTr("Next track") },
        { id: "volume_down",      group: qsTr("Audio"), label: qsTr("Volume down") },
        { id: "volume_up",        group: qsTr("Audio"), label: qsTr("Volume up") },
        { id: "volume_mute",      group: qsTr("Audio"), label: qsTr("Mute speaker") },
        { id: "mic_mute",         group: qsTr("Audio"), label: qsTr("Mute microphone") },
        { id: "show_desktop",     group: qsTr("System"), label: qsTr("Show desktop (Win+D)") },
        { id: "switch_window",    group: qsTr("System"), label: qsTr("Switch window (Alt+Tab)") },
        { id: "screenshot",       group: qsTr("System"), label: qsTr("Screenshot (Win+Shift+S)") },
        { id: "lock_pc",          group: qsTr("System"), label: qsTr("Lock PC") },
        { id: "shortcut",         group: qsTr("Custom"), label: qsTr("Keyboard shortcut") },
        { id: "open_url",         group: qsTr("Custom"), label: qsTr("Open website") },
        { id: "type_text",        group: qsTr("Custom"), label: qsTr("Type text snippet") },
        { id: "launch_app",       group: qsTr("Apps"),   label: qsTr("Launch application (.exe / .lnk)") },
        { id: "run_command",      group: qsTr("Apps"),   label: qsTr("Run shell command") }
    ]

    function openAddTile() {
        editorSheet.tileId = ""
        editorSheet.actionKind = "shortcut"
        editorSheet.tileLabel = DeckController.defaultLabelFor("shortcut")
        editorSheet.labelCustomized = false
        editorSheet.tileIcon = DeckController.defaultIconFor("shortcut")
        editorSheet.tileColor = DeckController.defaultColorFor("shortcut")
        editorSheet.paramValue = "c"
        editorSheet.modCtrl = true
        editorSheet.modAlt = false
        editorSheet.modShift = false
        editorSheet.modWin = false
        editorSheet.errorText = ""
        editorSheet.open()
    }

    function openEditTile(tile) {
        editorSheet.tileId = tile.id
        editorSheet.actionKind = tile.kind
        editorSheet.tileLabel = tile.label
        editorSheet.labelCustomized = true
        editorSheet.tileIcon = tile.icon
        editorSheet.tileColor = tile.color
        editorSheet.paramValue = tile.param || ""
        editorSheet.modCtrl = !!tile.ctrl
        editorSheet.modAlt = !!tile.alt
        editorSheet.modShift = !!tile.shift
        editorSheet.modWin = !!tile.win
        editorSheet.errorText = ""
        editorSheet.open()
    }

    FileDialog {
        id: appFilePicker
        title: qsTr("Choose an application or shortcut")
        nameFilters: [qsTr("Applications and shortcuts (*.exe *.lnk)")]
        onAccepted: {
            const localPath = DeckController.urlToLocalPath(selectedFile.toString())
            if (localPath.length > 0) {
                editorSheet.paramValue = localPath
                if (!editorSheet.labelCustomized || editorSheet.tileLabel.length === 0 || editorSheet.tileLabel === qsTr("Launch app")) {
                    const parts = localPath.split(/[\\/]/)
                    const file = parts[parts.length - 1] || ""
                    const stem = file.replace(/\.(exe|lnk)$/i, "")
                    if (stem.length > 0)
                        editorSheet.tileLabel = stem
                }
            }
        }
    }

    Flickable {
        anchors.fill: parent
        contentHeight: mainCol.height + Theme.contentPadding * 2
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: mainCol
            width: Math.min(960, parent.width - Theme.contentPadding * 2)
            x: Math.max(Theme.contentPadding, (parent.width - width) / 2)
            y: Theme.contentPadding
            spacing: Theme.gutter

            // ---- Header & Live PC Status ----
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 14

                    Row {
                        width: parent.width
                        spacing: 12
                        Column {
                            width: parent.width - headerActions.width - 12
                            spacing: 4
                            Txt { text: qsTr("Control Deck"); role: "title" }
                            Txt {
                                width: parent.width
                                wrapMode: Text.WordWrap
                                role: "bodySmall"
                                muted: true
                                text: qsTr("One-tap buttons on your phone for media, volume, mic mute, window management, shortcuts, and apps. Drag tiles to reorder them or click any tile to customize it.")
                            }
                        }
                        Row {
                            id: headerActions
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Button {
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.refresh
                                text: qsTr("Reset defaults")
                                onClicked: resetConfirmSheet.open()
                            }
                            Button {
                                variant: "fill"
                                size: "sm"
                                iconPath: Icons.plus
                                text: qsTr("Add tile")
                                onClicked: page.openAddTile()
                            }
                        }
                    }

                    Divider { width: parent.width }

                    // Live status summary showing what phones see right now
                    Flow {
                        width: parent.width
                        spacing: 16

                        Row {
                            spacing: 6
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 15; height: 15
                                path: DeckController.muted ? Icons.soundOff : Icons.speaker
                                color: Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                muted: true
                                text: DeckController.muted
                                    ? qsTr("Speaker muted (%1%)").arg(DeckController.volume)
                                    : qsTr("Speaker %1%").arg(DeckController.volume)
                            }
                        }
                        Row {
                            spacing: 6
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 15; height: 15
                                path: DeckController.micState === 1 ? Icons.micOff : Icons.mic
                                color: Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                muted: true
                                text: DeckController.micState === 1 ? qsTr("Mic muted")
                                    : DeckController.micState === 0 ? qsTr("Mic live")
                                    : qsTr("No mic")
                            }
                        }
                        Row {
                            spacing: 6
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 15; height: 15
                                path: DeckController.playing ? Icons.play : Icons.pause
                                color: Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                muted: true
                                text: DeckController.playing ? qsTr("Media playing") : qsTr("Media paused")
                            }
                        }
                    }
                }
            }

            // ---- Page Tabs & Page Management ----
            Item {
                width: parent.width
                height: 38

                Row {
                    id: pageTabsRow
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Repeater {
                        model: page.pagesList
                        delegate: Chip {
                            required property var modelData
                            required property int index
                            interactive: true
                            selected: index === page.currentPageIndex
                            text: "%1 (%2)".arg(modelData.name).arg((modelData.tiles || []).length)
                            onClicked: page.currentPageIndex = index
                        }
                    }

                    Button {
                        visible: page.pagesList.length < 8
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.plus
                        text: qsTr("New page")
                        onClicked: {
                            pageNameSheet.isNew = true
                            pageNameInput.text = qsTr("Page %1").arg(page.pagesList.length + 1)
                            pageNameSheet.open()
                        }
                    }
                }

                Row {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Button {
                        variant: "text"
                        size: "sm"
                        iconPath: Icons.edit
                        text: qsTr("Rename page")
                        onClicked: {
                            pageNameSheet.isNew = false
                            pageNameInput.text = page.currentPage.name
                            pageNameSheet.open()
                        }
                    }
                    Button {
                        visible: page.pagesList.length > 1
                        variant: "text"
                        size: "sm"
                        iconPath: Icons.trash
                        text: qsTr("Delete page")
                        onClicked: DeckController.removePage(page.currentPage.id)
                    }
                }
            }

            // ---- Tile Grid (Phone Preview & Interactive Editor) ----
            Grid {
                id: tileGrid
                width: parent.width
                columns: width >= 760 ? 4 : 3
                spacing: 12

                readonly property real cellWidth: (width - spacing * (columns - 1)) / columns

                Repeater {
                    model: page.currentTiles
                    delegate: Item {
                        id: tileSlot
                        required property var modelData
                        required property int index

                        width: tileGrid.cellWidth
                        height: 112
                        activeFocusOnTab: true
                        Accessible.role: Accessible.Button
                        Accessible.name: modelData.label
                        Accessible.onPressAction: page.openEditTile(modelData)
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                page.openEditTile(modelData)
                                event.accepted = true
                            }
                        }

                        readonly property bool isBeingDragged: page.draggedTileIndex === index
                        readonly property bool highlighted: page.isTileHighlighted(modelData.kind)

                        Rectangle {
                            id: tileCard
                            anchors.fill: parent
                            radius: Theme.radiusMd
                            color: Theme.graphite
                                ? (tileHover.hovered ? Theme.surfaceContainer : "transparent")
                                : (tileHover.hovered ? Theme.surfaceContainerHigh : Theme.tileColor)
                            border.width: isBeingDragged || Theme.focusVisible(tileSlot) ? 2 : (Theme.graphite ? 1 : 0)
                            border.color: isBeingDragged || Theme.focusVisible(tileSlot)
                                ? Theme.primary
                                : (tileSlot.highlighted ? Theme.surfaceContent : Theme.outlineVariant)
                            scale: isBeingDragged ? Theme.pressScale : 1.0
                            Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
                            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }

                            // Top row: Icon badge + live status pill + quick test button
                            Item {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: 14
                                height: 34

                                Rectangle {
                                    id: iconBadge
                                    width: 34; height: 34
                                    radius: Theme.radiusSm
                                    color: tileSlot.highlighted
                                        ? Theme.primaryContainer
                                        : (Theme.graphite ? Theme.surfaceContainerHigh : Theme.secondaryContainer)
                                    Icon {
                                        anchors.centerIn: parent
                                        width: 18; height: 18
                                        path: page.iconPathFor(tileSlot.modelData.icon, tileSlot.modelData.kind)
                                        color: tileSlot.highlighted
                                            ? Theme.primaryContainerContent
                                            : (Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent)
                                    }
                                }

                                // Live status pill (e.g. "74%", "Muted", "Playing", "Live")
                                Rectangle {
                                    visible: (tileSlot.modelData.status || "").length > 0
                                    anchors.left: iconBadge.right
                                    anchors.leftMargin: 8
                                    anchors.verticalCenter: parent.verticalCenter
                                    height: 22
                                    width: statusLabel.implicitWidth + 12
                                    radius: 11
                                    color: tileSlot.highlighted ? Theme.primaryContainer : Theme.surfaceContainerHigh
                                    Txt {
                                        id: statusLabel
                                        anchors.centerIn: parent
                                        text: tileSlot.modelData.status || ""
                                        role: "mono"
                                        size: 11
                                        color: tileSlot.highlighted ? Theme.primaryContainerContent : Theme.surfaceContentVariant
                                    }
                                }

                                // Quick "Test" button in top-right corner
                                Rectangle {
                                    id: testBtn
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 28; height: 28
                                    radius: 14
                                    activeFocusOnTab: true
                                    color: testHover.hovered ? Theme.surfaceContainerHighest : "transparent"
                                    border.width: Theme.focusVisible(testBtn) ? 2 : 0
                                    border.color: Theme.primary
                                    opacity: tileHover.hovered || testHover.hovered || testBtn.activeFocus ? 1 : 0.55
                                    Accessible.role: Accessible.Button
                                    Accessible.name: qsTr("Test %1").arg(tileSlot.modelData.label)
                                    Accessible.onPressAction: DeckController.testTile(tileSlot.modelData.id)
                                    Keys.onPressed: (event) => {
                                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                            DeckController.testTile(tileSlot.modelData.id)
                                            event.accepted = true
                                        }
                                    }

                                    Icon {
                                        anchors.centerIn: parent
                                        width: 14; height: 14
                                        path: Icons.play
                                        color: Theme.surfaceContentVariant
                                    }
                                    HoverHandler { id: testHover; cursorShape: Qt.PointingHandCursor }
                                    TapHandler {
                                        onTapped: DeckController.testTile(tileSlot.modelData.id)
                                    }
                                }
                            }

                            // Bottom labels: Tile label + action subtitle
                            Column {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                anchors.margins: 14
                                spacing: 2

                                Txt {
                                    width: parent.width
                                    text: tileSlot.modelData.label
                                    role: "body"
                                    weight: Font.DemiBold
                                    elide: Text.ElideRight
                                    color: Theme.surfaceContent
                                }
                                Txt {
                                    visible: (tileSlot.modelData.subtitle || "").length > 0
                                    width: parent.width
                                    text: tileSlot.modelData.subtitle || ""
                                    role: "bodySmall"
                                    size: 11
                                    elide: Text.ElideRight
                                    muted: true
                                }
                            }

                            HoverHandler { id: tileHover; cursorShape: Qt.PointingHandCursor }

                            // Drag-to-reorder and click-to-edit
                            MouseArea {
                                anchors.fill: parent
                                anchors.rightMargin: 36 // Leave top-right test button clickable
                                acceptedButtons: Qt.LeftButton
                                pressAndHoldInterval: 180
                                property point pressPos: Qt.point(0, 0)
                                property bool dragging: false

                                onPressed: (mouse) => {
                                    pressPos = Qt.point(mouse.x, mouse.y)
                                    dragging = false
                                }
                                onPositionChanged: (mouse) => {
                                    if (!pressed) return
                                    const dx = mouse.x - pressPos.x
                                    const dy = mouse.y - pressPos.y
                                    if (!dragging && (dx * dx + dy * dy > 64)) {
                                        dragging = true
                                        page.draggedTileIndex = tileSlot.index
                                    }
                                    if (dragging) {
                                        const inGrid = tileSlot.mapToItem(tileGrid, mouse.x, mouse.y)
                                        const col = Math.floor(inGrid.x / (tileGrid.cellWidth + tileGrid.spacing))
                                        const row = Math.floor(inGrid.y / (tileSlot.height + tileGrid.spacing))
                                        if (col >= 0 && col < tileGrid.columns && row >= 0) {
                                            const targetIdx = row * tileGrid.columns + col
                                            if (targetIdx >= 0 && targetIdx < page.currentTiles.length && targetIdx !== tileSlot.index) {
                                                DeckController.moveTile(page.currentPage.id, tileSlot.index, targetIdx)
                                                page.draggedTileIndex = targetIdx
                                            }
                                        }
                                    }
                                }
                                onReleased: {
                                    if (!dragging) {
                                        page.openEditTile(tileSlot.modelData)
                                    }
                                    dragging = false
                                    page.draggedTileIndex = -1
                                }
                                onCanceled: {
                                    dragging = false
                                    page.draggedTileIndex = -1
                                }
                            }
                        }
                    }
                }

                // Add-tile card at the end of the grid
                Rectangle {
                    visible: page.currentTiles.length < 24
                    width: tileGrid.cellWidth
                    height: 112
                    radius: Theme.radiusMd
                    color: addHover.hovered ? Theme.surfaceContainerHigh : "transparent"
                    border.width: Theme.focusVisible(addTileBtn) ? 2 : 1
                    border.color: Theme.focusVisible(addTileBtn) ? Theme.primary : Theme.outlineVariant
                    id: addTileBtn
                    activeFocusOnTab: true
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("Add tile")
                    Accessible.onPressAction: page.openAddTile()
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            page.openAddTile()
                            event.accepted = true
                        }
                    }

                    Column {
                        anchors.centerIn: parent
                        spacing: 6
                        Icon {
                            anchors.horizontalCenter: parent.horizontalCenter
                            path: Icons.plus
                            color: Theme.surfaceContentVariant
                        }
                        Txt {
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: qsTr("Add tile")
                            role: "bodySmall"
                            muted: true
                        }
                    }
                    HoverHandler { id: addHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: page.openAddTile() }
                }
            }
        }
    }

    // ---- Add / Edit Tile Sheet ----
    Sheet {
        id: editorSheet
        cardWidth: 680

        property string tileId: ""
        property string actionKind: "shortcut"
        property string tileLabel: ""
        property bool labelCustomized: false
        property string tileIcon: "keyboard"
        property string tileColor: "amber"
        property string paramValue: ""
        property bool modCtrl: false
        property bool modAlt: false
        property bool modShift: false
        property bool modWin: false
        property string errorText: ""

        function selectKind(kind) {
            actionKind = kind
            if (!labelCustomized || tileLabel.trim().length === 0) {
                tileLabel = DeckController.defaultLabelFor(kind)
            }
            tileIcon = DeckController.defaultIconFor(kind)
            tileColor = DeckController.defaultColorFor(kind)
            errorText = ""
            if (kind === "shortcut" && paramValue.length === 0) {
                paramValue = "c"
                modCtrl = true
            } else if (kind === "open_url" && paramValue.length === 0) {
                paramValue = "https://"
            }
        }

        Column {
            width: parent.width
            spacing: 10

            // Sheet Header + Live Tile Preview
            Row {
                width: parent.width
                spacing: 14

                Column {
                    width: parent.width - previewBox.width - 14
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4
                    Txt {
                        text: editorSheet.tileId.length === 0 ? qsTr("Add tile") : qsTr("Edit tile")
                        role: "headline"
                    }
                    Txt {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        role: "bodySmall"
                        muted: true
                        text: qsTr("Phones only receive the tile's label, icon, and action kind. File paths and commands stay on this PC.")
                    }
                }

                // Live miniature tile preview
                Rectangle {
                    id: previewBox
                    width: 148; height: 68
                    radius: Theme.radiusMd
                    color: Theme.graphite ? "transparent" : Theme.tileColor
                    border.width: Theme.graphite ? 1 : 0
                    border.color: Theme.outlineVariant

                    Rectangle {
                        x: 10; y: 10
                        width: 24; height: 24
                        radius: Theme.radiusSm
                        color: Theme.graphite ? Theme.surfaceContainerHigh : Theme.secondaryContainer
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            path: page.iconPathFor(editorSheet.tileIcon, editorSheet.actionKind)
                            color: Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
                        }
                    }
                    Txt {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        anchors.margins: 10
                        text: editorSheet.tileLabel.length > 0 ? editorSheet.tileLabel : DeckController.defaultLabelFor(editorSheet.actionKind)
                        role: "bodySmall"
                        weight: Font.DemiBold
                        elide: Text.ElideRight
                        color: Theme.surfaceContent
                    }
                }
            }

            Divider { width: parent.width }

            // Action Picker
            Column {
                width: parent.width
                spacing: 6
                Txt { text: qsTr("Action"); role: "label"; muted: true }
                Flow {
                    id: actionFlow
                    width: parent.width
                    spacing: 6

                    Repeater {
                        model: page.actionChoices
                        delegate: Chip {
                            required property var modelData
                            interactive: true
                            selected: editorSheet.actionKind === modelData.id
                            text: modelData.label
                            onClicked: editorSheet.selectKind(modelData.id)
                        }
                    }
                }
            }

            // Parameter inputs for parameterized actions
            Column {
                width: parent.width
                spacing: 6
                visible: editorSheet.actionKind === "shortcut"
                      || editorSheet.actionKind === "open_url"
                      || editorSheet.actionKind === "type_text"
                      || editorSheet.actionKind === "launch_app"
                      || editorSheet.actionKind === "run_command"

                // Modifiers for Shortcut
                Row {
                    visible: editorSheet.actionKind === "shortcut"
                    spacing: 8
                    Chip {
                        interactive: true
                        selected: editorSheet.modCtrl
                        text: qsTr("Ctrl")
                        onClicked: editorSheet.modCtrl = !editorSheet.modCtrl
                    }
                    Chip {
                        interactive: true
                        selected: editorSheet.modAlt
                        text: qsTr("Alt")
                        onClicked: editorSheet.modAlt = !editorSheet.modAlt
                    }
                    Chip {
                        interactive: true
                        selected: editorSheet.modShift
                        text: qsTr("Shift")
                        onClicked: editorSheet.modShift = !editorSheet.modShift
                    }
                    Chip {
                        interactive: true
                        selected: editorSheet.modWin
                        text: qsTr("Win")
                        onClicked: editorSheet.modWin = !editorSheet.modWin
                    }
                }

                Txt {
                    role: "bodySmall"
                    muted: true
                    text: editorSheet.actionKind === "shortcut" ? qsTr("Key (e.g. c, v, z, f5, tab, enter, space, escape, up, down):")
                        : editorSheet.actionKind === "open_url" ? qsTr("Website address (https://…):")
                        : editorSheet.actionKind === "type_text" ? qsTr("Text snippet to type:")
                        : editorSheet.actionKind === "launch_app" ? qsTr("Application or shortcut path (.exe or .lnk):")
                        : qsTr("Shell command to run on this PC:")
                }

                Row {
                    width: parent.width
                    spacing: 8

                    Rectangle {
                        width: editorSheet.actionKind === "launch_app" ? parent.width - browseBtn.width - 8 : parent.width
                        height: 36
                        radius: Theme.radiusSm
                        color: Theme.surfaceContainerHigh
                        border.width: paramInput.activeFocus ? 2 : 1
                        border.color: paramInput.activeFocus ? Theme.primary : Theme.outlineVariant

                        TextInput {
                            id: paramInput
                            anchors.fill: parent
                            anchors.leftMargin: 12
                            anchors.rightMargin: 12
                            verticalAlignment: TextInput.AlignVCenter
                            clip: true
                            color: Theme.surfaceContent
                            selectionColor: Theme.primaryContainer
                            selectedTextColor: Theme.primaryContainerContent
                            font.family: editorSheet.actionKind === "run_command" || editorSheet.actionKind === "shortcut"
                                ? Theme.fontMono : Theme.fontUi
                            font.pixelSize: 13
                            text: editorSheet.paramValue
                            onTextEdited: editorSheet.paramValue = text
                            Accessible.name: qsTr("Action parameter")
                        }
                    }

                    Button {
                        id: browseBtn
                        visible: editorSheet.actionKind === "launch_app"
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.folder
                        text: qsTr("Browse…")
                        onClicked: appFilePicker.open()
                    }
                }

                Txt {
                    visible: editorSheet.actionKind === "run_command"
                    width: parent.width
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    color: Theme.warning
                    text: qsTr("Requires \"Allow running Deck commands\" to be turned on for the phone in Settings → Devices.")
                }
            }

            // Tile Label Input
            Column {
                width: parent.width
                spacing: 6
                Txt { text: qsTr("Label"); role: "label"; muted: true }
                Rectangle {
                    width: parent.width
                    height: 36
                    radius: Theme.radiusSm
                    color: Theme.surfaceContainerHigh
                    border.width: labelInput.activeFocus ? 2 : 1
                    border.color: labelInput.activeFocus ? Theme.primary : Theme.outlineVariant

                    TextInput {
                        id: labelInput
                        anchors.fill: parent
                        anchors.leftMargin: 12
                        anchors.rightMargin: 12
                        verticalAlignment: TextInput.AlignVCenter
                        clip: true
                        color: Theme.surfaceContent
                        selectionColor: Theme.primaryContainer
                        selectedTextColor: Theme.primaryContainerContent
                        font.family: Theme.fontUi
                        font.pixelSize: 14
                        text: editorSheet.tileLabel
                        onTextEdited: {
                            editorSheet.tileLabel = text
                            editorSheet.labelCustomized = true
                        }
                        Accessible.name: qsTr("Tile label")
                    }
                }
            }

            // Icon Picker
            Column {
                width: parent.width
                spacing: 6
                Txt { text: qsTr("Icon"); role: "label"; muted: true }
                Flow {
                    width: parent.width
                    spacing: 5
                    Repeater {
                        model: page.iconChoices
                        delegate: Rectangle {
                            id: iconCell
                            required property string modelData
                            readonly property bool chosen: editorSheet.tileIcon === modelData
                            width: 27; height: 27
                            radius: Theme.radiusSm
                            activeFocusOnTab: true
                            color: chosen ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                            border.width: chosen || Theme.focusVisible(iconCell) ? 2 : 0
                            border.color: Theme.primary
                            Accessible.role: Accessible.RadioButton
                            Accessible.name: modelData
                            Accessible.checked: chosen
                            Accessible.onPressAction: editorSheet.tileIcon = modelData
                            Keys.onPressed: (event) => {
                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                    editorSheet.tileIcon = modelData
                                    event.accepted = true
                                }
                            }
                            Icon {
                                anchors.centerIn: parent
                                width: 15; height: 15
                                path: page.iconPathFor(modelData, "")
                                color: chosen ? Theme.secondaryContainerContent : Theme.surfaceContent
                            }
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: editorSheet.tileIcon = modelData }
                        }
                    }
                }
            }

            Txt {
                visible: editorSheet.errorText.length > 0
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                color: Theme.error
                text: editorSheet.errorText
            }

            // Sheet Actions
            Item {
                width: parent.width
                height: 38

                Row {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8
                    visible: editorSheet.tileId.length > 0

                    Button {
                        variant: "outline"
                        size: "sm"
                        iconPath: Icons.trash
                        text: qsTr("Delete")
                        onClicked: {
                            DeckController.removeTile(page.currentPage.id, editorSheet.tileId)
                            editorSheet.close()
                        }
                    }
                    Button {
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.bolt
                        text: qsTr("Test")
                        onClicked: DeckController.testTile(editorSheet.tileId)
                    }
                }

                Row {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Button {
                        variant: "text"
                        text: qsTr("Cancel")
                        onClicked: editorSheet.close()
                    }
                    Button {
                        variant: "fill"
                        text: qsTr("Save")
                        onClicked: {
                            const err = DeckController.saveTile(
                                page.currentPage.id,
                                editorSheet.tileId,
                                editorSheet.tileLabel,
                                editorSheet.tileIcon,
                                editorSheet.tileColor,
                                editorSheet.actionKind,
                                editorSheet.paramValue,
                                editorSheet.modCtrl,
                                editorSheet.modAlt,
                                editorSheet.modShift,
                                editorSheet.modWin
                            )
                            if (err.length > 0) {
                                editorSheet.errorText = err
                            } else {
                                editorSheet.close()
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- Add / Rename Page Sheet ----
    Sheet {
        id: pageNameSheet
        property bool isNew: true
        cardWidth: 380

        function submitPageName() {
            const trimmed = pageNameInput.text.trim()
            if (trimmed.length === 0) return
            if (pageNameSheet.isNew) {
                DeckController.addPage(trimmed)
                page.currentPageIndex = page.pagesList.length - 1
            } else {
                DeckController.renamePage(page.currentPage.id, trimmed)
            }
            pageNameSheet.close()
        }

        Column {
            width: parent.width
            spacing: 14

            Txt {
                text: pageNameSheet.isNew ? qsTr("New page") : qsTr("Rename page")
                role: "headline"
            }

            Rectangle {
                width: parent.width
                height: 38
                radius: Theme.radiusSm
                color: Theme.surfaceContainerHigh
                border.width: pageNameInput.activeFocus ? 2 : 1
                border.color: pageNameInput.activeFocus ? Theme.primary : Theme.outlineVariant

                TextInput {
                    id: pageNameInput
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    verticalAlignment: TextInput.AlignVCenter
                    clip: true
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    Accessible.name: qsTr("Page name")
                    Keys.onReturnPressed: pageNameSheet.submitPageName()
                    Keys.onEnterPressed: pageNameSheet.submitPageName()
                }
            }

            Row {
                anchors.right: parent.right
                spacing: 8
                Button { variant: "text"; text: qsTr("Cancel"); onClicked: pageNameSheet.close() }
                Button {
                    variant: "fill"
                    text: pageNameSheet.isNew ? qsTr("Create") : qsTr("Save")
                    onClicked: pageNameSheet.submitPageName()
                }
            }
        }
    }

    // ---- Reset Confirmation Sheet ----
    Sheet {
        id: resetConfirmSheet
        cardWidth: 400
        Column {
            width: parent.width
            spacing: 14
            Txt { text: qsTr("Reset Deck to defaults?"); role: "headline" }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("This replaces your custom pages and tiles with the default 11-tile Deck.")
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                Button { variant: "text"; text: qsTr("Cancel"); onClicked: resetConfirmSheet.close() }
                Button {
                    variant: "fill"
                    text: qsTr("Reset")
                    onClicked: {
                        DeckController.resetDefault()
                        page.currentPageIndex = 0
                        resetConfirmSheet.close()
                    }
                }
            }
        }
    }
}
