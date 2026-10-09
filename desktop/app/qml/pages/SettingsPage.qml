// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Settings: appearance, behavior, paired devices and what they may do, and
// details for support.
Item {
    id: page
    property bool active: false
    signal pairRequested

    onActiveChanged: {
        Webcam.setPreviewActive(active)
        if (active)
            AppController.refreshDataRetention()
    }
    Component.onCompleted: {
        if (active) {
            Webcam.setPreviewActive(true)
            AppController.refreshDataRetention()
        }
    }
    Component.onDestruction: Webcam.setPreviewActive(false)

    readonly property var retentionSummary: {
        try {
            return JSON.parse(AppController.dataRetentionSummary || "{}")
        } catch (e) {
            return ({})
        }
    }

    function formatRetentionBytes(bytes) {
        const b = Number(bytes || 0)
        if (b <= 0)
            return qsTr("%1 B").arg(0)
        if (b < 1024)
            return qsTr("%1 B").arg(b)
        if (b < 1024 * 1024)
            return qsTr("%1 KB").arg(Math.max(1, Math.round(b / 1024)))
        if (b < 1024 * 1024 * 1024)
            return qsTr("%1 MB").arg((b / (1024 * 1024)).toFixed(1))
        if (b < 1024 * 1024 * 1024 * 1024)
            return qsTr("%1 GB").arg((b / (1024 * 1024 * 1024)).toFixed(1))
        return qsTr("%1 TB").arg((b / (1024 * 1024 * 1024 * 1024)).toFixed(1))
    }

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    readonly property var toggleNames: ({
        notifications: qsTr("Notifications"),
        messages: qsTr("Messages"),
        calls: qsTr("Calls"),
        contacts: qsTr("Contacts"),
        clipboard: qsTr("Clipboard"),
        files: qsTr("Files"),
        recordings: qsTr("Voice recordings"),
        media: qsTr("Media playing on either device"),
        photos: qsTr("Photos"),
        toggles: qsTr("Phone controls"),
        pc_actions: qsTr("Lock, sleep and wake this PC"),
        mirroring: qsTr("Screen mirroring"),
        webcam: qsTr("Webcam"),
        storage: qsTr("Phone storage in File Explorer"),
        remote_input: qsTr("Control this PC's mouse, keyboard and Deck"),
        commands: qsTr("Allow running Deck commands"),
        remote_files: qsTr("Browse this PC's files while away")
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
        webcam: Icons.video,
        storage: Icons.folder,
        remote_input: Icons.deck,
        commands: Icons.terminal,
        remote_files: Icons.globe
    })

    readonly property var toggleDescriptions: ({
        webcam: qsTr("Use this phone's camera as a webcam on this PC."),
        storage: qsTr("Show this phone's storage in File Explorer so you can browse, open and drop in files."),
        commands: qsTr("Lets this phone trigger Deck tiles that run shell commands configured on this PC. Off by default — only enable for a phone you control.")
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
            keys: ["media", "toggles", "mirroring", "webcam", "pc_actions", "remote_input", "commands"]
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
        "media", "toggles", "mirroring", "webcam", "pc_actions", "remote_input", "commands"
    ]

    FolderDialog {
        id: recordingsFolderPicker
        title: qsTr("Save recordings in")
        onAccepted: Preferences.chooseRecordingsFolder(selectedFolder.toString())
    }

    Flickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.contentPadding * 2
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: column
            // A readable column, centered in wide windows.
            width: Math.min(760, parent.width - Theme.contentPadding * 2)
            x: Math.max(Theme.contentPadding, (parent.width - width) / 2)
            y: Theme.contentPadding
            spacing: Theme.gutter

            // ---- Appearance ----
            Txt { text: qsTr("Appearance"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.palette
                        title: qsTr("Theme")
                        description: qsTr("Bloom is soft and colorful; Graphite is ink on paper.")
                        Segmented {
                            options: [{ value: "bloom", label: qsTr("Bloom") }, { value: "graphite", label: qsTr("Graphite") }]
                            value: Preferences.theme
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
                    Divider { width: parent.width; visible: !Theme.graphite }
                    ListRow {
                        id: colorRow
                        width: parent.width
                        visible: !Theme.graphite
                        iconPath: Icons.sparkle
                        title: qsTr("Color")
                        description: qsTr("Match your wallpaper, your phone's colors, or pick a color.")
                        Column {
                            id: colorControl
                            readonly property bool inlineSinglePhone: Theme.phoneSeeds.length === 1 && colorRow.width >= 660
                            readonly property bool sideBySidePhones: Theme.phoneSeeds.length > 1 && colorRow.width >= 660
                            width: Math.max(
                                swatchesRow.implicitWidth,
                                phonePillsRow.visible ? phonePillsRow.implicitWidth : 0,
                                phonePillsCol.visible ? phonePillsCol.implicitWidth : 0
                            )
                            spacing: 8

                            Row {
                                id: swatchesRow
                                anchors.right: parent.right
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

                                // Single paired phone inline at standard window width
                                Repeater {
                                    model: colorControl.inlineSinglePhone ? Theme.phoneSeeds : []
                                    delegate: Rectangle {
                                        id: inlinePhoneSwatch
                                        required property var modelData
                                        required property int index
                                        readonly property bool selected: Preferences.seed === modelData.key
                                            || (Preferences.seed === "phone" && index === 0)
                                            || (modelData.id.length > 0 && Preferences.seed === "phone:" + modelData.id)
                                        readonly property var phonePalette: modelData.scheme
                                            ? modelData.scheme[Theme.dark ? "dark" : "light"] : null
                                        height: 28
                                        width: inlinePhoneRow.implicitWidth + 14
                                        radius: Theme.pill(height)
                                        activeFocusOnTab: true
                                        color: selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                                        border.width: selected || Theme.focusVisible(inlinePhoneSwatch) ? 2 : 0
                                        border.color: Theme.focusVisible(inlinePhoneSwatch) ? Theme.primary : Theme.surfaceContent
                                        Accessible.role: Accessible.RadioButton
                                        Accessible.name: modelData.label
                                        Accessible.checked: selected
                                        Accessible.onPressAction: Preferences.seed = modelData.key
                                        Keys.onPressed: (event) => {
                                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                Preferences.seed = inlinePhoneSwatch.modelData.key
                                                event.accepted = true
                                            }
                                        }
                                        Row {
                                            id: inlinePhoneRow
                                            anchors.centerIn: parent
                                            spacing: 6
                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: 18; height: 18; radius: 9
                                                color: inlinePhoneSwatch.phonePalette ? inlinePhoneSwatch.phonePalette.primary : Theme.primary
                                                Icon {
                                                    anchors.centerIn: parent
                                                    width: 11; height: 11
                                                    stroke: 2
                                                    path: Icons.phone
                                                    color: inlinePhoneSwatch.phonePalette ? inlinePhoneSwatch.phonePalette.onPrimary : Theme.primaryContent
                                                }
                                            }
                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "label"
                                                size: 12
                                                text: inlinePhoneSwatch.modelData.label
                                                color: inlinePhoneSwatch.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                                            }
                                        }
                                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                                        TapHandler { onTapped: Preferences.seed = inlinePhoneSwatch.modelData.key }
                                    }
                                }

                                // Preset seeds
                                Repeater {
                                    model: Object.keys(Tokens.data.themes.bloom.seeds)
                                    delegate: Rectangle {
                                        id: swatch
                                        required property string modelData
                                        readonly property bool selected: Preferences.seed === modelData
                                            || (!Theme.isPhoneSeed && Preferences.seed !== "wallpaper" && Theme.seed === modelData)
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
                                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                                        TapHandler { onTapped: Preferences.seed = swatch.modelData }
                                    }
                                }
                            }

                            // Multiple phones side-by-side at standard width
                            Row {
                                id: phonePillsRow
                                visible: !colorControl.inlineSinglePhone && colorControl.sideBySidePhones && Theme.phoneSeeds.length > 0
                                anchors.right: parent.right
                                spacing: 8
                                Repeater {
                                    model: phonePillsRow.visible ? Theme.phoneSeeds : []
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
                                                color: phoneSwatch.phonePalette ? phoneSwatch.phonePalette.primary : Theme.primary
                                                Icon {
                                                    anchors.centerIn: parent
                                                    width: 11; height: 11
                                                    stroke: 2
                                                    path: Icons.phone
                                                    color: phoneSwatch.phonePalette ? phoneSwatch.phonePalette.onPrimary : Theme.primaryContent
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

                            // Narrow window fallback: right-aligned column of phone pills
                            Column {
                                id: phonePillsCol
                                visible: !colorControl.inlineSinglePhone && !colorControl.sideBySidePhones && Theme.phoneSeeds.length > 0
                                anchors.right: parent.right
                                spacing: 6
                                Repeater {
                                    model: phonePillsCol.visible ? Theme.phoneSeeds : []
                                    delegate: Rectangle {
                                        id: narrowPhoneSwatch
                                        required property var modelData
                                        required property int index
                                        readonly property bool selected: Preferences.seed === modelData.key
                                            || (Preferences.seed === "phone" && index === 0)
                                            || (modelData.id.length > 0 && Preferences.seed === "phone:" + modelData.id)
                                        readonly property var phonePalette: modelData.scheme
                                            ? modelData.scheme[Theme.dark ? "dark" : "light"] : null
                                        anchors.right: parent.right
                                        height: 28
                                        width: narrowPhoneRow.implicitWidth + 14
                                        radius: Theme.pill(height)
                                        activeFocusOnTab: true
                                        color: selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                                        border.width: selected || Theme.focusVisible(narrowPhoneSwatch) ? 2 : 0
                                        border.color: Theme.focusVisible(narrowPhoneSwatch) ? Theme.primary : Theme.surfaceContent
                                        Accessible.role: Accessible.RadioButton
                                        Accessible.name: modelData.label
                                        Accessible.checked: selected
                                        Accessible.onPressAction: Preferences.seed = modelData.key
                                        Keys.onPressed: (event) => {
                                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                                Preferences.seed = narrowPhoneSwatch.modelData.key
                                                event.accepted = true
                                            }
                                        }
                                        Row {
                                            id: narrowPhoneRow
                                            anchors.centerIn: parent
                                            spacing: 6
                                            Rectangle {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: 18; height: 18; radius: 9
                                                color: narrowPhoneSwatch.phonePalette ? narrowPhoneSwatch.phonePalette.primary : Theme.primary
                                                Icon {
                                                    anchors.centerIn: parent
                                                    width: 11; height: 11
                                                    stroke: 2
                                                    path: Icons.phone
                                                    color: narrowPhoneSwatch.phonePalette ? narrowPhoneSwatch.phonePalette.onPrimary : Theme.primaryContent
                                                }
                                            }
                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                role: "label"
                                                size: 12
                                                text: narrowPhoneSwatch.modelData.label
                                                color: narrowPhoneSwatch.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                                            }
                                        }
                                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                                        TapHandler { onTapped: Preferences.seed = narrowPhoneSwatch.modelData.key }
                                    }
                                }
                            }
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.desktop
                        title: qsTr("Mica backdrop")
                        description: qsTr("Let the desktop's colors show softly through the window.")
                        Toggle {
                            label: qsTr("Mica backdrop")
                            checked: Preferences.backdrop
                            onToggled: (on) => Preferences.backdrop = on
                        }
                    }
                }
            }

            // ---- Behavior ----
            Txt { text: qsTr("Behavior"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.power
                        title: qsTr("Start with Windows")
                        description: qsTr("Nectarlink starts in the notification area when you sign in, so your phone can reach this PC right away.")
                        Toggle {
                            label: qsTr("Start with Windows")
                            checked: Preferences.startWithWindows
                            onToggled: (on) => Preferences.startWithWindows = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.power
                        title: qsTr("Keep running when the window is closed")
                        description: qsTr("Nectarlink stays in the notification area so your phone can reach this PC.")
                        Toggle {
                            label: qsTr("Keep running when the window is closed")
                            checked: Preferences.closeToTray
                            onToggled: (on) => Preferences.closeToTray = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.clipboard
                        title: qsTr("Send what you copy to your phone")
                        description: qsTr("Text and images you copy on this PC are ready to paste on your phone. Passwords from password managers are never sent.")
                        Toggle {
                            label: qsTr("Send what you copy to your phone")
                            checked: Preferences.autoClipboard
                            onToggled: (on) => Preferences.autoClipboard = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.sparkle
                        title: qsTr("Suggest actions for copied text")
                        description: qsTr("Offer to open links, look up addresses in Maps, call numbers, track parcels, or compose emails when text arrives from your phone.")
                        Toggle {
                            label: qsTr("Suggest actions for copied text")
                            checked: Preferences.suggestClipboardActions
                            onToggled: (on) => Preferences.suggestClipboardActions = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.copy
                        title: qsTr("Copy one-time codes automatically")
                        description: qsTr("Copy verification codes from your phone's notifications and texts as they arrive. They're never sent back or saved.")
                        Toggle {
                            label: qsTr("Copy one-time codes automatically")
                            checked: Preferences.autoCopyOtp
                            onToggled: (on) => Preferences.autoCopyOtp = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.pause
                        title: qsTr("Pause media during phone calls")
                        description: qsTr("Pause what's playing on this PC when your phone rings, and resume it after the call.")
                        Toggle {
                            label: qsTr("Pause media during phone calls")
                            checked: Preferences.pauseMediaOnCall
                            onToggled: (on) => Preferences.pauseMediaOnCall = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.send
                        title: qsTr("Show your phones in File Explorer")
                        description: qsTr("Right-click files, choose Send to, then your phone.")
                        Toggle {
                            label: qsTr("Show your phones in File Explorer")
                            checked: Preferences.sendToMenu
                            onToggled: (on) => Preferences.sendToMenu = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.globe
                        title: qsTr("LocalSend on local network")
                        description: qsTr("Send and receive files with devices running LocalSend on the same Wi‑Fi network without pairing. Incoming files ask before saving.")
                        Toggle {
                            label: qsTr("LocalSend on local network")
                            checked: AppController.localsendEnabled
                            onToggled: (on) => AppController.toggleLocalsend(on)
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.battery
                        title: qsTr("Battery alerts")
                        description: qsTr("Get a notification when your phone's battery is low, and when it's fully charged.")
                        Toggle {
                            label: qsTr("Battery alerts")
                            checked: Preferences.batteryAlerts
                            onToggled: (on) => Preferences.batteryAlerts = on
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.search
                        title: qsTr("Command palette shortcut")
                        description: qsTr("Global shortcut to open the Command Palette from any app so you can run actions, text or call contacts, or mirror apps.")
                        Row {
                            spacing: 8
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
                                width: 132
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: paletteHotkeyInput.activeFocus ? 2 : 1
                                border.color: paletteHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: paletteHotkeyInput
                                    anchors.fill: parent
                                    anchors.leftMargin: 10
                                    anchors.rightMargin: 10
                                    verticalAlignment: TextInput.AlignVCenter
                                    horizontalAlignment: TextInput.AlignHCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: 13
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    clip: true
                                    text: Preferences.commandPaletteHotkey
                                    onEditingFinished: Preferences.commandPaletteHotkey = text.trim()
                                    Accessible.name: qsTr("Command palette shortcut")
                                }
                            }
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.folder
                        title: qsTr("Shelf shortcut")
                        description: qsTr("Global shortcut to slide out the Shelf at the screen edge without stealing focus.")
                        Row {
                            spacing: 8
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
                                width: 132
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: shelfHotkeyInput.activeFocus ? 2 : 1
                                border.color: shelfHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: shelfHotkeyInput
                                    anchors.fill: parent
                                    anchors.leftMargin: 10
                                    anchors.rightMargin: 10
                                    verticalAlignment: TextInput.AlignVCenter
                                    horizontalAlignment: TextInput.AlignHCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: 13
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    clip: true
                                    text: Preferences.shelfHotkey
                                    onEditingFinished: Preferences.shelfHotkey = text.trim()
                                    Accessible.name: qsTr("Shelf shortcut")
                                }
                            }
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.camera
                        title: qsTr("Take photo shortcut")
                        description: qsTr("Global shortcut to open your phone's camera from any app and paste the photo back into the active window.")
                        Row {
                            spacing: 8
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
                                width: 132
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: photoHotkeyInput.activeFocus ? 2 : 1
                                border.color: photoHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: photoHotkeyInput
                                    anchors.fill: parent
                                    anchors.leftMargin: 10
                                    anchors.rightMargin: 10
                                    verticalAlignment: TextInput.AlignVCenter
                                    horizontalAlignment: TextInput.AlignHCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: 13
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    clip: true
                                    text: Preferences.continuityPhotoHotkey
                                    onEditingFinished: Preferences.continuityPhotoHotkey = text.trim()
                                    Accessible.name: qsTr("Take photo shortcut")
                                }
                            }
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.clipboardList
                        title: qsTr("Scan document shortcut")
                        description: qsTr("Global shortcut to scan a document with your phone's camera and paste the cropped scan into the active window.")
                        Row {
                            spacing: 8
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
                                width: 132
                                height: 32
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHighest
                                border.width: scanHotkeyInput.activeFocus ? 2 : 1
                                border.color: scanHotkeyInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: scanHotkeyInput
                                    anchors.fill: parent
                                    anchors.leftMargin: 10
                                    anchors.rightMargin: 10
                                    verticalAlignment: TextInput.AlignVCenter
                                    horizontalAlignment: TextInput.AlignHCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: 13
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    clip: true
                                    text: Preferences.continuityScanHotkey
                                    onEditingFinished: Preferences.continuityScanHotkey = text.trim()
                                    Accessible.name: qsTr("Scan document shortcut")
                                }
                            }
                        }
                    }
                }
            }

            // ---- Recordings ----
            Txt { text: qsTr("Recordings"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.folder
                        title: qsTr("Save recordings in")
                        description: Preferences.recordingsFolder
                        Row {
                            spacing: 8
                            Button {
                                visible: !Preferences.recordingsFolderIsDefault
                                variant: "text"
                                size: "sm"
                                text: qsTr("Reset")
                                onClicked: Preferences.resetRecordingsFolder()
                            }
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Choose folder")
                                onClicked: recordingsFolderPicker.open()
                            }
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.mic
                        title: qsTr("Format")
                        description: qsTr("Keep M4A as recorded, or convert on this PC.")
                        Segmented {
                            options: [
                                { value: "m4a", label: "M4A" },
                                { value: "mp3", label: "MP3" },
                                { value: "wav", label: "WAV" },
                                { value: "flac", label: "FLAC" }
                            ]
                            value: Preferences.recordingsFormat
                            onPicked: (value) => Preferences.recordingsFormat = value
                        }
                    }
                }
            }

            // ---- Webcam ----
            Txt { text: qsTr("Webcam"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.video
                        title: qsTr("Virtual camera add-on")
                        description: Webcam.addonRegistered
                            ? qsTr("\"Nectarlink Webcam\" is registered for Zoom, Teams, Meet and other apps.")
                            : qsTr("Set up once (needs administrator approval) so Zoom, Teams, Meet and other apps see \"Nectarlink Webcam\".")
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
                        description: qsTr("Which phone's camera to use on this PC.")
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
                    ListRow {
                        width: parent.width
                        iconPath: Icons.camera
                        title: qsTr("Resolution")
                        description: qsTr("720p uses less battery; 1080p is sharper.")
                        Segmented {
                            options: [
                                { value: "720", label: "720p" },
                                { value: "1080", label: "1080p" }
                            ]
                            value: String(Webcam.height)
                            onPicked: (value) => Webcam.setResolution(Number(value))
                        }
                    }
                    Divider { width: parent.width }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.mirror
                        title: qsTr("Mirror image")
                        description: qsTr("Flip the camera picture horizontally.")
                        Toggle {
                            label: qsTr("Mirror image")
                            checked: Webcam.mirror
                            onToggled: (on) => Webcam.setMirrorImage(on)
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

            // ---- Connection & this PC ----
            Txt { text: qsTr("Connection & this PC"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.wifi
                        title: qsTr("Connection Doctor")
                        description: qsTr("Finds what keeps your phone from reaching this PC (the firewall, the network, a VPN) and fixes what it can.")
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
                        iconPath: Icons.power
                        title: qsTr("Wake from your phone")
                        description: {
                            const adapter = AppController.wakeAdapter
                            const state = AppController.wakeState
                            const wired = AppController.wakeWired
                            if (state === "none" || adapter.length === 0)
                                return qsTr("No active Ethernet or Wi-Fi adapter found. Connect this PC to your network so your phone can learn how to wake it.")
                            if (state === "enabled") {
                                return wired
                                    ? qsTr("%1 · Wake on Magic Packet is on. If this PC still won't wake from sleep or shutdown, check that Wake-on-LAN is enabled in BIOS/UEFI.").arg(adapter)
                                    : qsTr("%1 · Wake on Magic Packet is on. Wi-Fi wake usually works from sleep, not full shutdown; use Ethernet for shutdown wake.").arg(adapter)
                            }
                            return wired
                                ? qsTr("%1 · To wake this PC while it sleeps or is shut down, enable Wake on Magic Packet in Device Manager → Network adapters → %1 → Properties (Advanced tab → Wake on Magic Packet → Enabled; Power Management tab → Allow this device to wake the computer, Only allow a magic packet), and enable Wake-on-LAN in BIOS/UEFI.").arg(adapter)
                                : qsTr("%1 · To wake this PC while it sleeps or is shut down, enable Wake on Magic Packet in Device Manager → Network adapters → %1 → Properties (Advanced tab → Wake on Magic Packet → Enabled; Power Management tab → Allow this device to wake the computer, Only allow a magic packet), and enable Wake-on-LAN in BIOS/UEFI. Wi-Fi wake rarely works from shutdown.").arg(adapter)
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
                                    : AppController.wakeState === "disabled" ? qsTr("Off in Windows")
                                    : AppController.wakeState === "unknown" ? qsTr("Check settings")
                                    : qsTr("No adapter")
                            }
                        }
                    }
                }
            }

            // ---- Notifications ----
            Txt { text: qsTr("Notifications"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.moon
                        title: qsTr("Follow your phone's Do not disturb")
                        description: qsTr("No pop-ups or sounds for phone notifications while your phone is on Do not disturb. They still show in the app.")
                        Toggle {
                            label: qsTr("Follow your phone's Do not disturb")
                            checked: Preferences.syncDnd
                            onToggled: (on) => Preferences.syncDnd = on
                        }
                    }
                    Divider { width: parent.width }
                    // Windows' Do not disturb holds back phone notifications
                    // too (they still reach the feed), so quiet hours are its
                    // schedule rather than a second one.
                    ListRow {
                        width: parent.width
                        iconPath: Icons.moon
                        title: qsTr("Quiet hours")
                        description: qsTr("When Windows' Do not disturb is on, phone notifications don't pop up; they wait in the app. Set when it turns on by itself in Windows Settings.")
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
                                height: 60
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

            // ---- Data & storage ----
            Txt { text: qsTr("Data & storage"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    // 1. Clipboard history
                    ListRow {
                        width: parent.width
                        iconPath: Icons.clipboard
                        title: qsTr("Clipboard history")
                        description: {
                            const count = Number(page.retentionSummary.clipboardItems || 0)
                            const size = page.formatRetentionBytes(page.retentionSummary.clipboardBytes)
                            return qsTr("7 days · up to 50 text and image clips encrypted on this PC (%1 stored · %2). Passwords and one-time codes are never saved.")
                                .arg(count)
                                .arg(size)
                        }
                        Row {
                            spacing: 10
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
                    Divider { width: parent.width }
                    // 2. Cross-device timeline
                    ListRow {
                        width: parent.width
                        iconPath: Icons.history
                        title: qsTr("Cross-device timeline")
                        description: {
                            const count = Number(page.retentionSummary.timelineItems || 0)
                            return count === 1
                                ? qsTr("Searchable local history of files, clips, links, photos, recordings and sessions (1 item · up to 5,000).")
                                : qsTr("Searchable local history of files, clips, links, photos, recordings and sessions (%1 items · up to 5,000).")
                                    .arg(count)
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
                    // 3. Notification history
                    ListRow {
                        width: parent.width
                        iconPath: Icons.bell
                        title: qsTr("Notification history")
                        description: {
                            const count = Number(page.retentionSummary.notificationHistory || 0)
                            const size = page.formatRetentionBytes(page.retentionSummary.notificationImageBytes)
                            return qsTr("24 hours · up to 300 dismissed notifications (%1 stored · %2 of cached images).")
                                .arg(count)
                                .arg(size)
                        }
                        Row {
                            spacing: 10
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
                    Divider { width: parent.width }
                    // 4. Cached messages & conversations
                    ListRow {
                        width: parent.width
                        iconPath: Icons.messages
                        title: qsTr("Cached messages & conversations")
                        description: {
                            const chatThreads = Number(page.retentionSummary.chatThreads || 0)
                            const smsThreads = Number(page.retentionSummary.cachedSmsThreads || 0)
                            const convs = chatThreads + smsThreads
                            const totalMsgs = Number(page.retentionSummary.chatMessages || 0)
                                              + Number(page.retentionSummary.cachedSmsMessages || 0)
                            const size = page.formatRetentionBytes(page.retentionSummary.messageAttachmentBytes)
                            const convPart = convs === 1 ? qsTr("1 conversation") : qsTr("%1 conversations").arg(convs)
                            const msgPart = totalMsgs === 1 ? qsTr("1 message") : qsTr("%1 messages").arg(totalMsgs)
                            return qsTr("90 days · up to 200 messages per conversation (%1 · %2 · %3 of attachments).")
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
                    // 5. Photo thumbnails cache
                    ListRow {
                        width: parent.width
                        iconPath: Icons.photo
                        title: qsTr("Photo thumbnails cache")
                        description: {
                            const files = Number(page.retentionSummary.photoThumbFiles || 0)
                            const size = page.formatRetentionBytes(page.retentionSummary.photoThumbBytes)
                            return files === 1
                                ? qsTr("30 days · up to 500 cached thumbnails (1 file · %1). Original photos on your phone are never touched.").arg(size)
                                : qsTr("30 days · up to 500 cached thumbnails (%1 files · %2). Original photos on your phone are never touched.")
                                    .arg(files)
                                    .arg(size)
                        }
                        Button {
                            variant: "text"
                            size: "sm"
                            visible: Number(page.retentionSummary.photoThumbFiles || 0) > 0
                            text: qsTr("Clear")
                            onClicked: AppController.clearPhotoThumbnails()
                        }
                    }
                    Divider { width: parent.width }
                    // 6. Received file transfer history
                    ListRow {
                        width: parent.width
                        iconPath: Icons.send
                        title: qsTr("Received file transfer history")
                        description: {
                            const records = Number(page.retentionSummary.receivedFileRecords || 0)
                            return records === 1
                                ? qsTr("Kept with timeline · 1 transfer record on this PC. Saved files in Downloads are kept.")
                                : qsTr("Kept with timeline · %1 transfer records on this PC. Saved files in Downloads are kept.")
                                    .arg(records)
                        }
                        Button {
                            variant: "text"
                            size: "sm"
                            visible: Number(page.retentionSummary.receivedFileRecords || 0) > 0
                            text: qsTr("Clear")
                            onClicked: AppController.clearReceivedFileHistory()
                        }
                    }
                    Divider { width: parent.width }
                    // 7. Clear everything
                    ListRow {
                        width: parent.width
                        iconPath: Icons.trash
                        title: qsTr("Clear all local history & caches")
                        description: qsTr("Wipes clipboard history, timeline, notification history, cached messages, photo thumbnails, and transfer history on this PC. Paired devices and settings are kept.")
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

            // ---- Devices ----
            Row {
                width: parent.width
                Txt { text: qsTr("Devices"); role: "label"; muted: true; anchors.verticalCenter: parent.verticalCenter }
            }
            Repeater {
                model: DeviceList
                delegate: Card {
                    id: deviceCard
                    required property string deviceId
                    required property string name
                    required property bool online
                    width: column.width

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
                                        Repeater {
                                            model: groupCol.groupItems
                                            delegate: Column {
                                                required property var modelData
                                                required property int index
                                                width: parent.width
                                                Divider {
                                                    width: parent.width
                                                    visible: index > 0
                                                }
                                                ListRow {
                                                    width: parent.width
                                                    iconPath: page.toggleIcons[modelData.name] || ""
                                                    title: page.toggleNames[modelData.name] || modelData.name
                                                    description: page.toggleDescriptions[modelData.name] || ""
                                                    Row {
                                                        spacing: 10
                                                        Button {
                                                            visible: modelData.name === "storage" && modelData.on
                                                            anchors.verticalCenter: parent.verticalCenter
                                                            variant: "tonal"
                                                            size: "sm"
                                                            iconPath: Icons.folder
                                                            text: qsTr("Open in File Explorer")
                                                            onClicked: AppController.openPhoneStorage(deviceCard.deviceId)
                                                        }
                                                        Toggle {
                                                            anchors.verticalCenter: parent.verticalCenter
                                                            label: page.toggleNames[modelData.name] || modelData.name
                                                            checked: modelData.on
                                                            onToggled: (on) => AppController.setDeviceToggle(deviceCard.deviceId, modelData.name, on)
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
            Button {
                variant: "tonal"
                iconPath: Icons.plus
                text: qsTr("Pair a new device")
                onClicked: page.pairRequested()
            }

            // ---- About ----
            Txt { text: qsTr("About"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 8
                    Txt { text: qsTr("Nectarlink %1").arg(AppController.version); role: "title" }
                    Txt { text: qsTr("This PC: %1").arg(AppController.deviceName); role: "bodySmall"; muted: true }
                    // Updates (installed copies only).
                    Row {
                        visible: AppController.canUpdate
                        spacing: 10
                        Button {
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
                        Spinner { anchors.verticalCenter: parent.verticalCenter; visible: AppController.updateBusy }
                    }
                    Item {
                        width: parent.width
                        height: autoUpdate.implicitHeight
                        visible: AppController.canUpdate
                        Txt {
                            anchors.left: parent.left
                            anchors.right: autoUpdate.left
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Check for updates automatically")
                            role: "body"
                        }
                        Toggle {
                            id: autoUpdate
                            anchors.right: parent.right
                            label: qsTr("Check for updates automatically")
                            checked: Preferences.autoUpdate
                            onToggled: (on) => Preferences.autoUpdate = on
                        }
                    }
                    Button {
                        variant: "outline"
                        size: "sm"
                        iconPath: Icons.folder
                        text: qsTr("Open the logs folder")
                        onClicked: Qt.openUrlExternally(AppController.logsUrl())
                    }
                }
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
                text: qsTr("This wipes clipboard history, the cross-device timeline, notification history, cached messages, photo thumbnails, and transfer history on this PC. Your paired devices, settings, and saved files in Downloads are kept.")
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
