// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Deck editor: configure pages of one-tap control tiles for paired phones,
// preview live PC states (volume, mic mute, media), and test actions locally.
Item {
    id: page
    property bool active: false

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    readonly property var pagesList: {
        try {
            const parsed = JSON.parse(DeckController.pagesJson)
            return Array.isArray(parsed) && parsed.length > 0 ? parsed : [{ id: "main", name: "Main", tiles: [] }]
        } catch (e) {
            return [{ id: "main", name: "Main", tiles: [] }]
        }
    }
    property int currentPageIndex: 0
    readonly property var currentPage: pagesList[Math.min(currentPageIndex, Math.max(0, pagesList.length - 1))] || { id: "main", name: "Main", tiles: [] }
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

    // Curated tile color palette for Light and Dark modes (matches deck_colors::ALL).
    function tilePalette(colorName) {
        const dark = Theme.dark
        const map = {
            amber:  dark ? { bg: "#3B2A14", fg: "#FFDDB3", accent: "#F5A623", border: "#6A4B24" }
                         : { bg: "#FFF0D6", fg: "#4A2E00", accent: "#C97A00", border: "#F0D199" },
            coral:  dark ? { bg: "#3E221D", fg: "#FFDAD3", accent: "#FF7E67", border: "#6E3D34" }
                         : { bg: "#FFE6E0", fg: "#521A10", accent: "#C84B31", border: "#F4BFB3" },
            red:    dark ? { bg: "#3D1F24", fg: "#FFD9DE", accent: "#F45B69", border: "#6D353E" }
                         : { bg: "#FFE4E8", fg: "#51121B", accent: "#C92A37", border: "#F2B6BE" },
            teal:   dark ? { bg: "#153230", fg: "#C4F3EE", accent: "#38CFC3", border: "#285B57" }
                         : { bg: "#DCF7F4", fg: "#083B37", accent: "#0D9488", border: "#A3E5DF" },
            green:  dark ? { bg: "#183324", fg: "#C9F5DA", accent: "#4ADE80", border: "#2C5C41" }
                         : { bg: "#E0F8EA", fg: "#0C3B22", accent: "#16A34A", border: "#A9E8C2" },
            blue:   dark ? { bg: "#172E3D", fg: "#CEE7FF", accent: "#56B4F9", border: "#2B536E" }
                         : { bg: "#E0F2FE", fg: "#0B324D", accent: "#0284C7", border: "#AEDBFA" },
            violet: dark ? { bg: "#2E2340", fg: "#E9DDFF", accent: "#A985FF", border: "#523F73" }
                         : { bg: "#EFE7FF", fg: "#2E1557", accent: "#6B46C1", border: "#D3C0FA" },
            slate:  dark ? { bg: "#222934", fg: "#DCE4F0", accent: "#8FA3BF", border: "#3E4B5E" }
                         : { bg: "#EAEEF4", fg: "#1E293B", accent: "#475569", border: "#CBD5E1" }
        }
        return map[colorName] || map.amber
    }

    readonly property var colorChoices: [
        { id: "amber",  label: qsTr("Amber") },
        { id: "coral",  label: qsTr("Coral") },
        { id: "red",    label: qsTr("Red") },
        { id: "teal",   label: qsTr("Teal") },
        { id: "green",  label: qsTr("Green") },
        { id: "blue",   label: qsTr("Blue") },
        { id: "violet", label: qsTr("Violet") },
        { id: "slate",  label: qsTr("Slate") }
    ]

    readonly property var iconChoices: [
        "play", "pause", "skip_next", "skip_previous",
        "volume_up", "volume_down", "volume_off", "mic",
        "mic_off", "lock", "desktop", "switch_window",
        "screenshot", "shortcut", "globe", "text",
        "app", "terminal", "sparkle", "star"
    ]

    readonly property var actionChoices: [
        { id: "media_play_pause", group: qsTr("Media"), label: qsTr("Play / Pause") },
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
                if (!editorSheet.labelCustomized || editorSheet.tileLabel.length === 0 || editorSheet.tileLabel === qsTr("Launch App")) {
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
                                text: qsTr("Big, one-tap buttons on your phone for media, volume, mic mute, window management, shortcuts, and apps. Drag tiles to reorder them or click any tile to customize it.")
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

                    // Live status pills showing what phones see right now
                    Flow {
                        width: parent.width
                        spacing: 10

                        Chip {
                            iconPath: DeckController.muted ? Icons.soundOff : Icons.speaker
                            selected: !DeckController.muted
                            text: DeckController.muted
                                ? qsTr("Speaker: Muted (%1%)").arg(DeckController.volume)
                                : qsTr("Speaker: %1%").arg(DeckController.volume)
                        }
                        Chip {
                            iconPath: DeckController.micState === 1 ? Icons.micOff : Icons.mic
                            selected: DeckController.micState === 0
                            text: DeckController.micState === 1 ? qsTr("Mic: Muted")
                                : DeckController.micState === 0 ? qsTr("Mic: Live")
                                : qsTr("Mic: None")
                        }
                        Chip {
                            iconPath: DeckController.playing ? Icons.play : Icons.pause
                            selected: DeckController.playing
                            text: DeckController.playing ? qsTr("Media: Playing") : qsTr("Media: Paused")
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
                            selected: index === page.currentPageIndex
                            text: "%1 (%2)".arg(modelData.name).arg((modelData.tiles || []).length)
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: page.currentPageIndex = index }
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
                spacing: 14

                readonly property real cellWidth: (width - spacing * (columns - 1)) / columns

                Repeater {
                    model: page.currentTiles
                    delegate: Item {
                        id: tileSlot
                        required property var modelData
                        required property int index

                        width: tileGrid.cellWidth
                        height: 118

                        readonly property var pal: page.tilePalette(modelData.color)
                        readonly property bool isBeingDragged: page.draggedTileIndex === index

                        Rectangle {
                            id: tileCard
                            anchors.fill: parent
                            radius: Theme.radiusMd
                            color: Theme.graphite ? Theme.surfaceContainer : tileSlot.pal.bg
                            border.width: isBeingDragged ? 2 : (Theme.graphite ? 1 : 1)
                            border.color: isBeingDragged ? Theme.primary
                                        : Theme.graphite ? tileSlot.pal.accent : tileSlot.pal.border
                            scale: isBeingDragged ? 0.96 : (tileHover.hovered ? 1.01 : 1.0)
                            Behavior on scale { NumberAnimation { duration: Theme.fadeFast } }

                            // Left accent bar in Graphite so tile color is clear without heavy fill
                            Rectangle {
                                visible: Theme.graphite
                                width: 4
                                height: parent.height - 24
                                x: 8
                                anchors.verticalCenter: parent.verticalCenter
                                radius: 2
                                color: tileSlot.pal.accent
                            }

                            // Top row: Icon badge + live status pill + quick test button
                            Item {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.margins: 14
                                anchors.leftMargin: Theme.graphite ? 18 : 14
                                height: 34

                                Rectangle {
                                    id: iconBadge
                                    width: 34; height: 34
                                    radius: Theme.radiusSm
                                    color: Theme.graphite ? Theme.surfaceContainerHigh
                                                          : Qt.rgba(tileSlot.pal.accent.r, tileSlot.pal.accent.g, tileSlot.pal.accent.b, 0.18)
                                    Icon {
                                        anchors.centerIn: parent
                                        width: 18; height: 18
                                        path: page.iconPathFor(tileSlot.modelData.icon, tileSlot.modelData.kind)
                                        color: Theme.graphite ? tileSlot.pal.accent : tileSlot.pal.fg
                                    }
                                }

                                // Live status pill (e.g. "74%", "Muted", "Playing", "Live")
                                Rectangle {
                                    visible: (tileSlot.modelData.status || "").length > 0
                                    anchors.left: iconBadge.right
                                    anchors.leftMargin: 8
                                    anchors.verticalCenter: parent.verticalCenter
                                    height: 22
                                    width: statusLabel.implicitWidth + 14
                                    radius: 11
                                    color: Theme.graphite ? Theme.surfaceContainerHigh
                                                          : Qt.rgba(0, 0, 0, Theme.dark ? 0.28 : 0.08)
                                    Txt {
                                        id: statusLabel
                                        anchors.centerIn: parent
                                        text: tileSlot.modelData.status || ""
                                        role: "mono"
                                        size: 11
                                        color: Theme.graphite ? Theme.surfaceContent : tileSlot.pal.fg
                                    }
                                }

                                // Quick "Test" button in top-right corner
                                Rectangle {
                                    id: testBtn
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 28; height: 28
                                    radius: 14
                                    color: testHover.hovered
                                        ? (Theme.graphite ? Theme.surfaceContainerHighest : Qt.rgba(0, 0, 0, Theme.dark ? 0.35 : 0.14))
                                        : "transparent"
                                    opacity: tileHover.hovered || testHover.hovered ? 1 : 0.65
                                    Accessible.role: Accessible.Button
                                    Accessible.name: qsTr("Test %1").arg(tileSlot.modelData.label)
                                    Accessible.onPressAction: DeckController.testTile(tileSlot.modelData.id)

                                    Icon {
                                        anchors.centerIn: parent
                                        width: 14; height: 14
                                        path: Icons.bolt
                                        color: Theme.graphite ? Theme.surfaceContentVariant : tileSlot.pal.fg
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
                                anchors.leftMargin: Theme.graphite ? 18 : 14
                                spacing: 2

                                Txt {
                                    width: parent.width
                                    text: tileSlot.modelData.label
                                    role: "title"
                                    size: 15
                                    elide: Text.ElideRight
                                    color: Theme.graphite ? Theme.surfaceContent : tileSlot.pal.fg
                                }
                                Txt {
                                    width: parent.width
                                    text: tileSlot.modelData.subtitle || ""
                                    role: "bodySmall"
                                    size: 11
                                    elide: Text.ElideRight
                                    color: Theme.graphite ? Theme.surfaceContentVariant : tileSlot.pal.fg
                                    opacity: Theme.graphite ? 1.0 : 0.76
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
                    height: 118
                    radius: Theme.radiusMd
                    color: addHover.hovered ? Theme.surfaceContainerHigh : "transparent"
                    border.width: 1
                    border.color: Theme.outlineVariant

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

        readonly property var previewPal: page.tilePalette(tileColor)

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
                        text: qsTr("Phones only receive the tile's label, icon, color, and action kind. File paths and commands stay on this PC.")
                    }
                }

                // Live miniature tile preview
                Rectangle {
                    id: previewBox
                    width: 148; height: 64
                    radius: Theme.radiusMd
                    color: Theme.graphite ? Theme.surfaceContainer : editorSheet.previewPal.bg
                    border.width: 1
                    border.color: Theme.graphite ? editorSheet.previewPal.accent : editorSheet.previewPal.border

                    Icon {
                        x: 12; y: 10
                        width: 16; height: 16
                        path: page.iconPathFor(editorSheet.tileIcon, editorSheet.actionKind)
                        color: Theme.graphite ? editorSheet.previewPal.accent : editorSheet.previewPal.fg
                    }
                    Txt {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        anchors.margins: 10
                        text: editorSheet.tileLabel.length > 0 ? editorSheet.tileLabel : DeckController.defaultLabelFor(editorSheet.actionKind)
                        role: "title"
                        size: 13
                        elide: Text.ElideRight
                        color: Theme.graphite ? Theme.surfaceContent : editorSheet.previewPal.fg
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
                            selected: editorSheet.actionKind === modelData.id
                            text: modelData.label
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: editorSheet.selectKind(modelData.id) }
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
                        selected: editorSheet.modCtrl
                        text: "Ctrl"
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: editorSheet.modCtrl = !editorSheet.modCtrl }
                    }
                    Chip {
                        selected: editorSheet.modAlt
                        text: "Alt"
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: editorSheet.modAlt = !editorSheet.modAlt }
                    }
                    Chip {
                        selected: editorSheet.modShift
                        text: "Shift"
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: editorSheet.modShift = !editorSheet.modShift }
                    }
                    Chip {
                        selected: editorSheet.modWin
                        text: "Win"
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: editorSheet.modWin = !editorSheet.modWin }
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

            // Tile Label Input + Color Swatches side-by-side
            Row {
                width: parent.width
                spacing: 16

                Column {
                    width: parent.width - colorCol.width - 16
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
                        }
                    }
                }

                Column {
                    id: colorCol
                    spacing: 6
                    Txt { text: qsTr("Color"); role: "label"; muted: true }
                    Row {
                        height: 36
                        spacing: 8
                        Repeater {
                            model: page.colorChoices
                            delegate: Rectangle {
                                required property var modelData
                                readonly property var pal: page.tilePalette(modelData.id)
                                readonly property bool chosen: editorSheet.tileColor === modelData.id
                                anchors.verticalCenter: parent.verticalCenter
                                width: 26; height: 26; radius: 13
                                color: pal.accent
                                border.width: chosen ? 3 : 1
                                border.color: chosen ? Theme.surfaceContent : pal.border
                                Accessible.role: Accessible.RadioButton
                                Accessible.name: modelData.label
                                Accessible.checked: chosen
                                Accessible.onPressAction: editorSheet.tileColor = modelData.id
                                HoverHandler { cursorShape: Qt.PointingHandCursor }
                                TapHandler { onTapped: editorSheet.tileColor = modelData.id }
                            }
                        }
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
                            required property string modelData
                            readonly property bool chosen: editorSheet.tileIcon === modelData
                            width: 27; height: 27
                            radius: Theme.radiusSm
                            color: chosen ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                            border.width: chosen ? 2 : 0
                            border.color: Theme.primary
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
                }
            }

            Row {
                anchors.right: parent.right
                spacing: 8
                Button { variant: "text"; text: qsTr("Cancel"); onClicked: pageNameSheet.close() }
                Button {
                    variant: "fill"
                    text: pageNameSheet.isNew ? qsTr("Create") : qsTr("Save")
                    onClicked: {
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
