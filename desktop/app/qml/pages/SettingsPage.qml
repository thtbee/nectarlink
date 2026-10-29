// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Settings: two-pane categorized layout for appearance, general behavior,
// paired devices & permissions, camera & audio, notifications, files & sync,
// shortcuts & rules, storage & privacy, and connection & about.
Item {
    id: page
    property bool active: false
    property string currentSection: "appearance"
    property int selectedDeviceIndex: 0
    signal pairRequested

    readonly property bool wideLayout: page.width >= 680

    readonly property var categories: [
        { id: "appearance", title: qsTr("Appearance"), icon: Icons.palette, subtitle: qsTr("Theme, mode & colors") },
        { id: "general", title: qsTr("General"), icon: Icons.laptop, subtitle: qsTr("Startup, clipboard & desktop") },
        { id: "devices", title: qsTr("Devices"), icon: Icons.phone, subtitle: qsTr("Paired phones & permissions") },
        { id: "camera_audio", title: qsTr("Camera & Audio"), icon: Icons.video, subtitle: qsTr("Webcam, speaker & calls") },
        { id: "notifications", title: qsTr("Notifications"), icon: Icons.bell, subtitle: qsTr("Do not disturb & app rules") },
        { id: "files_sync", title: qsTr("Files & Sync"), icon: Icons.folder, subtitle: qsTr("Folder sync, storage & recordings") },
        { id: "shortcuts", title: qsTr("Shortcuts & Rules"), icon: Icons.bolt, subtitle: qsTr("Hotkeys & automations") },
        { id: "storage", title: qsTr("Storage & Privacy"), icon: Icons.clipboardList, subtitle: qsTr("History, caches & retention") },
        { id: "connection", title: qsTr("Connection & About"), icon: Icons.wifi, subtitle: qsTr("Doctor, Wake-on-LAN, API & version") }
    ]

    // The languages the app speaks besides the system's ({ code, name }).
    readonly property var languages: { try { return JSON.parse(Preferences.languages()) } catch (e) { return [] } }
    readonly property var activeCategoryObj: {
        for (let i = 0; i < categories.length; i++) {
            if (categories[i].id === page.currentSection)
                return categories[i]
        }
        return categories[0]
    }

    function syncSectionSideEffects() {
        Webcam.setPreviewVisible("settings", page.active && page.currentSection === "camera_audio"
                                 && page.Window.window !== null && page.Window.window.visible
                                 && page.Window.window.visibility !== Window.Minimized)
        if (page.active && (page.currentSection === "storage" || page.currentSection === "appearance"))
            AppController.refreshDataRetention()
    }

    function formatTrigger(t) {
        switch (t) {
        case "phone_connected": return qsTr("When phone connects")
        case "phone_disconnected": return qsTr("When phone disconnects")
        case "call_ringing": return qsTr("When phone rings")
        case "call_ended": return qsTr("When call ends")
        case "battery_low": return qsTr("When phone battery is low")
        case "battery_charged": return qsTr("When phone is charged")
        default: return (t || "").replace(/_/g, " ")
        }
    }

    function formatAction(a) {
        switch (a) {
        case "lock_pc": return qsTr("Lock this PC")
        case "sleep_pc": return qsTr("Sleep this PC")
        case "pause_pc_media":
        case "pause_media": return qsTr("Pause PC media")
        case "resume_pc_media":
        case "resume_media": return qsTr("Resume PC media")
        case "mute_pc": return qsTr("Mute PC audio")
        case "unmute_pc": return qsTr("Unmute PC audio")
        case "run_command": return qsTr("Run command")
        case "notify":
        case "show_toast": return qsTr("Show notification")
        default: return (a || "").replace(/_/g, " ")
        }
    }

    function formatKeyCombo(event) {
        if (event.key === Qt.Key_Control || event.key === Qt.Key_Shift
            || event.key === Qt.Key_Alt || event.key === Qt.Key_Meta
            || event.key === Qt.Key_unknown)
            return ""
        if (event.key === Qt.Key_Escape || event.key === Qt.Key_Tab || event.key === Qt.Key_Backtab)
            return ""
        const mods = []
        if (event.modifiers & Qt.ControlModifier) mods.push("Ctrl")
        if (event.modifiers & Qt.AltModifier) mods.push("Alt")
        if (event.modifiers & Qt.ShiftModifier) mods.push("Shift")
        if (event.modifiers & Qt.MetaModifier) mods.push(Qt.platform.os === "linux" ? "Super" : "Win")
        if (mods.length === 0)
            return ""
        let keyName = ""
        if (event.key === Qt.Key_Space) keyName = "Space"
        else if (event.key >= Qt.Key_A && event.key <= Qt.Key_Z)
            keyName = String.fromCharCode(event.key)
        else if (event.key >= Qt.Key_0 && event.key <= Qt.Key_9)
            keyName = String.fromCharCode(event.key)
        else if (event.key >= Qt.Key_F1 && event.key <= Qt.Key_F12)
            keyName = "F" + (event.key - Qt.Key_F1 + 1)
        if (keyName.length === 0)
            return ""
        return mods.join("+") + "+" + keyName
    }

    onActiveChanged: syncSectionSideEffects()
    onCurrentSectionChanged: {
        contentFlickable.contentY = 0
        syncSectionSideEffects()
        if (!Theme.reduceMotion) {
            column.opacity = 0.35
            sectionTranslate.y = 5
            Qt.callLater(() => sectionAnim.restart())
        } else {
            column.opacity = 1.0
            sectionTranslate.y = 0
        }
    }
    Component.onCompleted: {
        if (active)
            syncSectionSideEffects()
    }
    Component.onDestruction: Webcam.setPreviewVisible("settings", false)

    Connections {
        target: page.Window.window
        function onVisibilityChanged() { page.syncSectionSideEffects() }
    }

    Connections {
        target: DeviceList
        function onCountChanged() {
            if (page.selectedDeviceIndex >= DeviceList.count)
                page.selectedDeviceIndex = Math.max(0, DeviceList.count - 1)
        }
    }

    readonly property var retentionSummary: {
        try {
            return JSON.parse(AppController.dataRetentionSummary || "{}")
        } catch (e) {
            return ({})
        }
    }

    function formatRetentionBytes(bytes) {
        const b = Number(bytes || 0)
        const locale = Qt.locale()
        if (b <= 0)
            return qsTr("%1 B").arg(0)
        if (b < 1024)
            return qsTr("%1 B").arg(b)
        if (b < 1024 * 1024)
            return qsTr("%1 KB").arg(Math.max(1, Math.round(b / 1024)))
        if (b < 1024 * 1024 * 1024)
            return qsTr("%1 MB").arg(locale.toString(b / (1024 * 1024), "f", 1))
        if (b < 1024 * 1024 * 1024 * 1024)
            return qsTr("%1 GB").arg(locale.toString(b / (1024 * 1024 * 1024), "f", 1))
        return qsTr("%1 TB").arg(locale.toString(b / (1024 * 1024 * 1024 * 1024), "f", 1))
    }

    visible: active

    readonly property var toggleNames: ({
        notifications: qsTr("Notifications"),
        messages: qsTr("Messages"),
        calls: qsTr("Calls"),
        contacts: qsTr("Contacts"),
        clipboard: qsTr("Clipboard"),
        files: qsTr("Files"),
        recordings: qsTr("Voice recordings"),
        media: qsTr("Media controls"),
        photos: qsTr("Photos"),
        toggles: qsTr("Phone controls"),
        pc_actions: qsTr("Lock, sleep & wake PC"),
        mirroring: qsTr("Screen mirroring"),
        pc_mirror: qsTr("Share PC screen to phone"),
        webcam: qsTr("Webcam"),
        storage: Qt.platform.os === "linux" ? qsTr("File manager integration") : qsTr("File Explorer integration"),
        remote_input: qsTr("Remote mouse, keyboard & Deck"),
        commands: qsTr("Run Deck shell commands"),
        remote_files: qsTr("Remote PC file access")
    })

    readonly property var toggleIcons: ({
        notifications: Icons.bell,
        messages: Icons.messages,
        calls: Icons.call,
        contacts: Icons.person,
        clipboard: Icons.clipboard,
        files: Icons.send,
        recordings: Icons.mic,
        media: Icons.music,
        photos: Icons.photo,
        toggles: Icons.phone,
        pc_actions: Icons.lock,
        mirroring: Icons.mirror,
        pc_mirror: Icons.laptop,
        webcam: Icons.video,
        storage: Icons.folder,
        remote_input: Icons.deck,
        commands: Icons.terminal,
        remote_files: Icons.globe
    })

    readonly property var toggleDescriptions: ({
        pc_mirror: qsTr("Allow this phone to view this PC's screen."),
        commands: qsTr("Only enable for a phone you trust.")
    })

    readonly property var toggleGroups: [
        {
            title: qsTr("Calls, messages & notifications"),
            keys: ["notifications", "messages", "calls", "contacts"]
        },
        {
            title: qsTr("Files, photos & clipboard"),
            keys: ["clipboard", "files", "photos", "recordings", "storage", "remote_files"]
        },
        {
            title: qsTr("Screen, camera & control"),
            keys: ["media", "toggles", "mirroring", "pc_mirror", "webcam", "pc_actions", "remote_input", "commands"]
        }
    ]

    function groupTogglesFor(allToggles, groupKeys, isLastGroup, allGroupKeys) {
        if (!allToggles || allToggles.length === 0)
            return []
        const res = []
        for (let i = 0; i < groupKeys.length; i++) {
            const k = groupKeys[i]
            for (let j = 0; j < allToggles.length; j++) {
                if (allToggles[j].name === k) {
                    res.push(allToggles[j])
                    break
                }
            }
        }
        if (isLastGroup) {
            for (let j = 0; j < allToggles.length; j++) {
                const name = allToggles[j].name
                if (allGroupKeys.indexOf(name) === -1)
                    res.push(allToggles[j])
            }
        }
        return res
    }

    readonly property var allGroupedKeys: [
        "notifications", "messages", "calls", "contacts",
        "clipboard", "files", "photos", "recordings", "storage", "remote_files",
        "media", "toggles", "mirroring", "pc_mirror", "webcam", "pc_actions", "remote_input", "commands"
    ]

    FolderDialog {
        id: photoBackupFolderPicker
        property string deviceId
        title: qsTr("Back up photos to")
        onAccepted: PhotoBackup.chooseFolder(deviceId, selectedFolder.toString())
    }

    FolderDialog {
        id: recordingsFolderPicker
        title: qsTr("Save recordings in")
        onAccepted: Preferences.chooseRecordingsFolder(selectedFolder.toString())
    }

    // ---- Left Category Navigation Rail (wide windows >= 680px) ----
    Flickable {
        id: categoryRail
        visible: page.wideLayout
        x: Theme.contentPadding
        y: Theme.contentPadding
        width: 232
        height: parent.height - Theme.contentPadding * 2
        contentHeight: railCol.height
        interactive: contentHeight > height
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: railCol
            width: parent.width
            spacing: 4
            property string hoveredCategoryId: ""

            Repeater {
                model: page.categories
                delegate: Rectangle {
                    id: navItem
                    required property var modelData
                    readonly property bool selected: page.currentSection === modelData.id
                    readonly property bool isHovered: railCol.hoveredCategoryId === modelData.id && navMouse.containsMouse
                    readonly property color fg: selected
                        ? (Theme.graphite ? Theme.surface : Theme.secondaryContainerContent)
                        : Theme.surfaceContent
                    width: railCol.width
                    height: 42
                    radius: Theme.radiusMd
                    activeFocusOnTab: true
                    color: selected
                        ? (Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainer)
                        : (isHovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, Theme.dark ? 0.06 : 0.04) : "transparent")
                    border.width: Theme.focusVisible(navItem) ? 2 : (Theme.graphite && selected ? 1 : 0)
                    border.color: Theme.focusVisible(navItem) ? Theme.primary : Theme.outlineVariant

                    Accessible.role: Accessible.PageTab
                    Accessible.name: modelData.title
                    Accessible.selected: selected
                    Accessible.onPressAction: page.currentSection = modelData.id
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            page.currentSection = navItem.modelData.id
                            event.accepted = true
                        }
                    }

                    Row {
                        anchors.left: parent.left
                        anchors.leftMargin: 12
                        anchors.right: navBadge.visible ? navBadge.left : parent.right
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 12
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 18; height: 18
                            path: navItem.modelData.icon
                            color: navItem.fg
                        }
                        Txt {
                            width: parent.width - 30
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            size: 13
                            text: navItem.modelData.title
                            color: navItem.fg
                            elide: Text.ElideRight
                        }
                    }

                    Rectangle {
                        id: navBadge
                        visible: navItem.modelData.id === "devices" && DeviceList.count > 0
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        height: 20
                        width: Math.max(20, navBadgeTxt.implicitWidth + 10)
                        radius: 10
                        color: navItem.selected
                            ? Qt.rgba(navItem.fg.r, navItem.fg.g, navItem.fg.b, 0.16)
                            : Theme.surfaceContainerHighest
                        Txt {
                            id: navBadgeTxt
                            anchors.centerIn: parent
                            role: "caption"
                            text: String(DeviceList.count)
                            color: navItem.selected ? navItem.fg : Theme.surfaceContentVariant
                        }
                    }

                    MouseArea {
                        id: navMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onContainsMouseChanged: {
                            if (containsMouse)
                                railCol.hoveredCategoryId = navItem.modelData.id
                            else if (railCol.hoveredCategoryId === navItem.modelData.id)
                                railCol.hoveredCategoryId = ""
                        }
                        onClicked: page.currentSection = navItem.modelData.id
                    }
                }
            }
        }
    }

    Rectangle {
        visible: page.wideLayout
        x: categoryRail.x + categoryRail.width + 16
        y: Theme.contentPadding
        width: 1
        height: parent.height - Theme.contentPadding * 2
        color: Theme.outlineVariant
        opacity: 0.6
    }

    // ---- Compact Top Category Pill Bar (narrow windows < 680px) ----
    Flickable {
        id: compactCategoryBar
        visible: !page.wideLayout
        x: Theme.contentPadding
        y: 12
        width: parent.width - Theme.contentPadding * 2
        height: 38
        contentWidth: compactRow.width
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Row {
            id: compactRow
            height: parent.height
            spacing: 6
            Repeater {
                model: page.categories
                delegate: Rectangle {
                    id: compactTab
                    required property var modelData
                    readonly property bool selected: page.currentSection === modelData.id
                    readonly property color fg: selected
                        ? (Theme.graphite ? Theme.surface : Theme.secondaryContainerContent)
                        : Theme.surfaceContent
                    height: 34
                    width: compactTabRow.implicitWidth + 22
                    anchors.verticalCenter: parent.verticalCenter
                    radius: Theme.pill(height)
                    color: selected
                        ? (Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainer)
                        : Theme.surfaceContainerHigh
                    border.width: Theme.graphite ? 1 : 0
                    border.color: Theme.outlineVariant
                    Row {
                        id: compactTabRow
                        anchors.centerIn: parent
                        spacing: 6
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 15; height: 15
                            path: compactTab.modelData.icon
                            color: compactTab.fg
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            size: 12
                            text: compactTab.modelData.title
                            color: compactTab.fg
                        }
                    }
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: page.currentSection = compactTab.modelData.id }
                }
            }
        }
    }

    // ---- Right Content Pane ----
    Flickable {
        id: contentFlickable
        x: page.wideLayout ? (categoryRail.x + categoryRail.width + 33) : 0
        y: page.wideLayout ? 0 : (compactCategoryBar.y + compactCategoryBar.height + 8)
        width: parent.width - x
        height: parent.height - y
        contentHeight: column.height + Theme.contentPadding * 2
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: column
            width: Math.min(840, contentFlickable.width - Theme.contentPadding * 2)
            x: Math.max(Theme.contentPadding, Math.round((contentFlickable.width - width) / 2))
            y: Theme.contentPadding
            spacing: Theme.gutter
            transform: Translate { id: sectionTranslate; y: 0 }

            ParallelAnimation {
                id: sectionAnim
                NumberAnimation {
                    target: column
                    property: "opacity"
                    from: 0.45
                    to: 1.0
                    duration: 120
                    easing.type: Easing.OutCubic
                }
                NumberAnimation {
                    target: sectionTranslate
                    property: "y"
                    from: 5
                    to: 0
                    duration: 135
                    easing.type: Easing.OutCubic
                }
            }

            // Section Title Header
            Item {
                width: parent.width
                height: Math.max(sectionHeaderTitleTxt.implicitHeight, pairNewHeaderBtn.visible ? pairNewHeaderBtn.height : 0)
                Txt {
                    id: sectionHeaderTitleTxt
                    anchors.left: parent.left
                    anchors.right: pairNewHeaderBtn.visible ? pairNewHeaderBtn.left : parent.right
                    anchors.rightMargin: pairNewHeaderBtn.visible ? 12 : 0
                    anchors.verticalCenter: parent.verticalCenter
                    text: page.activeCategoryObj.title
                    role: "headline"
                    elide: Text.ElideRight
                }
                Button {
                    id: pairNewHeaderBtn
                    visible: page.currentSection === "devices"
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.plus
                    text: qsTr("Pair a new device")
                    onClicked: page.pairRequested()
                }
            }

            // ================================================================
            // 1. APPEARANCE
            // ================================================================
            Column {
                visible: page.currentSection === "appearance"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.palette
                            title: qsTr("Theme")
                            Segmented {
                                options: [
                                    { value: "hive", label: qsTr("Hive") },
                                    { value: "bloom", label: qsTr("Bloom") },
                                    { value: "glass", label: qsTr("Glass") },
                                    { value: "graphite", label: qsTr("Graphite") }
                                ]
                                value: Theme.hive ? "hive" : (Theme.glass ? "glass" : Preferences.theme)
                                onPicked: (value) => Preferences.theme = value
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.moon
                            title: qsTr("Mode")
                            Segmented {
                                options: [
                                    { value: "system", label: qsTr("System") },
                                    { value: "light", label: qsTr("Light") },
                                    { value: "dark", label: qsTr("Dark") }
                                ]
                                value: Preferences.colorMode
                                onPicked: (value) => Preferences.colorMode = value
                            }
                        }
                        // Hive's colors are fixed: no seed to pick.
                        Divider { width: parent.width; visible: !Theme.graphite && !Theme.hive }
                        ListRow {
                            id: colorRow
                            width: parent.width
                            visible: !Theme.graphite && !Theme.hive
                            stackBelow: true
                            iconPath: Icons.colorSwatch
                            title: qsTr("Color")
                            Column {
                                id: colorControl
                                width: Math.max(
                                    swatchesRow.implicitWidth,
                                    phonePillsRow.implicitWidth
                                )
                                spacing: 8

                                Row {
                                    id: swatchesRow
                                    anchors.right: colorRow.stackTrailing ? undefined : parent.right
                                    spacing: 8

                                    // Wallpaper swatch
                                    Rectangle {
                                        id: wallpaperSwatch
                                        readonly property bool selected: Preferences.seed === "wallpaper"
                                        readonly property var wallpaperPalette: Theme.wallpaperSeed
                                            ? Theme.wallpaperSeed[Theme.dark ? "dark" : "light"] : null
                                        width: 28; height: 28; radius: 14
                                        activeFocusOnTab: true
                                        color: wallpaperPalette ? wallpaperPalette.primary : Theme.surfaceContainerHighest
                                        border.width: selected || Theme.focusVisible(wallpaperSwatch) ? 2 : 0
                                        border.color: Theme.focusVisible(wallpaperSwatch) ? Theme.primary : Theme.surfaceContent
                                        Accessible.role: Accessible.RadioButton
                                        Accessible.name: qsTr("Wallpaper")
                                        Accessible.checked: selected
                                        Accessible.onPressAction: Preferences.seed = "wallpaper"
                                        Keys.onPressed: (event) => {
                                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                Preferences.seed = "wallpaper"
                                                event.accepted = true
                                            }
                                        }
                                        Icon {
                                            anchors.centerIn: parent
                                            width: 14; height: 14
                                            stroke: 2
                                            path: Icons.photo
                                            color: wallpaperSwatch.wallpaperPalette ? wallpaperSwatch.wallpaperPalette.onPrimary : Theme.surfaceContent
                                        }
                                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                                        TapHandler { onTapped: Preferences.seed = "wallpaper" }
                                    }

                                    // Preset seeds
                                    Repeater {
                                        model: Object.keys(Tokens.data.themes.bloom.seeds)
                                        delegate: Rectangle {
                                            id: swatch
                                            required property string modelData
                                            readonly property bool selected: Preferences.seed === modelData
                                                || (!Theme.isPhoneSeed && !Theme.isArtworkSeed && Preferences.seed !== "wallpaper" && Theme.seed === modelData)
                                            width: 28; height: 28; radius: 14
                                            activeFocusOnTab: true
                                            color: Tokens.data.themes.bloom.seeds[modelData].seed
                                            border.width: selected || Theme.focusVisible(swatch) ? 2 : 0
                                            border.color: Theme.focusVisible(swatch) ? Theme.primary : Theme.surfaceContent
                                            Accessible.role: Accessible.RadioButton
                                            Accessible.name: modelData.charAt(0).toUpperCase() + modelData.slice(1)
                                            Accessible.checked: selected
                                            Accessible.onPressAction: Preferences.seed = modelData
                                            Keys.onPressed: (event) => {
                                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                    Preferences.seed = swatch.modelData
                                                    event.accepted = true
                                                }
                                            }
                                            Icon {
                                                anchors.centerIn: parent
                                                width: 14; height: 14
                                                stroke: 2
                                                path: Icons.check
                                                color: "white"
                                                visible: swatch.selected
                                            }
                                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                                            TapHandler { onTapped: Preferences.seed = swatch.modelData }
                                        }
                                    }

                                    // Compact inline AMOLED toggle shown inside the Color row when Black seed is selected
                                    Rectangle {
                                        id: inlineAmoledPill
                                        visible: !Theme.graphite && Preferences.seed === "black"
                                        height: 28
                                        width: inlineAmoledRow.implicitWidth + 16
                                        radius: 14
                                        color: Theme.glass ? Theme.tileColor : Theme.surfaceContainerHigh
                                        border.width: Theme.glass || Preferences.amoled ? 1 : 0
                                        border.color: Preferences.amoled ? Theme.primary : Theme.cardBorder

                                        Row {
                                            id: inlineAmoledRow
                                            anchors.centerIn: parent
                                            spacing: 6

                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "caption"
                                                weight: 600
                                                text: qsTr("AMOLED")
                                                color: Preferences.amoled ? Theme.primary : Theme.surfaceContentVariant
                                            }

                                            Toggle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                scale: 0.78
                                                checked: Preferences.amoled
                                                label: qsTr("Pure black (AMOLED)")
                                                onToggled: (on) => Preferences.amoled = on
                                            }
                                        }
                                    }
                                }

                                // Dynamic pills row: Playing media artwork + paired phone color pills
                                Row {
                                    id: phonePillsRow
                                    anchors.right: colorRow.stackTrailing ? undefined : parent.right
                                    spacing: 8

                                    // Media artwork dynamic sync pill
                                    Rectangle {
                                        id: artworkSwatch
                                        readonly property bool selected: Theme.isArtworkSeed
                                        readonly property var artworkPalette: Theme.artworkSeed
                                            ? Theme.artworkSeed[Theme.dark ? "dark" : "light"] : null
                                        height: 28
                                        width: artworkSwatchRow.implicitWidth + 14
                                        radius: Theme.pill(height)
                                        activeFocusOnTab: true
                                        color: selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                                        border.width: selected || Theme.focusVisible(artworkSwatch) ? 2 : 0
                                        border.color: Theme.focusVisible(artworkSwatch) ? Theme.primary : Theme.surfaceContent
                                        Accessible.role: Accessible.RadioButton
                                        Accessible.name: qsTr("Media artwork")
                                        Accessible.checked: selected
                                        Accessible.onPressAction: Preferences.seed = "artwork"
                                        Keys.onPressed: (event) => {
                                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                Preferences.seed = "artwork"
                                                event.accepted = true
                                            }
                                        }
                                        Row {
                                            id: artworkSwatchRow
                                            anchors.centerIn: parent
                                            spacing: 6
                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: 18; height: 18; radius: 9
                                                color: Theme.artworkSeed && Theme.artworkSeed.seed
                                                    ? Theme.artworkSeed.seed
                                                    : (artworkSwatch.artworkPalette ? artworkSwatch.artworkPalette.primary : Theme.primary)
                                                Icon {
                                                    anchors.centerIn: parent
                                                    width: 11; height: 11
                                                    stroke: 2
                                                    path: Icons.music
                                                    color: "white"
                                                }
                                            }
                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "label"
                                                size: 12
                                                text: qsTr("Media artwork")
                                                color: artworkSwatch.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                                            }
                                        }
                                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                                        TapHandler { onTapped: Preferences.seed = "artwork" }
                                    }

                                    Repeater {
                                        model: Theme.phoneSeeds
                                        delegate: Rectangle {
                                            id: phoneSwatch
                                            required property var modelData
                                            required property int index
                                            readonly property bool selected: Preferences.seed === modelData.key
                                                || (Preferences.seed === "phone" && index === 0)
                                                || (modelData.id.length > 0 && Preferences.seed === "phone:" + modelData.id)
                                            readonly property var phonePalette: modelData.scheme
                                                ? modelData.scheme[Theme.dark ? "dark" : "light"] : null
                                            height: 28
                                            width: phoneSwatchRow.implicitWidth + 14
                                            radius: Theme.pill(height)
                                            activeFocusOnTab: true
                                            color: selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                                            border.width: selected || Theme.focusVisible(phoneSwatch) ? 2 : 0
                                            border.color: Theme.focusVisible(phoneSwatch) ? Theme.primary : Theme.surfaceContent
                                            Accessible.role: Accessible.RadioButton
                                            Accessible.name: modelData.label
                                            Accessible.checked: selected
                                            Accessible.onPressAction: Preferences.seed = modelData.key
                                            Keys.onPressed: (event) => {
                                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                    Preferences.seed = phoneSwatch.modelData.key
                                                    event.accepted = true
                                                }
                                            }
                                            Row {
                                                id: phoneSwatchRow
                                                anchors.centerIn: parent
                                                spacing: 6
                                                Rectangle {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    width: 18; height: 18; radius: 9
                                                    color: phoneSwatch.modelData.scheme && phoneSwatch.modelData.scheme.seed
                                                        ? phoneSwatch.modelData.scheme.seed
                                                        : (phoneSwatch.phonePalette ? phoneSwatch.phonePalette.primary : Theme.primary)
                                                    Icon {
                                                        anchors.centerIn: parent
                                                        width: 11; height: 11
                                                        stroke: 2
                                                        path: Icons.phone
                                                        color: "white"
                                                    }
                                                }
                                                Txt {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    role: "label"
                                                    size: 12
                                                    text: phoneSwatch.modelData.label
                                                    color: phoneSwatch.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                                                }
                                            }
                                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                                            TapHandler { onTapped: Preferences.seed = phoneSwatch.modelData.key }
                                        }
                                    }
                                }
                            }
                        }
                        Divider { width: parent.width }

                        // Hive: the comb's motion instead of Mica (Hive keeps its own colors).
                        ListRow {
                            width: parent.width
                            visible: Theme.hive
                            iconPath: Icons.sparkle
                            title: qsTr("Animated comb")
                            description: AppController.reduceMotion
                                ? (Qt.platform.os === "linux"
                                    ? qsTr("Still while animations are off in system settings.")
                                    : qsTr("Still while animations are off in Windows."))
                                : (AppController.powerSaver
                                    ? qsTr("Still while energy saver is on.")
                                    : qsTr("The honeycomb breathes and pulses."))
                            Toggle {
                                label: qsTr("Animated comb")
                                checked: Preferences.combMotion
                                onToggled: (on) => Preferences.combMotion = on
                            }
                        }

                        // Side-by-side when Graphite Dark shows Pure Black + Mica backdrop
                        Item {
                            id: appearanceBottomPair
                            readonly property bool showGraphiteAmoled: Theme.graphite && Theme.dark
                            readonly property bool showMica: Qt.platform.os === "windows"
                                && !!Window.window && Window.window.systemBackdropSupported === true
                            visible: !Theme.hive && (showMica || showGraphiteAmoled)
                            width: parent.width
                            readonly property bool splitCols: showGraphiteAmoled && showMica && width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: !showGraphiteAmoled
                                ? (showMica ? micaRow.implicitHeight : 0)
                                : (!showMica
                                    ? graphiteAmoledRow.implicitHeight
                                    : (splitCols
                                        ? Math.max(graphiteAmoledRow.implicitHeight, micaRow.implicitHeight)
                                        : (graphiteAmoledRow.implicitHeight + 1 + micaRow.implicitHeight)))

                            ListRow {
                                id: graphiteAmoledRow
                                visible: appearanceBottomPair.showGraphiteAmoled
                                x: 0
                                y: appearanceBottomPair.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: appearanceBottomPair.colWidth
                                iconPath: Icons.moon
                                title: qsTr("Pure black (AMOLED)")
                                description: qsTr("Pitch-black #000000 surfaces.")
                                Toggle {
                                    checked: Preferences.amoled
                                    label: qsTr("Pure black (AMOLED)")
                                    onToggled: (on) => Preferences.amoled = on
                                }
                            }

                            Rectangle {
                                visible: appearanceBottomPair.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: appearanceBottomPair.showGraphiteAmoled && appearanceBottomPair.showMica && !appearanceBottomPair.splitCols
                                y: graphiteAmoledRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: micaRow
                                visible: appearanceBottomPair.showMica
                                x: appearanceBottomPair.splitCols ? (parent.width - width) : 0
                                y: !appearanceBottomPair.showGraphiteAmoled
                                    ? 0
                                    : (appearanceBottomPair.splitCols
                                        ? Math.round((parent.height - height) / 2)
                                        : (graphiteAmoledRow.implicitHeight + 1))
                                width: appearanceBottomPair.colWidth
                                iconPath: Icons.desktop
                                title: qsTr("Mica backdrop")
                                description: qsTr("Tint window with desktop wallpaper.")
                                Toggle {
                                    label: qsTr("Mica backdrop")
                                    checked: Preferences.backdrop
                                    onToggled: (on) => Preferences.backdrop = on
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 2. GENERAL
            // ================================================================
            Column {
                visible: page.currentSection === "general"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Language: the system's, English, or another one the app
                        // speaks, picked from a list (each named in its own language).
                        ListRow {
                            width: parent.width
                            iconPath: Icons.globe
                            title: qsTr("Language")
                            description: Qt.platform.os === "linux"
                                ? qsTr("NectarLink follows the system unless you pick one.")
                                : qsTr("NectarLink follows Windows unless you pick one.")
                            Segmented {
                                readonly property var other: page.languages.find(
                                    (l) => l.code === Preferences.language && l.code !== "en")
                                options: [
                                    { value: "", label: qsTr("System") },
                                    { value: "en", label: "English" },
                                    other ? { value: other.code, label: other.name }
                                          : { value: "more", label: qsTr("More") }
                                ]
                                value: Preferences.language
                                onPicked: (v) => {
                                    if (v === "" || v === "en")
                                        Preferences.language = v
                                    else
                                        languageSheet.open()
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 1: Start with Windows | Stay in system tray
                        Item {
                            id: startupPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(startWinRow.implicitHeight, stayTrayRow.implicitHeight)
                                : (startWinRow.implicitHeight + 1 + stayTrayRow.implicitHeight)

                            ListRow {
                                id: startWinRow
                                x: 0
                                y: startupPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: startupPairRow.colWidth
                                iconPath: Icons.power
                                title: Qt.platform.os === "linux" ? qsTr("Start at login") : qsTr("Start with Windows")
                                description: qsTr("Launch in the tray when you sign in.")
                                Toggle {
                                    label: Qt.platform.os === "linux" ? qsTr("Start at login") : qsTr("Start with Windows")
                                    checked: Preferences.startWithWindows
                                    onToggled: (on) => Preferences.startWithWindows = on
                                }
                            }

                            Rectangle {
                                visible: startupPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !startupPairRow.splitCols
                                y: startWinRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: stayTrayRow
                                x: startupPairRow.splitCols ? (parent.width - width) : 0
                                y: startupPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (startWinRow.implicitHeight + 1)
                                width: startupPairRow.colWidth
                                opacity: Qt.platform.os === "osx" ? 0.5 : 1.0
                                iconPath: Icons.window
                                title: qsTr("Stay in system tray")
                                description: qsTr("Keep running when the window is closed.")
                                Toggle {
                                    label: qsTr("Stay in system tray")
                                    checked: Preferences.closeToTray
                                    enabled: Qt.platform.os !== "osx"
                                    onToggled: (on) => {
                                        if (Qt.platform.os !== "osx")
                                            Preferences.closeToTray = on
                                    }
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 2: Nectar Island | Battery alerts
                        Item {
                            id: islandBatteryPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(islandRow.implicitHeight, batteryAlertsRow.implicitHeight)
                                : (islandRow.implicitHeight + 1 + batteryAlertsRow.implicitHeight)

                            ListRow {
                                id: islandRow
                                x: 0
                                y: islandBatteryPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: islandBatteryPairRow.colWidth
                                iconPath: Icons.island
                                title: qsTr("Nectar Island")
                                description: qsTr("Top-screen pill for calls, transfers & media.")
                                Toggle {
                                    label: qsTr("Nectar Island")
                                    checked: Preferences.nectarIsland
                                    onToggled: (on) => Preferences.nectarIsland = on
                                }
                            }

                            Rectangle {
                                visible: islandBatteryPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !islandBatteryPairRow.splitCols
                                y: islandRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: batteryAlertsRow
                                x: islandBatteryPairRow.splitCols ? (parent.width - width) : 0
                                y: islandBatteryPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (islandRow.implicitHeight + 1)
                                width: islandBatteryPairRow.colWidth
                                iconPath: Icons.battery
                                title: qsTr("Battery alerts")
                                description: qsTr("Notify when phone battery is low or full.")
                                Toggle {
                                    label: qsTr("Battery alerts")
                                    checked: Preferences.batteryAlerts
                                    onToggled: (on) => Preferences.batteryAlerts = on
                                }
                            }
                        }

                        // Sub-row: Granular Nectar Island triggers (Calls, Transfers, Media, Notifications)
                        Column {
                            id: islandTriggersBlock
                            visible: Preferences.nectarIsland
                            width: parent.width
                            spacing: 0

                            Divider { width: parent.width }

                            ListRow {
                                id: islandTriggersRow
                                width: parent.width
                                title: qsTr("Island triggers")
                                description: qsTr("Choose when the top pill appears.")

                                Row {
                                    id: islandTriggersRowItems
                                    spacing: 8

                                    Repeater {
                                        model: [
                                            { key: "calls", label: qsTr("Calls"), icon: Icons.call },
                                            { key: "transfers", label: qsTr("Transfers"), icon: Icons.send },
                                            { key: "media", label: qsTr("Media"), icon: Icons.music },
                                            { key: "notifications", label: qsTr("Alerts"), icon: Icons.bell }
                                        ]
                                        delegate: Rectangle {
                                            id: trigChip
                                            required property var modelData
                                            readonly property bool active: {
                                                if (modelData.key === "calls") return Preferences.nectarIslandCalls
                                                if (modelData.key === "transfers") return Preferences.nectarIslandTransfers
                                                if (modelData.key === "media") return Preferences.nectarIslandMedia
                                                return Preferences.nectarIslandNotifications
                                            }
                                            function toggleTrigger(nextVal) {
                                                if (modelData.key === "calls") Preferences.nectarIslandCalls = nextVal
                                                else if (modelData.key === "transfers") Preferences.nectarIslandTransfers = nextVal
                                                else if (modelData.key === "media") Preferences.nectarIslandMedia = nextVal
                                                else Preferences.nectarIslandNotifications = nextVal
                                            }
                                            height: 30
                                            width: trigChipRow.implicitWidth + 16
                                            radius: 15
                                            color: active
                                                ? (Theme.glass ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.16) : Theme.secondaryContainer)
                                                : (Theme.glass ? Theme.tileColor : Theme.surfaceContainerHigh)
                                            border.width: 1
                                            border.color: active ? Theme.primary : Theme.cardBorder

                                            Behavior on color { ColorAnimation { duration: 150 } }
                                            Behavior on border.color { ColorAnimation { duration: 150 } }

                                            Row {
                                                id: trigChipRow
                                                anchors.centerIn: parent
                                                spacing: 5

                                                Icon {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    width: 13; height: 13
                                                    stroke: 1.9
                                                    path: trigChip.modelData.icon
                                                    color: trigChip.active ? Theme.primary : Theme.surfaceContentVariant
                                                }

                                                Txt {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    role: "caption"
                                                    weight: 600
                                                    text: trigChip.modelData.label
                                                    color: trigChip.active ? Theme.surfaceContent : Theme.surfaceContentVariant
                                                }

                                                Toggle {
                                                    anchors.verticalCenter: parent.verticalCenter
                                                    scale: 0.72
                                                    checked: trigChip.active
                                                    label: trigChip.modelData.label
                                                    onToggled: (on) => trigChip.toggleTrigger(on)
                                                }
                                            }

                                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                                            TapHandler { onTapped: trigChip.toggleTrigger(!trigChip.active) }
                                        }
                                    }
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 3: Sync clipboard to phone
                        ListRow {
                            width: parent.width
                            iconPath: Icons.clipboard
                            title: qsTr("Sync clipboard to phone")
                            description: qsTr("Share copied text and images with your phone.")
                            Toggle {
                                label: qsTr("Sync clipboard to phone")
                                checked: Preferences.autoClipboard
                                onToggled: (on) => Preferences.autoClipboard = on
                            }
                        }

                        Divider { width: parent.width }

                        ListRow {
                            width: parent.width
                            iconPath: Icons.clipboard
                            title: qsTr("Clipboard follows control")
                            description: qsTr("Send your clipboard to the phone when you start controlling it, or when the mouse moves into its window.")
                            Toggle {
                                label: qsTr("Clipboard follows control")
                                checked: Preferences.flowFollowClipboard
                                onToggled: (on) => Preferences.flowFollowClipboard = on
                            }
                        }

                        Divider { width: parent.width }

                        // Row 4: Copy one-time codes | Copy received files
                        Item {
                            id: copyPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(copyOtpRow.implicitHeight, copyFilesRow.implicitHeight)
                                : (copyOtpRow.implicitHeight + 1 + copyFilesRow.implicitHeight)

                            ListRow {
                                id: copyOtpRow
                                x: 0
                                y: copyPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: copyPairRow.colWidth
                                iconPath: Icons.lock
                                title: qsTr("Copy one-time codes")
                                description: qsTr("Auto-copy verification codes from texts.")
                                Toggle {
                                    label: qsTr("Copy one-time codes")
                                    checked: Preferences.autoCopyOtp
                                    onToggled: (on) => Preferences.autoCopyOtp = on
                                }
                            }

                            Rectangle {
                                visible: copyPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !copyPairRow.splitCols
                                y: copyOtpRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: copyFilesRow
                                x: copyPairRow.splitCols ? (parent.width - width) : 0
                                y: copyPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (copyOtpRow.implicitHeight + 1)
                                width: copyPairRow.colWidth
                                iconPath: Icons.copy
                                title: qsTr("Copy received files")
                                description: qsTr("Ready to Ctrl+V incoming files anywhere.")
                                Toggle {
                                    label: qsTr("Copy received files")
                                    checked: Preferences.clipboardAutoCopyReceivedFiles
                                    onToggled: (on) => Preferences.clipboardAutoCopyReceivedFiles = on
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 5: Suggest clipboard actions
                        ListRow {
                            width: parent.width
                            iconPath: Icons.openExternal
                            title: qsTr("Suggest clipboard actions")
                            description: qsTr("Open links, maps, numbers & emails from phone clips.")
                            Toggle {
                                label: qsTr("Suggest clipboard actions")
                                checked: Preferences.suggestClipboardActions
                                onToggled: (on) => Preferences.suggestClipboardActions = on
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 3. DEVICES
            // ================================================================
            Column {
                visible: page.currentSection === "devices"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                // Device Selector Pill Bar when more than 1 phone is paired
                Flow {
                    width: parent.width
                    visible: DeviceList.count > 1
                    spacing: 8
                    Repeater {
                        model: page.currentSection === "devices" ? DeviceList : null
                        delegate: Rectangle {
                            id: deviceTabPill
                            required property int index
                            required property string deviceId
                            required property string name
                            required property bool online
                            readonly property bool selected: page.selectedDeviceIndex === index
                            readonly property color fg: selected
                                ? (Theme.graphite ? Theme.surface : Theme.secondaryContainerContent)
                                : Theme.surfaceContent
                            height: 36
                            width: deviceTabRow.implicitWidth + 24
                            radius: Theme.pill(height)
                            activeFocusOnTab: true
                            color: selected
                                ? (Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainer)
                                : Theme.surfaceContainerHigh
                            border.width: Theme.focusVisible(deviceTabPill) ? 2 : (Theme.graphite || selected ? 1 : 0)
                            border.color: Theme.focusVisible(deviceTabPill) ? Theme.primary : (selected ? Theme.primary : Theme.outlineVariant)
                            Row {
                                id: deviceTabRow
                                anchors.centerIn: parent
                                spacing: 8
                                StatusDot {
                                    anchors.verticalCenter: parent.verticalCenter
                                    online: deviceTabPill.online
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "label"
                                    text: deviceTabPill.name
                                    color: deviceTabPill.fg
                                }
                            }
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: page.selectedDeviceIndex = deviceTabPill.index }
                        }
                    }
                }

                // Empty state when 0 devices are paired
                Card {
                    width: parent.width
                    visible: DeviceList.count === 0
                    Column {
                        width: parent.width
                        spacing: 10
                        Txt {
                            width: parent.width
                            text: qsTr("No phones paired yet")
                            role: "title"
                        }
                        Txt {
                            width: parent.width
                            text: qsTr("Pair your phone to get started.")
                            role: "bodySmall"
                            muted: true
                            wrapMode: Text.WordWrap
                        }
                        Button {
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.plus
                            text: qsTr("Pair a phone")
                            onClicked: page.pairRequested()
                        }
                    }
                }

                // Selected device card (only the selected device is shown at a time)
                Repeater {
                    model: page.currentSection === "devices" ? DeviceList : null
                    delegate: Card {
                        id: deviceCard
                        required property int index
                        required property string deviceId
                        required property string name
                        required property bool online
                        visible: index === page.selectedDeviceIndex
                        width: visible ? column.width : 0

                        // Re-read when toggles (and so capabilities) change.
                        readonly property var toggles: AppController.capsRevision >= 0 && AppController.togglesRevision >= 0
                                                       ? AppController.deviceToggles(deviceId) : []

                        Column {
                            width: parent.width
                            spacing: 14
                            Row {
                                width: parent.width
                                spacing: 12
                                Avatar { iconPath: Icons.phone; emphasized: deviceCard.online }
                                Column {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: parent.width - 36 - 12 - unpair.width - 12
                                    Txt { width: parent.width; text: deviceCard.name; role: "title"; elide: Text.ElideRight }
                                    Row {
                                        spacing: 6
                                        StatusDot {
                                            anchors.verticalCenter: parent.verticalCenter
                                            online: deviceCard.online
                                        }
                                        Txt {
                                            anchors.verticalCenter: parent.verticalCenter
                                            text: deviceCard.online ? qsTr("Connected") : qsTr("Not connected")
                                            role: "bodySmall"
                                            muted: true
                                        }
                                    }
                                }
                                Button {
                                    id: unpair
                                    anchors.verticalCenter: parent.verticalCenter
                                    variant: "outline"
                                    size: "sm"
                                    iconPath: Icons.unlink
                                    text: qsTr("Unpair")
                                    onClicked: {
                                        unpairSheet.deviceId = deviceCard.deviceId
                                        unpairSheet.deviceName = deviceCard.name
                                        unpairSheet.open()
                                    }
                                }
                            }
                            // Instant Hotspot: the saved Wi-Fi network the PC joins
                            // after this phone turns its hotspot on.
                            ListRow {
                                id: hotspotRow
                                width: parent.width
                                property string network: AppController.hotspotNetwork(deviceCard.deviceId)
                                iconPath: Icons.wifi
                                title: qsTr("Join this phone's hotspot")
                                description: hotspotRow.network.length > 0
                                    ? qsTr("When Instant Hotspot turns it on, this PC joins %1.").arg(hotspotRow.network)
                                    : (Qt.platform.os === "linux"
                                        ? qsTr("Join the phone's hotspot once from your system Wi-Fi list, then choose it here.")
                                        : qsTr("Join the phone's hotspot once from Windows' Wi-Fi list, then choose it here."))
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    text: qsTr("Choose network")
                                    onClicked: {
                                        hotspotSheet.deviceId = deviceCard.deviceId
                                        hotspotSheet.row = hotspotRow
                                        hotspotSheet.networks = JSON.parse(AppController.wifiNetworks() || "[]")
                                        hotspotSheet.open()
                                    }
                                }
                            }
                            Repeater {
                                model: page.toggleGroups
                                delegate: Column {
                                    id: groupCol
                                    required property var modelData
                                    required property int index
                                    readonly property var groupItems: page.groupTogglesFor(
                                        deviceCard.toggles,
                                        modelData.keys,
                                        index === page.toggleGroups.length - 1,
                                        page.allGroupedKeys
                                    )
                                    visible: groupItems.length > 0
                                    width: parent.width
                                    spacing: 6
                                    Txt {
                                        text: groupCol.modelData.title
                                        role: "label"
                                        muted: true
                                    }
                                    Rectangle {
                                        width: parent.width
                                        height: groupInnerCol.height + 8
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant
                                        Column {
                                            id: groupInnerCol
                                            x: 14
                                            y: 4
                                            width: parent.width - 28
                                            readonly property bool splitCols: width >= 440
                                            readonly property int rowCount: splitCols
                                                ? Math.ceil(groupCol.groupItems.length / 2)
                                                : groupCol.groupItems.length

                                            Repeater {
                                                model: groupInnerCol.rowCount
                                                delegate: Column {
                                                    id: permPairWrap
                                                    required property int index
                                                    width: parent.width
                                                    readonly property int leftIdx: groupInnerCol.splitCols ? index * 2 : index
                                                    readonly property int rightIdx: groupInnerCol.splitCols ? index * 2 + 1 : -1
                                                    readonly property var leftItem: leftIdx < groupCol.groupItems.length ? groupCol.groupItems[leftIdx] : null
                                                    readonly property var rightItem: (rightIdx >= 0 && rightIdx < groupCol.groupItems.length) ? groupCol.groupItems[rightIdx] : null
                                                    readonly property bool hasRight: rightItem !== null
                                                    readonly property real colWidth: (groupInnerCol.splitCols && hasRight)
                                                        ? Math.floor((width - 33) / 2)
                                                        : width

                                                    Divider {
                                                        width: parent.width
                                                        visible: permPairWrap.index > 0
                                                    }

                                                    Item {
                                                        width: parent.width
                                                        implicitHeight: permPairWrap.hasRight
                                                            ? Math.max(leftPermRow.implicitHeight, rightPermRow.implicitHeight)
                                                            : leftPermRow.implicitHeight

                                                        ListRow {
                                                            id: leftPermRow
                                                            visible: permPairWrap.leftItem !== null
                                                            x: 0
                                                            y: Math.round((parent.height - height) / 2)
                                                            width: permPairWrap.colWidth
                                                            iconPath: permPairWrap.leftItem ? (page.toggleIcons[permPairWrap.leftItem.name] || "") : ""
                                                            title: permPairWrap.leftItem ? (page.toggleNames[permPairWrap.leftItem.name] || permPairWrap.leftItem.name) : ""
                                                            description: permPairWrap.leftItem ? (page.toggleDescriptions[permPairWrap.leftItem.name] || "") : ""
                                                            Row {
                                                                spacing: 8
                                                                Button {
                                                                    visible: permPairWrap.leftItem !== null && permPairWrap.leftItem.name === "storage" && permPairWrap.leftItem.on
                                                                    anchors.verticalCenter: parent.verticalCenter
                                                                    variant: "tonal"
                                                                    size: "sm"
                                                                    iconPath: Icons.folder
                                                                    text: qsTr("Open")
                                                                    onClicked: AppController.openPhoneStorage(deviceCard.deviceId)
                                                                }
                                                                Toggle {
                                                                    anchors.verticalCenter: parent.verticalCenter
                                                                    label: permPairWrap.leftItem ? (page.toggleNames[permPairWrap.leftItem.name] || permPairWrap.leftItem.name) : ""
                                                                    checked: permPairWrap.leftItem ? Boolean(permPairWrap.leftItem.on) : false
                                                                    onToggled: (on) => {
                                                                        if (permPairWrap.leftItem)
                                                                            AppController.setDeviceToggle(deviceCard.deviceId, permPairWrap.leftItem.name, on)
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        Rectangle {
                                                            visible: permPairWrap.hasRight
                                                            x: Math.floor(parent.width / 2)
                                                            y: 8
                                                            width: 1
                                                            height: Math.max(24, parent.height - 16)
                                                            color: Theme.outlineVariant
                                                        }

                                                        ListRow {
                                                            id: rightPermRow
                                                            visible: permPairWrap.hasRight
                                                            x: parent.width - width
                                                            y: Math.round((parent.height - height) / 2)
                                                            width: permPairWrap.colWidth
                                                            iconPath: permPairWrap.rightItem ? (page.toggleIcons[permPairWrap.rightItem.name] || "") : ""
                                                            title: permPairWrap.rightItem ? (page.toggleNames[permPairWrap.rightItem.name] || permPairWrap.rightItem.name) : ""
                                                            description: permPairWrap.rightItem ? (page.toggleDescriptions[permPairWrap.rightItem.name] || "") : ""
                                                            Row {
                                                                spacing: 8
                                                                Button {
                                                                    visible: permPairWrap.rightItem !== null && permPairWrap.rightItem.name === "storage" && permPairWrap.rightItem.on
                                                                    anchors.verticalCenter: parent.verticalCenter
                                                                    variant: "tonal"
                                                                    size: "sm"
                                                                    iconPath: Icons.folder
                                                                    text: qsTr("Open")
                                                                    onClicked: AppController.openPhoneStorage(deviceCard.deviceId)
                                                                }
                                                                Toggle {
                                                                    anchors.verticalCenter: parent.verticalCenter
                                                                    label: permPairWrap.rightItem ? (page.toggleNames[permPairWrap.rightItem.name] || permPairWrap.rightItem.name) : ""
                                                                    checked: permPairWrap.rightItem ? Boolean(permPairWrap.rightItem.on) : false
                                                                    onToggled: (on) => {
                                                                        if (permPairWrap.rightItem)
                                                                            AppController.setDeviceToggle(deviceCard.deviceId, permPairWrap.rightItem.name, on)
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 4. CAMERA & AUDIO
            // ================================================================
            Column {
                visible: page.currentSection === "camera_audio"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Row 1: Stream PC audio to phone | Pause media during phone calls
                        Item {
                            id: audioCallsPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(streamAudioRow.implicitHeight, pauseOnCallRow.implicitHeight)
                                : (streamAudioRow.implicitHeight + 1 + pauseOnCallRow.implicitHeight)

                            ListRow {
                                id: streamAudioRow
                                x: 0
                                y: audioCallsPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: audioCallsPairRow.colWidth
                                iconPath: Icons.speaker
                                title: qsTr("Stream PC audio to phone")
                                description: qsTr("Listen to PC audio through your phone.")
                                Toggle {
                                    label: qsTr("Stream PC audio to phone")
                                    checked: Preferences.streamPcAudioToPhone
                                    onToggled: (on) => Preferences.streamPcAudioToPhone = on
                                }
                            }

                            Rectangle {
                                visible: audioCallsPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !audioCallsPairRow.splitCols
                                y: streamAudioRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: pauseOnCallRow
                                x: audioCallsPairRow.splitCols ? (parent.width - width) : 0
                                y: audioCallsPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (streamAudioRow.implicitHeight + 1)
                                width: audioCallsPairRow.colWidth
                                iconPath: Icons.pause
                                title: qsTr("Pause media during calls")
                                description: qsTr("Auto-pause PC media when phone rings.")
                                Toggle {
                                    label: qsTr("Pause media during calls")
                                    checked: Preferences.pauseMediaOnCall
                                    onToggled: (on) => Preferences.pauseMediaOnCall = on
                                }
                            }
                        }

                        Divider { width: parent.width }

                        ListRow {
                            width: parent.width
                            iconPath: Icons.soundOff
                            title: qsTr("Mute this PC while a phone watches")
                            description: qsTr("With Play on phone, the phone still gets the sound.")
                            Toggle {
                                label: qsTr("Mute this PC while a phone watches")
                                checked: Preferences.muteWhileWatched
                                onToggled: (on) => Preferences.muteWhileWatched = on
                            }
                        }

                        Divider { width: parent.width }

                        // Phone as microphone: its sound plays into an output; a
                        // virtual audio cable's other end is a microphone apps can pick.
                        ListRow {
                            width: parent.width
                            iconPath: Icons.mic
                            title: qsTr("Phone as microphone")
                            description: PhoneMic.phase === "live"
                                ? qsTr("Live from %1, playing into %2.").arg(PhoneMic.deviceName).arg(PhoneMic.outputName)
                                : PhoneMic.phase === "asking"
                                    ? qsTr("Tap Start on %1.").arg(PhoneMic.deviceName)
                                    : PhoneMic.cable
                                        ? qsTr("Plays into %1, so apps can pick its other end as a microphone.").arg(PhoneMic.outputName)
                                        : qsTr("Plays into %1. To use it as a microphone in other apps, install a virtual audio cable such as VB-CABLE.").arg(PhoneMic.outputName)
                            Row {
                                spacing: 8
                                Button {
                                    variant: "outline"
                                    size: "sm"
                                    text: qsTr("Output")
                                    onClicked: {
                                        PhoneMic.refresh()
                                        micOutputSheet.open()
                                    }
                                }
                                Button {
                                    size: "sm"
                                    variant: PhoneMic.phase === "off" ? "fill" : "tonal"
                                    enabled: PhoneMic.phase !== "off" || PhoneMic.device !== ""
                                    text: PhoneMic.phase === "off" ? qsTr("Start") : qsTr("Stop")
                                    onClicked: PhoneMic.phase === "off" ? PhoneMic.start(PhoneMic.device) : PhoneMic.stop()
                                }
                            }
                        }

                        ListRow {
                            width: parent.width
                            iconPath: Icons.speaker
                            title: qsTr("Microphone volume")
                            description: qsTr("%1%").arg(PhoneMic.gain)
                            ValueSlider {
                                width: 200
                                from: PhoneMic.minGain
                                to: PhoneMic.maxGain
                                step: 10
                                value: PhoneMic.gain
                                label: qsTr("Microphone volume")
                                valueText: qsTr("%1%").arg(PhoneMic.gain)
                                onMoved: (v) => PhoneMic.setLoudness(v)
                            }
                        }

                        Divider { width: parent.width }

                        // Row 2: Virtual camera add-on
                        ListRow {
                            width: parent.width
                            iconPath: Icons.video
                            title: qsTr("Virtual camera add-on")
                            description: Webcam.addonRegistered
                                ? qsTr("Registered for Zoom, Teams & Meet.")
                                : qsTr("One-time setup (admin required) for Zoom, Teams & Meet.")
                            Row {
                                spacing: 12
                                Spinner {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: Webcam.addonBusy
                                }
                                Row {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: Webcam.addonRegistered
                                    spacing: 6
                                    StatusDot {
                                        anchors.verticalCenter: parent.verticalCenter
                                        online: true
                                    }
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        role: "bodySmall"
                                        muted: true
                                        text: qsTr("Ready")
                                    }
                                }
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    variant: Webcam.addonRegistered ? "outline" : "tonal"
                                    size: "sm"
                                    enabled: !Webcam.addonBusy
                                    text: Webcam.addonRegistered ? qsTr("Remove") : qsTr("Set up")
                                    onClicked: Webcam.addonRegistered ? Webcam.removeAddon() : Webcam.setupAddon()
                                }
                            }
                        }

                        Divider { width: parent.width; visible: DeviceList.count > 1 }
                        ListRow {
                            width: parent.width
                            visible: DeviceList.count > 1
                            iconPath: Icons.phone
                            title: qsTr("Phone")
                            Row {
                                spacing: 8
                                Repeater {
                                    model: DeviceList
                                    delegate: Chip {
                                        required property string deviceId
                                        required property string name
                                        interactive: true
                                        text: name
                                        selected: Webcam.selectedPhone === deviceId
                                        onClicked: Webcam.selectPhone(deviceId)
                                    }
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 3: Camera (Back | Front) | Mirror image
                        Item {
                            id: camMirrorPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(cameraSelectRow.implicitHeight, mirrorCamRow.implicitHeight)
                                : (cameraSelectRow.implicitHeight + 1 + mirrorCamRow.implicitHeight)

                            ListRow {
                                id: cameraSelectRow
                                x: 0
                                y: camMirrorPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: camMirrorPairRow.colWidth
                                iconPath: Icons.camera
                                title: qsTr("Camera")
                                Segmented {
                                    options: [
                                        { value: "back", label: qsTr("Back") },
                                        { value: "front", label: qsTr("Front") }
                                    ]
                                    value: Webcam.camera
                                    onPicked: (value) => Webcam.setCamera(value)
                                }
                            }

                            Rectangle {
                                visible: camMirrorPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !camMirrorPairRow.splitCols
                                y: cameraSelectRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: mirrorCamRow
                                x: camMirrorPairRow.splitCols ? (parent.width - width) : 0
                                y: camMirrorPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (cameraSelectRow.implicitHeight + 1)
                                width: camMirrorPairRow.colWidth
                                iconPath: Icons.mirror
                                title: qsTr("Mirror image")
                                Toggle {
                                    label: qsTr("Mirror image")
                                    checked: Webcam.mirror
                                    onToggled: (on) => Webcam.setMirrorImage(on)
                                }
                            }
                        }

                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.camera
                            title: qsTr("Resolution")
                            Segmented {
                                // 4K only when the phone's camera in use can stream it.
                                options: [
                                    { value: "720", label: "720p" },
                                    { value: "1080", label: "1080p" }
                                ].concat(Webcam.uhd ? [{ value: "2160", label: "4K" }] : [])
                                value: String(Webcam.height)
                                onPicked: (value) => Webcam.setResolution(Number(value))
                            }
                        }

                        Divider { width: parent.width }
                        Column {
                            width: parent.width
                            spacing: 10
                            topPadding: 8
                            Item {
                                width: parent.width
                                height: Math.max(webcamStatusLabel.height, webcamStartBtn.height)
                                Txt {
                                    id: webcamStatusLabel
                                    anchors.left: parent.left
                                    anchors.right: webcamStartBtn.left
                                    anchors.rightMargin: 12
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "bodySmall"
                                    muted: Webcam.phase !== "streaming"
                                    elide: Text.ElideRight
                                    text: Webcam.statusText.length > 0
                                        ? Webcam.statusText
                                        : (Webcam.selectedPhoneName.length > 0
                                           ? qsTr("Preview · %1").arg(Webcam.selectedPhoneName)
                                           : qsTr("Pair a phone to use its camera as a webcam"))
                                }
                                Button {
                                    id: webcamStartBtn
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    enabled: DeviceList.count > 0
                                    variant: Webcam.phase === "streaming" || Webcam.phase === "asking" ? "outline" : "tonal"
                                    size: "sm"
                                    iconPath: Webcam.phase === "streaming" || Webcam.phase === "asking" ? Icons.close : Icons.video
                                    text: Webcam.phase === "streaming" || Webcam.phase === "asking"
                                        ? qsTr("Stop webcam")
                                        : qsTr("Start webcam")
                                    onClicked: {
                                        if (Webcam.phase === "streaming" || Webcam.phase === "asking")
                                            Webcam.stop()
                                        else
                                            Webcam.start("")
                                    }
                                }
                            }
                            Rectangle {
                                width: parent.width
                                height: Math.round(width * 9 / 16)
                                radius: Theme.radiusMd
                                color: "#121318"
                                border.width: 1
                                border.color: Theme.outlineVariant
                                clip: true
                                VideoView {
                                    anchors.fill: parent
                                    stream: "webcam"
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 5. NOTIFICATIONS
            // ================================================================
            Column {
                visible: page.currentSection === "notifications"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Row 1: Follow phone Do not disturb | Forward PC notifications
                        Item {
                            id: notifPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(syncDndRow.implicitHeight, forwardPcNotifRow.implicitHeight)
                                : (syncDndRow.implicitHeight + 1 + forwardPcNotifRow.implicitHeight)

                            ListRow {
                                id: syncDndRow
                                x: 0
                                y: notifPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: notifPairRow.colWidth
                                iconPath: Icons.moon
                                title: qsTr("Follow phone Do not disturb")
                                description: qsTr("Mute pop-ups while phone is on DND.")
                                Toggle {
                                    label: qsTr("Follow phone Do not disturb")
                                    checked: Preferences.syncDnd
                                    onToggled: (on) => Preferences.syncDnd = on
                                }
                            }

                            Rectangle {
                                visible: notifPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !notifPairRow.splitCols
                                y: syncDndRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: forwardPcNotifRow
                                x: notifPairRow.splitCols ? (parent.width - width) : 0
                                y: notifPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (syncDndRow.implicitHeight + 1)
                                width: notifPairRow.colWidth
                                iconPath: Icons.bell
                                title: qsTr("Forward PC notifications")
                                description: qsTr("Send PC alerts to phone while away.")
                                Toggle {
                                    label: qsTr("Forward PC notifications")
                                    checked: Preferences.forwardPcNotificationsToPhone
                                    onToggled: (on) => Preferences.forwardPcNotificationsToPhone = on
                                }
                            }
                        }

                        Divider { width: parent.width; visible: Qt.platform.os === "windows" }
                        ListRow {
                            width: parent.width
                            visible: Qt.platform.os === "windows"
                            iconPath: Icons.moon
                            title: qsTr("Quiet hours")
                            description: qsTr("Configure automatic DND schedule in Windows.")
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Set a schedule")
                                onClicked: Qt.openUrlExternally("ms-settings:notifications")
                            }
                        }
                        Repeater {
                            model: { try { return JSON.parse(NotificationList.apps) } catch (e) { return [] } }
                            delegate: Column {
                                id: appRow
                                required property var modelData
                                width: parent.width
                                Divider { width: parent.width }
                                Item {
                                    width: parent.width
                                    height: 54
                                    Item {
                                        id: appIcon
                                        x: 4
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 28; height: 28
                                        Image {
                                            id: appImage
                                            anchors.fill: parent
                                            source: appRow.modelData.icon
                                            sourceSize: Qt.size(56, 56)
                                            mipmap: true
                                            visible: status === Image.Ready
                                        }
                                        Avatar { anchors.fill: parent; visible: appImage.status !== Image.Ready; name: appRow.modelData.name }
                                    }
                                    Txt {
                                        anchors.left: appIcon.right
                                        anchors.leftMargin: 14
                                        anchors.right: appChoice.left
                                        anchors.rightMargin: 12
                                        anchors.verticalCenter: parent.verticalCenter
                                        role: "body"
                                        text: appRow.modelData.name
                                        elide: Text.ElideRight
                                    }
                                    Segmented {
                                        id: appChoice
                                        anchors.right: parent.right
                                        anchors.verticalCenter: parent.verticalCenter
                                        options: [
                                            { value: "show", label: qsTr("Show") },
                                            { value: "quiet", label: qsTr("No pop-ups") },
                                            { value: "hidden", label: qsTr("Hide") }
                                        ]
                                        value: appRow.modelData.rule
                                        onPicked: (value) => NotificationList.setAppRule(appRow.modelData.app, value)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 6. FILES & SYNC
            // ================================================================
            Column {
                visible: page.currentSection === "files_sync"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Row 1: Show phones in File Explorer | Your PC in your pocket
                        Item {
                            id: filesSharePairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(sendToMenuRow.implicitHeight, pcInPocketRow.implicitHeight)
                                : (sendToMenuRow.implicitHeight + 1 + pcInPocketRow.implicitHeight)

                            ListRow {
                                id: sendToMenuRow
                                x: 0
                                y: filesSharePairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: filesSharePairRow.colWidth
                                iconPath: Icons.send
                                title: Qt.platform.os === "linux" ? qsTr("File manager Send to") : qsTr("File Explorer Send to")
                                description: qsTr("Send files from right-click menu.")
                                Toggle {
                                    label: Qt.platform.os === "linux" ? qsTr("File manager Send to") : qsTr("File Explorer Send to")
                                    checked: Preferences.sendToMenu
                                    onToggled: (on) => Preferences.sendToMenu = on
                                }
                            }

                            Rectangle {
                                visible: filesSharePairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !filesSharePairRow.splitCols
                                y: sendToMenuRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: pcInPocketRow
                                x: filesSharePairRow.splitCols ? (parent.width - width) : 0
                                y: filesSharePairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (sendToMenuRow.implicitHeight + 1)
                                width: filesSharePairRow.colWidth
                                iconPath: Icons.folder
                                title: qsTr("Your PC in your pocket")
                                description: qsTr("Share Downloads, Documents, Desktop & Pictures.")
                                Toggle {
                                    label: qsTr("Your PC in your pocket")
                                    checked: Preferences.sharePcStorage
                                    onToggled: (on) => Preferences.sharePcStorage = on
                                }
                            }
                        }

                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.globe
                            title: qsTr("LocalSend on local network")
                            description: qsTr("Share with nearby LocalSend devices without pairing.")
                            Toggle {
                                label: qsTr("LocalSend on local network")
                                checked: AppController.localsendEnabled
                                onToggled: (on) => AppController.toggleLocalsend(on)
                            }
                        }

                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.openExternal
                            title: qsTr("Open files shared from the phone")
                            description: qsTr("Opens a shared photo or document as soon as it arrives.")
                            Toggle {
                                label: qsTr("Open files shared from the phone")
                                checked: Preferences.openSharedFiles
                                onToggled: (on) => Preferences.openSharedFiles = on
                            }
                        }

                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            stackBelow: true
                            iconPath: Icons.mic
                            title: qsTr("Voice recordings")
                            description: Preferences.recordingsFolder
                            Row {
                                spacing: 8
                                Segmented {
                                    anchors.verticalCenter: parent.verticalCenter
                                    options: [
                                        { value: "m4a", label: "M4A" },
                                        { value: "mp3", label: "MP3" },
                                        { value: "wav", label: "WAV" },
                                        { value: "flac", label: "FLAC" }
                                    ]
                                    value: Preferences.recordingsFormat
                                    onPicked: (value) => Preferences.recordingsFormat = value
                                }
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: !Preferences.recordingsFolderIsDefault
                                    variant: "text"
                                    size: "sm"
                                    text: qsTr("Reset")
                                    onClicked: Preferences.resetRecordingsFolder()
                                }
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    variant: "tonal"
                                    size: "sm"
                                    text: qsTr("Choose folder")
                                    onClicked: recordingsFolderPicker.open()
                                }
                            }
                        }
                    }
                }

                // Photo backup: every album, by year and month, never deleting.
                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.photo
                            title: qsTr("Photo backup")
                            description: qsTr("Copies new photos from every album into a folder on this PC, by year and month. Nothing is deleted, here or on the phone. Needs only the phone's photos permission.")
                        }
                        Repeater {
                            model: { try { return JSON.parse(PhotoBackup.phones || "[]") } catch (e) { return [] } }
                            delegate: Column {
                                id: backupDelegate
                                required property var modelData
                                width: parent.width
                                Divider { width: parent.width }
                                ListRow {
                                    width: parent.width
                                    iconPath: Icons.phone
                                    title: backupDelegate.modelData.name
                                    description: backupDelegate.modelData.status
                                    Toggle {
                                        label: qsTr("Back up photos from %1").arg(backupDelegate.modelData.name)
                                        checked: Boolean(backupDelegate.modelData.enabled)
                                        onToggled: (on) => PhotoBackup.setEnabled(backupDelegate.modelData.id, on)
                                    }
                                }
                                ListRow {
                                    width: parent.width
                                    visible: Boolean(backupDelegate.modelData.enabled)
                                    iconPath: Icons.folder
                                    title: backupDelegate.modelData.folder
                                    description: qsTr("Backup folder")
                                    Row {
                                        spacing: 8
                                        Button {
                                            anchors.verticalCenter: parent.verticalCenter
                                            variant: "text"
                                            size: "sm"
                                            text: qsTr("Open")
                                            onClicked: PhotoBackup.openFolder(backupDelegate.modelData.id)
                                        }
                                        Button {
                                            anchors.verticalCenter: parent.verticalCenter
                                            variant: "tonal"
                                            size: "sm"
                                            text: qsTr("Choose folder")
                                            onClicked: {
                                                photoBackupFolderPicker.deviceId = backupDelegate.modelData.id
                                                photoBackupFolderPicker.open()
                                            }
                                        }
                                    }
                                }
                                ListRow {
                                    width: parent.width
                                    visible: Boolean(backupDelegate.modelData.enabled)
                                    iconPath: Icons.video
                                    title: qsTr("Include videos")
                                    description: qsTr("Videos can be large. Copied one at a time, so a lost connection costs at most one file.")
                                    Row {
                                        spacing: 8
                                        Button {
                                            anchors.verticalCenter: parent.verticalCenter
                                            variant: "text"
                                            size: "sm"
                                            iconPath: Icons.refresh
                                            text: qsTr("Back up now")
                                            enabled: Boolean(backupDelegate.modelData.online)
                                            onClicked: PhotoBackup.backUpNow(backupDelegate.modelData.id)
                                        }
                                        Toggle {
                                            anchors.verticalCenter: parent.verticalCenter
                                            label: qsTr("Include videos")
                                            checked: Boolean(backupDelegate.modelData.videos)
                                            onToggled: (on) => PhotoBackup.setVideos(backupDelegate.modelData.id, on)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.folder
                            title: qsTr("Synced folders")
                            description: qsTr("One-way or two-way folder sync between phone and PC.")
                            Row {
                                spacing: 8
                                Button {
                                    variant: "text"
                                    size: "sm"
                                    iconPath: Icons.refresh
                                    text: qsTr("Sync now")
                                    onClicked: Preferences.syncFoldersNow()
                                }
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    iconPath: Icons.plus
                                    text: qsTr("Add DCIM → Pictures")
                                    onClicked: Preferences.addFolderSyncRule("", "DCIM/Camera", Preferences.recordingsFolder.replace(/[\\\/][^\\\/]+[\\\/]?$/, "") + "/Pictures/Phone Camera", false)
                                }
                            }
                        }
                        Repeater {
                            model: { try { return JSON.parse(Preferences.folderSyncRulesJson || "[]") } catch (e) { return [] } }
                            delegate: Column {
                                id: syncRuleDelegate
                                required property var modelData
                                width: parent.width
                                Divider { width: parent.width }
                                ListRow {
                                    width: parent.width
                                    iconPath: Icons.folder
                                    title: syncRuleDelegate.modelData.phoneFolder + (syncRuleDelegate.modelData.mode === "two_way" ? " ⇄ " : " → ") + syncRuleDelegate.modelData.pcFolder
                                    description: syncRuleDelegate.modelData.mode === "two_way" ? qsTr("Two-way sync · conflict copies kept") : qsTr("One-way · phone to PC")
                                    Row {
                                        spacing: 8
                                        Button {
                                            anchors.verticalCenter: parent.verticalCenter
                                            variant: "text"
                                            size: "sm"
                                            iconPath: Icons.trash
                                            text: qsTr("Remove")
                                            onClicked: Preferences.removeFolderSyncRule(syncRuleDelegate.modelData.id)
                                        }
                                        Toggle {
                                            anchors.verticalCenter: parent.verticalCenter
                                            label: syncRuleDelegate.modelData.phoneFolder
                                            checked: Boolean(syncRuleDelegate.modelData.enabled)
                                            onToggled: (on) => Preferences.setFolderSyncRuleEnabled(syncRuleDelegate.modelData.id, on)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 7. SHORTCUTS & RULES
            // ================================================================
            Column {
                visible: page.currentSection === "shortcuts"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Row 1: Command palette | File shelf
                        Item {
                            id: shortcutsPairRow1
                            width: parent.width
                            readonly property bool splitCols: width >= 540
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(paletteShortcutRow.implicitHeight, shelfShortcutRow.implicitHeight)
                                : (paletteShortcutRow.implicitHeight + 1 + shelfShortcutRow.implicitHeight)

                            ListRow {
                                id: paletteShortcutRow
                                x: 0
                                y: shortcutsPairRow1.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: shortcutsPairRow1.colWidth
                                iconPath: Icons.search
                                title: qsTr("Command palette")
                                description: qsTr("Open from any app.")
                                Row {
                                    spacing: 6
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Preferences.commandPaletteHotkey !== "Ctrl+Alt+Space"
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Reset")
                                        onClicked: {
                                            paletteHotkeyInput.text = "Ctrl+Alt+Space"
                                            Preferences.commandPaletteHotkey = "Ctrl+Alt+Space"
                                        }
                                    }
                                    Rectangle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 124
                                        height: 32
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHighest
                                        border.width: paletteHotkeyInput.activeFocus ? 2 : 1
                                        border.color: paletteHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                        TextInput {
                                            id: paletteHotkeyInput
                                            anchors.fill: parent
                                            anchors.leftMargin: 8
                                            anchors.rightMargin: 8
                                            verticalAlignment: TextInput.AlignVCenter
                                            horizontalAlignment: TextInput.AlignHCenter
                                            font.family: Theme.fontUi
                                            font.pixelSize: 12
                                            color: Theme.surfaceContent
                                            selectionColor: Theme.primaryContainer
                                            selectedTextColor: Theme.primaryContainerContent
                                            clip: true
                                            text: Preferences.commandPaletteHotkey
                                            Keys.onPressed: (event) => {
                                                const combo = page.formatKeyCombo(event)
                                                if (combo.length > 0) {
                                                    text = combo
                                                    Preferences.commandPaletteHotkey = combo
                                                    event.accepted = true
                                                }
                                            }
                                            onEditingFinished: Preferences.commandPaletteHotkey = text.trim()
                                            Accessible.name: qsTr("Command palette shortcut")
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                visible: shortcutsPairRow1.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !shortcutsPairRow1.splitCols
                                y: paletteShortcutRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: shelfShortcutRow
                                x: shortcutsPairRow1.splitCols ? (parent.width - width) : 0
                                y: shortcutsPairRow1.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (paletteShortcutRow.implicitHeight + 1)
                                width: shortcutsPairRow1.colWidth
                                iconPath: Icons.folder
                                title: qsTr("File shelf")
                                description: qsTr("Toggle screen-edge shelf.")
                                Row {
                                    spacing: 6
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Preferences.shelfHotkey !== "Ctrl+Alt+S"
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Reset")
                                        onClicked: {
                                            shelfHotkeyInput.text = "Ctrl+Alt+S"
                                            Preferences.shelfHotkey = "Ctrl+Alt+S"
                                        }
                                    }
                                    Rectangle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 124
                                        height: 32
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHighest
                                        border.width: shelfHotkeyInput.activeFocus ? 2 : 1
                                        border.color: shelfHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                        TextInput {
                                            id: shelfHotkeyInput
                                            anchors.fill: parent
                                            anchors.leftMargin: 8
                                            anchors.rightMargin: 8
                                            verticalAlignment: TextInput.AlignVCenter
                                            horizontalAlignment: TextInput.AlignHCenter
                                            font.family: Theme.fontUi
                                            font.pixelSize: 12
                                            color: Theme.surfaceContent
                                            selectionColor: Theme.primaryContainer
                                            selectedTextColor: Theme.primaryContainerContent
                                            clip: true
                                            text: Preferences.shelfHotkey
                                            Keys.onPressed: (event) => {
                                                const combo = page.formatKeyCombo(event)
                                                if (combo.length > 0) {
                                                    text = combo
                                                    Preferences.shelfHotkey = combo
                                                    event.accepted = true
                                                }
                                            }
                                            onEditingFinished: Preferences.shelfHotkey = text.trim()
                                            Accessible.name: qsTr("Shelf shortcut")
                                        }
                                    }
                                }
                            }
                        }

                        Divider { width: parent.width }

                        // Row 2: Take photo | Scan document
                        Item {
                            id: shortcutsPairRow2
                            width: parent.width
                            readonly property bool splitCols: width >= 540
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(photoShortcutRow.implicitHeight, scanShortcutRow.implicitHeight)
                                : (photoShortcutRow.implicitHeight + 1 + scanShortcutRow.implicitHeight)

                            ListRow {
                                id: photoShortcutRow
                                x: 0
                                y: shortcutsPairRow2.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: shortcutsPairRow2.colWidth
                                iconPath: Icons.camera
                                title: qsTr("Take photo")
                                description: qsTr("Capture & paste photo.")
                                Row {
                                    spacing: 6
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Preferences.continuityPhotoHotkey !== "Ctrl+Alt+C"
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Reset")
                                        onClicked: {
                                            photoHotkeyInput.text = "Ctrl+Alt+C"
                                            Preferences.continuityPhotoHotkey = "Ctrl+Alt+C"
                                        }
                                    }
                                    Rectangle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 124
                                        height: 32
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHighest
                                        border.width: photoHotkeyInput.activeFocus ? 2 : 1
                                        border.color: photoHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                        TextInput {
                                            id: photoHotkeyInput
                                            anchors.fill: parent
                                            anchors.leftMargin: 8
                                            anchors.rightMargin: 8
                                            verticalAlignment: TextInput.AlignVCenter
                                            horizontalAlignment: TextInput.AlignHCenter
                                            font.family: Theme.fontUi
                                            font.pixelSize: 12
                                            color: Theme.surfaceContent
                                            selectionColor: Theme.primaryContainer
                                            selectedTextColor: Theme.primaryContainerContent
                                            clip: true
                                            text: Preferences.continuityPhotoHotkey
                                            Keys.onPressed: (event) => {
                                                const combo = page.formatKeyCombo(event)
                                                if (combo.length > 0) {
                                                    text = combo
                                                    Preferences.continuityPhotoHotkey = combo
                                                    event.accepted = true
                                                }
                                            }
                                            onEditingFinished: Preferences.continuityPhotoHotkey = text.trim()
                                            Accessible.name: qsTr("Take photo shortcut")
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                visible: shortcutsPairRow2.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !shortcutsPairRow2.splitCols
                                y: photoShortcutRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: scanShortcutRow
                                x: shortcutsPairRow2.splitCols ? (parent.width - width) : 0
                                y: shortcutsPairRow2.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (photoShortcutRow.implicitHeight + 1)
                                width: shortcutsPairRow2.colWidth
                                iconPath: Icons.clipboardList
                                title: qsTr("Scan document")
                                description: qsTr("Scan & paste document.")
                                Row {
                                    spacing: 6
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Preferences.continuityScanHotkey !== "Ctrl+Alt+D"
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Reset")
                                        onClicked: {
                                            scanHotkeyInput.text = "Ctrl+Alt+D"
                                            Preferences.continuityScanHotkey = "Ctrl+Alt+D"
                                        }
                                    }
                                    Rectangle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 124
                                        height: 32
                                        radius: Theme.radiusSm
                                        color: Theme.surfaceContainerHighest
                                        border.width: scanHotkeyInput.activeFocus ? 2 : 1
                                        border.color: scanHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                        TextInput {
                                            id: scanHotkeyInput
                                            anchors.fill: parent
                                            anchors.leftMargin: 8
                                            anchors.rightMargin: 8
                                            verticalAlignment: TextInput.AlignVCenter
                                            horizontalAlignment: TextInput.AlignHCenter
                                            font.family: Theme.fontUi
                                            font.pixelSize: 12
                                            color: Theme.surfaceContent
                                            selectionColor: Theme.primaryContainer
                                            selectedTextColor: Theme.primaryContainerContent
                                            clip: true
                                            text: Preferences.continuityScanHotkey
                                            Keys.onPressed: (event) => {
                                                const combo = page.formatKeyCombo(event)
                                                if (combo.length > 0) {
                                                    text = combo
                                                    Preferences.continuityScanHotkey = combo
                                                    event.accepted = true
                                                }
                                            }
                                            onEditingFinished: Preferences.continuityScanHotkey = text.trim()
                                            Accessible.name: qsTr("Scan document shortcut")
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.bolt
                            title: qsTr("Trigger-action rules")
                            description: qsTr("Run actions when your phone connects, disconnects, or rings.")
                            Row {
                                spacing: 8
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    iconPath: Icons.plus
                                    text: qsTr("Lock PC on disconnect")
                                    onClicked: Preferences.addAutomationRule("phone_disconnected", "lock_pc", "")
                                }
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    iconPath: Icons.plus
                                    text: qsTr("Pause media on ring")
                                    onClicked: Preferences.addAutomationRule("call_ringing", "pause_pc_media", "")
                                }
                            }
                        }
                        Repeater {
                            model: { try { return JSON.parse(Preferences.automationRulesJson || "[]") } catch (e) { return [] } }
                            delegate: Column {
                                id: autoRuleDelegate
                                required property var modelData
                                width: parent.width
                                Divider { width: parent.width }
                                ListRow {
                                    width: parent.width
                                    iconPath: Icons.bolt
                                    title: page.formatTrigger(autoRuleDelegate.modelData.trigger) + " → " + page.formatAction(autoRuleDelegate.modelData.actionKind)
                                    description: autoRuleDelegate.modelData.actionArg && autoRuleDelegate.modelData.actionArg.length > 0
                                        ? autoRuleDelegate.modelData.actionArg
                                        : ""
                                    Row {
                                        spacing: 8
                                        Button {
                                            anchors.verticalCenter: parent.verticalCenter
                                            variant: "text"
                                            size: "sm"
                                            iconPath: Icons.trash
                                            text: qsTr("Remove")
                                            onClicked: Preferences.removeAutomationRule(autoRuleDelegate.modelData.id)
                                        }
                                        Toggle {
                                            anchors.verticalCenter: parent.verticalCenter
                                            label: autoRuleDelegate.modelData.trigger
                                            checked: Boolean(autoRuleDelegate.modelData.enabled)
                                            onToggled: (on) => Preferences.setAutomationRuleEnabled(autoRuleDelegate.modelData.id, on)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // OBS Studio, for Deck tiles that run "obs:" commands.
                Card {
                    id: obsCard
                    width: parent.width
                    property bool hasPassword: Preferences.obsHasPassword()
                    Column {
                        width: parent.width
                        spacing: 12
                        ListRow {
                            width: parent.width
                            iconPath: Icons.terminal
                            title: qsTr("OBS Studio")
                            description: qsTr("For Deck tiles that run obs: commands. Turn on OBS's WebSocket server in Tools › WebSocket Server Settings.")
                        }
                        Row {
                            spacing: 10
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                muted: true
                                text: qsTr("Port")
                            }
                            Rectangle {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 76
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: obsPortEdit.activeFocus ? 2 : 1
                                border.color: obsPortEdit.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: obsPortEdit
                                    anchors.fill: parent
                                    anchors.leftMargin: 8
                                    anchors.rightMargin: 8
                                    verticalAlignment: TextInput.AlignVCenter
                                    horizontalAlignment: TextInput.AlignHCenter
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    font.family: Theme.fontMono
                                    font.pixelSize: 12
                                    text: String(Preferences.obsPort())
                                    validator: IntValidator { bottom: 1024; top: 65535 }
                                    Accessible.name: qsTr("OBS WebSocket port")
                                }
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                muted: true
                                text: qsTr("Password")
                            }
                            Rectangle {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 180
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: obsPasswordEdit.activeFocus ? 2 : 1
                                border.color: obsPasswordEdit.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: obsPasswordEdit
                                    anchors.fill: parent
                                    anchors.leftMargin: 8
                                    anchors.rightMargin: 8
                                    verticalAlignment: TextInput.AlignVCenter
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    font.pixelSize: 12
                                    echoMode: TextInput.Password
                                    passwordCharacter: "\u2022"
                                    Accessible.name: qsTr("OBS WebSocket password")
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: obsPasswordEdit.text.length === 0 && !obsPasswordEdit.activeFocus
                                        role: "bodySmall"
                                        muted: true
                                        text: obsCard.hasPassword ? qsTr("Saved") : qsTr("None")
                                    }
                                }
                            }
                        }
                        Row {
                            spacing: 8
                            Button {
                                size: "sm"
                                text: qsTr("Save")
                                onClicked: {
                                    Preferences.saveObs(parseInt(obsPortEdit.text) || Preferences.obsPort(),
                                                        obsPasswordEdit.text, obsPasswordEdit.text.length > 0)
                                    obsPasswordEdit.text = ""
                                    obsCard.hasPassword = Preferences.obsHasPassword()
                                }
                            }
                            Button {
                                size: "sm"
                                variant: "tonal"
                                text: qsTr("Test connection")
                                onClicked: Preferences.testObs()
                            }
                            Button {
                                size: "sm"
                                variant: "tonal"
                                visible: obsCard.hasPassword
                                text: qsTr("Remove password")
                                onClicked: {
                                    Preferences.saveObs(Preferences.obsPort(), "", true)
                                    obsCard.hasPassword = Preferences.obsHasPassword()
                                }
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 8. STORAGE & PRIVACY
            // ================================================================
            Column {
                visible: page.currentSection === "storage"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width

                        // Row 1: Clipboard history | Notification history
                        Item {
                            id: historyPairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(clipHistRow.implicitHeight, notifHistRow.implicitHeight)
                                : (clipHistRow.implicitHeight + 1 + notifHistRow.implicitHeight)

                            ListRow {
                                id: clipHistRow
                                x: 0
                                y: historyPairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: historyPairRow.colWidth
                                iconPath: Icons.clipboard
                                title: qsTr("Clipboard history")
                                description: {
                                    const count = Number(page.retentionSummary.clipboardItems || 0)
                                    const size = page.formatRetentionBytes(page.retentionSummary.clipboardBytes)
                                    return qsTr("%1 clips · %2 · kept 7 days").arg(count).arg(size)
                                }
                                Row {
                                    spacing: 8
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Number(page.retentionSummary.clipboardItems || 0) > 0
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Clear")
                                        onClicked: {
                                            AppController.clearClipboardHistory()
                                            AppController.refreshDataRetention()
                                        }
                                    }
                                    Toggle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        label: qsTr("Clipboard history")
                                        checked: Preferences.clipboardHistory
                                        onToggled: (on) => {
                                            Preferences.clipboardHistory = on
                                            AppController.refreshDataRetention()
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                visible: historyPairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !historyPairRow.splitCols
                                y: clipHistRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: notifHistRow
                                x: historyPairRow.splitCols ? (parent.width - width) : 0
                                y: historyPairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (clipHistRow.implicitHeight + 1)
                                width: historyPairRow.colWidth
                                iconPath: Icons.bell
                                title: qsTr("Notification history")
                                description: {
                                    const count = Number(page.retentionSummary.notificationHistory || 0)
                                    const size = page.formatRetentionBytes(page.retentionSummary.notificationImageBytes)
                                    return qsTr("%1 alerts · %2 · kept 24h").arg(count).arg(size)
                                }
                                Row {
                                    spacing: 8
                                    Button {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: Number(page.retentionSummary.notificationHistory || 0) > 0
                                                 || Number(page.retentionSummary.notificationImageBytes || 0) > 0
                                        variant: "text"
                                        size: "sm"
                                        text: qsTr("Clear")
                                        onClicked: AppController.clearNotificationHistory()
                                    }
                                    Toggle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        label: qsTr("Notification history")
                                        checked: NotificationList.historyEnabled
                                        onToggled: (on) => {
                                            NotificationList.setHistoryEnabled(on)
                                            AppController.refreshDataRetention()
                                        }
                                    }
                                }
                            }
                        }

                        Divider { width: parent.width }
                        // 2. Cross-device timeline
                        ListRow {
                            width: parent.width
                            stackBelow: true
                            iconPath: Icons.history
                            title: qsTr("Cross-device timeline")
                            description: {
                                const count = Number(page.retentionSummary.timelineItems || 0)
                                return count === 1
                                    ? qsTr("1 item stored (max 5,000).")
                                    : qsTr("%1 items stored (max 5,000).").arg(count)
                            }
                            Row {
                                spacing: 8
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: Number(page.retentionSummary.timelineItems || 0) > 0
                                    variant: "text"
                                    size: "sm"
                                    text: qsTr("Clear")
                                    onClicked: {
                                        TimelineModel.clearAll()
                                        AppController.refreshDataRetention()
                                    }
                                }
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    variant: "text"
                                    size: "sm"
                                    text: qsTr("Open")
                                    onClicked: AppController.currentPage = "timeline"
                                }
                                Segmented {
                                    anchors.verticalCenter: parent.verticalCenter
                                    options: [
                                        { value: "30", label: qsTr("30d") },
                                        { value: "90", label: qsTr("90d") },
                                        { value: "365", label: qsTr("1y") },
                                        { value: "0", label: qsTr("All") }
                                    ]
                                    value: String(Preferences.timelineRetentionDays)
                                    onPicked: (value) => {
                                        Preferences.timelineRetentionDays = Number(value)
                                        AppController.refreshDataRetention()
                                    }
                                }
                            }
                        }
                        Divider { width: parent.width }
                        // 3. Cached messages & conversations
                        ListRow {
                            width: parent.width
                            iconPath: Icons.messages
                            title: qsTr("Cached messages")
                            description: {
                                const chatThreads = Number(page.retentionSummary.chatThreads || 0)
                                const smsThreads = Number(page.retentionSummary.cachedSmsThreads || 0)
                                const convs = chatThreads + smsThreads
                                const totalMsgs = Number(page.retentionSummary.chatMessages || 0)
                                                  + Number(page.retentionSummary.cachedSmsMessages || 0)
                                const size = page.formatRetentionBytes(page.retentionSummary.messageAttachmentBytes)
                                const convPart = convs === 1 ? qsTr("1 chat") : qsTr("%1 chats").arg(convs)
                                const msgPart = totalMsgs === 1 ? qsTr("1 message") : qsTr("%1 messages").arg(totalMsgs)
                                return qsTr("%1 · %2 · %3 · kept 90 days")
                                    .arg(convPart)
                                    .arg(msgPart)
                                    .arg(size)
                            }
                            Button {
                                variant: "text"
                                size: "sm"
                                visible: Number(page.retentionSummary.chatThreads || 0) > 0
                                         || Number(page.retentionSummary.cachedSmsThreads || 0) > 0
                                         || Number(page.retentionSummary.chatMessages || 0) > 0
                                         || Number(page.retentionSummary.cachedSmsMessages || 0) > 0
                                         || Number(page.retentionSummary.messageAttachmentBytes || 0) > 0
                                text: qsTr("Clear")
                                onClicked: AppController.clearMessageCache()
                            }
                        }
                        Divider { width: parent.width }
                        // 4. Photo thumbnails & Transfer history side-by-side
                        Item {
                            id: cachePairRow
                            width: parent.width
                            readonly property bool splitCols: width >= 460
                            readonly property real colWidth: splitCols ? Math.floor((width - 37) / 2) : width
                            implicitHeight: splitCols
                                ? Math.max(photoCacheRow.implicitHeight, transferHistRow.implicitHeight)
                                : (photoCacheRow.implicitHeight + 1 + transferHistRow.implicitHeight)

                            ListRow {
                                id: photoCacheRow
                                x: 0
                                y: cachePairRow.splitCols ? Math.round((parent.height - height) / 2) : 0
                                width: cachePairRow.colWidth
                                iconPath: Icons.photo
                                title: qsTr("Photo thumbnails")
                                description: {
                                    const files = Number(page.retentionSummary.photoThumbFiles || 0)
                                    const size = page.formatRetentionBytes(page.retentionSummary.photoThumbBytes)
                                    return qsTr("%1 cached · %2 · kept 30 days").arg(files).arg(size)
                                }
                                Button {
                                    variant: "text"
                                    size: "sm"
                                    visible: Number(page.retentionSummary.photoThumbFiles || 0) > 0
                                    text: qsTr("Clear")
                                    onClicked: AppController.clearPhotoThumbnails()
                                }
                            }

                            Rectangle {
                                visible: cachePairRow.splitCols
                                x: Math.floor(parent.width / 2)
                                y: 8
                                width: 1
                                height: Math.max(24, parent.height - 16)
                                color: Theme.outlineVariant
                            }

                            Divider {
                                visible: !cachePairRow.splitCols
                                y: photoCacheRow.implicitHeight
                                width: parent.width
                            }

                            ListRow {
                                id: transferHistRow
                                x: cachePairRow.splitCols ? (parent.width - width) : 0
                                y: cachePairRow.splitCols
                                    ? Math.round((parent.height - height) / 2)
                                    : (photoCacheRow.implicitHeight + 1)
                                width: cachePairRow.colWidth
                                iconPath: Icons.send
                                title: qsTr("Transfer history")
                                description: {
                                    const records = Number(page.retentionSummary.receivedFileRecords || 0)
                                    return qsTr("%1 records · kept with timeline").arg(records)
                                }
                                Button {
                                    variant: "text"
                                    size: "sm"
                                    visible: Number(page.retentionSummary.receivedFileRecords || 0) > 0
                                    text: qsTr("Clear")
                                    onClicked: AppController.clearReceivedFileHistory()
                                }
                            }
                        }

                        Divider { width: parent.width }
                        // 5. Clear everything
                        ListRow {
                            width: parent.width
                            iconPath: Icons.trash
                            title: qsTr("Clear all local history & caches")
                            description: qsTr("Wipes local history and caches; keeps paired devices and settings.")
                            Button {
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.trash
                                text: qsTr("Clear everything…")
                                onClicked: clearEverythingSheet.open()
                            }
                        }
                    }
                }
            }

            // ================================================================
            // 9. CONNECTION & ABOUT
            // ================================================================
            Column {
                visible: page.currentSection === "connection"
                width: visible ? parent.width : 0
                spacing: Theme.gutter

                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.wifi
                            title: qsTr("Connection Doctor")
                            description: qsTr("Diagnose and fix firewall, network, or VPN issues.")
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Check")
                                onClicked: AppController.runDoctor()
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.signal
                            title: qsTr("Connect over mobile data")
                            description: qsTr("Stay connected over LTE/5G or relay when away from Wi-Fi.")
                            Toggle {
                                checked: Preferences.awayMode
                                onToggled: function(v) { Preferences.awayMode = v }
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.power
                            title: qsTr("Wake from your phone")
                            description: {
                                const adapter = AppController.wakeAdapter
                                const state = AppController.wakeState
                                if (state === "none" || adapter.length === 0)
                                    return qsTr("No active network adapter found.")
                                if (state === "enabled")
                                    return qsTr("%1 · Wake on Magic Packet ready.").arg(adapter)
                                return qsTr("%1 · Enable Wake on Magic Packet in Device Manager.").arg(adapter)
                            }
                            Row {
                                spacing: 6
                                StatusDot {
                                    anchors.verticalCenter: parent.verticalCenter
                                    online: AppController.wakeState === "enabled"
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "bodySmall"
                                    muted: true
                                    text: AppController.wakeState === "enabled" ? qsTr("Ready")
                                        : AppController.wakeState === "disabled"
                                          ? (Qt.platform.os === "linux" ? qsTr("Off in the adapter's settings") : qsTr("Off in Windows"))
                                        : AppController.wakeState === "unknown" ? qsTr("Check settings")
                                        : qsTr("No adapter")
                                }
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.terminal
                            title: qsTr("Local automation API")
                            description: Preferences.localApiEnabled
                                ? qsTr("Listening on 127.0.0.1:%1 · Token in %2").arg(Preferences.localApiPort).arg(Preferences.localApiTokenPath)
                                : qsTr("Local HTTP/SSE server (127.0.0.1:%1) for scripts and tools.").arg(Preferences.localApiPort)
                            Row {
                                spacing: 10
                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 76
                                    height: 32
                                    radius: Theme.radiusSm
                                    color: Theme.surfaceContainerHighest
                                    border.width: localApiPortEdit.activeFocus ? 2 : 1
                                    border.color: localApiPortEdit.activeFocus ? Theme.primary : Theme.outlineVariant
                                    TextInput {
                                        id: localApiPortEdit
                                        anchors.fill: parent
                                        anchors.leftMargin: 8
                                        anchors.rightMargin: 8
                                        verticalAlignment: TextInput.AlignVCenter
                                        horizontalAlignment: TextInput.AlignHCenter
                                        color: Theme.surfaceContent
                                        selectionColor: Theme.primaryContainer
                                        selectedTextColor: Theme.primaryContainerContent
                                        font.family: Theme.fontMono
                                        font.pixelSize: 12
                                        text: String(Preferences.localApiPort)
                                        validator: IntValidator { bottom: 1024; top: 65535 }
                                        onEditingFinished: {
                                            const parsed = parseInt(text, 10)
                                            if (!isNaN(parsed) && parsed >= 1024 && parsed <= 65535)
                                                Preferences.localApiPort = parsed
                                            else
                                                text = String(Preferences.localApiPort)
                                        }
                                    }
                                }
                                Toggle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    label: qsTr("Local automation API")
                                    checked: Preferences.localApiEnabled
                                    onToggled: (on) => Preferences.localApiEnabled = on
                                }
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.desktop
                            title: qsTr("NectarLink %1").arg(AppController.version)
                            description: AppController.updateStatus.length > 0
                                         ? AppController.updateStatus
                                         : qsTr("This PC: %1").arg(AppController.deviceName)
                            Row {
                                spacing: 8
                                // The download's progress, or a spinner while its size isn't known.
                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: AppController.updateBusy && AppController.updateProgress >= 0
                                    width: 120
                                    height: 4
                                    radius: 2
                                    color: Theme.surfaceContainerHighest
                                    Accessible.role: Accessible.ProgressBar
                                    Accessible.name: qsTr("Update download")
                                    Rectangle {
                                        height: parent.height
                                        radius: parent.radius
                                        width: parent.width * Math.max(0, Math.min(1, AppController.updateProgress))
                                        color: Theme.primary
                                        Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: 180 } }
                                    }
                                }
                                Spinner {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: AppController.updateBusy && AppController.updateProgress < 0
                                }
                                Button {
                                    visible: AppController.canUpdate
                                    anchors.verticalCenter: parent.verticalCenter
                                    size: "sm"
                                    variant: AppController.updateVersion.length > 0 ? "fill" : "tonal"
                                    iconPath: Icons.refresh
                                    enabled: !AppController.updateBusy
                                    text: AppController.updateVersion.length > 0
                                          ? qsTr("Update to %1").arg(AppController.updateVersion)
                                          : qsTr("Check for updates")
                                    onClicked: AppController.updateVersion.length > 0
                                               ? AppController.installUpdate() : AppController.checkForUpdates()
                                }
                                Button {
                                    anchors.verticalCenter: parent.verticalCenter
                                    variant: "outline"
                                    size: "sm"
                                    iconPath: Icons.folder
                                    text: qsTr("Open logs")
                                    onClicked: Qt.openUrlExternally(AppController.logsUrl())
                                }
                            }
                        }
                        Divider { width: parent.width; visible: AppController.canUpdate }
                        ListRow {
                            width: parent.width
                            visible: AppController.canUpdate
                            iconPath: Icons.refresh
                            title: qsTr("Check for updates")
                            description: qsTr("Looks for a new version on GitHub once a day.")
                            Toggle {
                                label: qsTr("Check for updates")
                                checked: Preferences.autoUpdate
                                onToggled: (on) => Preferences.autoUpdate = on
                            }
                        }
                        ListRow {
                            width: parent.width
                            visible: AppController.canUpdate
                            enabled: Preferences.autoUpdate
                            opacity: enabled ? 1 : 0.5
                            iconPath: Icons.download
                            title: qsTr("Install updates automatically")
                            description: qsTr("When nobody is using the PC and nothing is in progress.")
                            Toggle {
                                label: qsTr("Install updates automatically")
                                checked: Preferences.installUpdates
                                onToggled: (on) => Preferences.installUpdates = on
                            }
                        }
                    }
                }

                Card {
                    width: parent.width
                    Column {
                        width: parent.width
                        ListRow {
                            width: parent.width
                            iconPath: Icons.compass
                            title: qsTr("Setup walkthrough")
                            description: qsTr("Re-open the setup guide.")
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Open")
                                onClicked: Preferences.restartWalkthrough()
                            }
                        }
                        Divider { width: parent.width }
                        ListRow {
                            width: parent.width
                            iconPath: Icons.book
                            title: qsTr("Features guide")
                            description: qsTr("What each feature needs and where to find it.")
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Open")
                                onClicked: AppController.currentPage = "guide"
                            }
                        }
                    }
                }
            }
        }
    }

    Connections {
        target: AppController
        function onHighlightTargetChanged() {
            const tgt = AppController.highlightTarget
            if (tgt.startsWith("settings:")) {
                const parts = tgt.split(":")
                if (parts.length > 1) {
                    let sec = parts[1]
                    if (sec === "camera") sec = "camera_audio"
                    else if (sec === "clipboard") sec = "files_sync"
                    else if (sec === "surfaces" || sec === "automation") sec = "shortcuts"
                    page.currentSection = sec
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: 10
        radius: Theme.radiusLg
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: opacity > 0
        opacity: AppController.highlightTarget.startsWith("settings:") ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    }

    Sheet {
        id: hotspotSheet
        property string deviceId
        property var row: null
        property var networks: []
        cardWidth: 420
        Column {
            width: parent.width
            spacing: 12
            Txt { width: parent.width; text: qsTr("Phone's hotspot network"); role: "headline"; wrapMode: Text.WordWrap }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: Qt.platform.os === "linux"
                    ? (hotspotSheet.networks.length > 0
                        ? qsTr("Wi-Fi networks this PC has joined before. Your system keeps their passwords; NectarLink never sees them.")
                        : qsTr("This PC hasn't saved any Wi-Fi networks yet. Join the phone's hotspot once from your system Wi-Fi list first."))
                    : (hotspotSheet.networks.length > 0
                        ? qsTr("Wi-Fi networks this PC has joined before. Windows keeps their passwords; NectarLink never sees them.")
                        : qsTr("This PC hasn't saved any Wi-Fi networks yet. Join the phone's hotspot once from Windows' Wi-Fi list first."))
            }
            // A PC can have saved dozens of networks: the list scrolls.
            ListView {
                width: parent.width
                height: Math.min(contentHeight, 320)
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                spacing: 4
                model: hotspotSheet.networks
                delegate: Button {
                    required property var modelData
                    width: ListView.view.width
                    readonly property bool chosen: hotspotSheet.row !== null && hotspotSheet.row.network === modelData
                    variant: chosen ? "tonal" : "text"
                    iconPath: chosen ? Icons.check : Icons.wifi
                    text: modelData
                    onClicked: {
                        AppController.setHotspotNetwork(hotspotSheet.deviceId, modelData)
                        if (hotspotSheet.row !== null)
                            hotspotSheet.row.network = modelData
                        hotspotSheet.close()
                    }
                }
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Don't join")
                    visible: hotspotSheet.row !== null && hotspotSheet.row.network.length > 0
                    onClicked: {
                        AppController.setHotspotNetwork(hotspotSheet.deviceId, "")
                        if (hotspotSheet.row !== null)
                            hotspotSheet.row.network = ""
                        hotspotSheet.close()
                    }
                }
                Button { variant: "text"; text: qsTr("Close"); onClicked: hotspotSheet.close() }
            }
        }
    }

    Sheet {
        id: languageSheet
        cardWidth: 400
        Column {
            width: parent.width
            spacing: 12
            Txt { width: parent.width; text: qsTr("Language"); role: "headline"; wrapMode: Text.WordWrap }
            Repeater {
                model: page.languages
                delegate: Button {
                    required property var modelData
                    width: parent.width
                    variant: modelData.code === Preferences.language ? "tonal" : "text"
                    iconPath: modelData.code === Preferences.language ? Icons.check : Icons.globe
                    text: modelData.name
                    onClicked: {
                        Preferences.language = modelData.code
                        languageSheet.close()
                    }
                }
            }
            Row {
                anchors.right: parent.right
                Button { variant: "text"; text: qsTr("Close"); onClicked: languageSheet.close() }
            }
        }
    }

    Sheet {
        id: micOutputSheet
        cardWidth: 440
        Column {
            width: parent.width
            spacing: 12
            Txt { width: parent.width; text: qsTr("Play the phone's microphone into"); role: "headline"; wrapMode: Text.WordWrap }
            Repeater {
                model: { try { return JSON.parse(PhoneMic.outputs) } catch (e) { return [] } }
                delegate: Button {
                    required property var modelData
                    width: parent.width
                    variant: modelData.id === PhoneMic.output ? "tonal" : "text"
                    iconPath: modelData.id === PhoneMic.output ? Icons.check : Icons.speaker
                    text: modelData.name
                    onClicked: {
                        PhoneMic.chooseOutput(modelData.id)
                        micOutputSheet.close()
                    }
                }
            }
            Row {
                anchors.right: parent.right
                Button { variant: "text"; text: qsTr("Close"); onClicked: micOutputSheet.close() }
            }
        }
    }

    Sheet {
        id: unpairSheet
        property string deviceId
        property string deviceName
        cardWidth: 400
        Column {
            width: parent.width
            spacing: 16
            Txt { width: parent.width; text: qsTr("Unpair %1?").arg(unpairSheet.deviceName); role: "headline"; wrapMode: Text.WordWrap }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("Both devices forget each other. You can pair them again any time.")
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button { variant: "text"; text: qsTr("Cancel"); onClicked: unpairSheet.close() }
                Button {
                    text: qsTr("Unpair")
                    onClicked: {
                        AppController.unpair(unpairSheet.deviceId)
                        unpairSheet.close()
                    }
                }
            }
        }
    }

    Sheet {
        id: clearEverythingSheet
        cardWidth: 420
        Column {
            width: parent.width
            spacing: 16
            Txt {
                width: parent.width
                text: qsTr("Clear all local history & caches?")
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("Deletes clipboard history, Timeline, notification history, cached messages, photo thumbnails and transfer history on this PC. Paired devices, settings and downloaded files are kept.")
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button { variant: "text"; text: qsTr("Cancel"); onClicked: clearEverythingSheet.close() }
                Button {
                    iconPath: Icons.trash
                    text: qsTr("Clear everything")
                    onClicked: {
                        AppController.clearEverything()
                        clearEverythingSheet.close()
                    }
                }
            }
        }
    }
}
