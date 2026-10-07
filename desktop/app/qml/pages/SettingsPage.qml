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

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
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
        pc_actions: qsTr("Lock and sleep this PC"),
        remote_input: qsTr("Control this PC's mouse and keyboard"),
        remote_files: qsTr("Browse this PC's files while away")
    })

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
                        iconPath: Icons.info
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
                        width: parent.width
                        visible: !Theme.graphite
                        iconPath: Icons.sparkle
                        title: qsTr("Color")
                        description: qsTr("Match your wallpaper, or pick a color.")
                        Row {
                            spacing: 10
                            Repeater {
                                model: ["wallpaper"].concat(Object.keys(Tokens.data.themes.bloom.seeds))
                                delegate: Rectangle {
                                    id: swatch
                                    required property string modelData
                                    readonly property bool wallpaper: modelData === "wallpaper"
                                    readonly property bool selected: Preferences.seed === "wallpaper"
                                        ? wallpaper : Theme.seed === modelData
                                    // The wallpaper swatch shows the accent it gives, in the
                                    // current mode, with a picture mark on it.
                                    readonly property var wallpaperPalette: Theme.wallpaperSeed
                                        ? Theme.wallpaperSeed[Theme.dark ? "dark" : "light"] : null
                                    width: 26; height: 26; radius: 13
                                    color: !wallpaper ? Tokens.data.themes.bloom.seeds[modelData].seed
                                        : wallpaperPalette ? wallpaperPalette.primary : Theme.surfaceContainerHighest
                                    border.width: selected ? 2 : 0
                                    border.color: Theme.surfaceContent
                                    Accessible.role: Accessible.RadioButton
                                    Accessible.name: wallpaper ? qsTr("Wallpaper")
                                        : modelData.charAt(0).toUpperCase() + modelData.slice(1)
                                    Accessible.checked: selected
                                    Accessible.onPressAction: Preferences.seed = modelData
                                    Icon {
                                        visible: swatch.wallpaper
                                        anchors.centerIn: parent
                                        width: 14; height: 14
                                        stroke: 2
                                        path: Icons.photo
                                        color: swatch.wallpaperPalette ? swatch.wallpaperPalette.onPrimary : Theme.surfaceContent
                                    }
                                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                                    TapHandler { onTapped: Preferences.seed = swatch.modelData }
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
                        iconPath: Icons.battery
                        title: qsTr("Battery alerts")
                        description: qsTr("Get a notification when your phone's battery is low, and when it's fully charged.")
                        Toggle {
                            label: qsTr("Battery alerts")
                            checked: Preferences.batteryAlerts
                            onToggled: (on) => Preferences.batteryAlerts = on
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

            // ---- Connection ----
            Txt { text: qsTr("Connection"); role: "label"; muted: true }
            Card {
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
            }

            // ---- Notifications ----
            Txt { text: qsTr("Notifications"); role: "label"; muted: true }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    ListRow {
                        width: parent.width
                        iconPath: Icons.history
                        title: qsTr("Keep a day of history")
                        description: qsTr("Notifications that go away on your phone stay in History on this PC for a day.")
                        Toggle {
                            label: qsTr("Keep a day of history")
                            checked: NotificationList.historyEnabled
                            onToggled: (on) => NotificationList.setHistoryEnabled(on)
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
                        spacing: 4
                        Row {
                            width: parent.width
                            spacing: 12
                            Avatar { iconPath: Icons.phone; emphasized: deviceCard.online }
                            Column {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 36 - 12 - unpair.width - 12
                                Txt { width: parent.width; text: deviceCard.name; role: "title" }
                                Txt {
                                    text: deviceCard.online ? qsTr("Connected") : qsTr("Not connected")
                                    role: "bodySmall"
                                    muted: true
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
                        Item { width: 1; height: 8 }
                        Repeater {
                            model: deviceCard.toggles
                            delegate: ListRow {
                                required property var modelData
                                width: parent.width
                                title: page.toggleNames[modelData.name] || modelData.name
                                Toggle {
                                    label: page.toggleNames[modelData.name] || modelData.name
                                    checked: modelData.on
                                    onToggled: (on) => AppController.setDeviceToggle(deviceCard.deviceId, modelData.name, on)
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
                    TextEdit {
                        width: parent.width
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.WrapAnywhere
                        text: AppController.deviceId
                        color: Theme.surfaceContentVariant
                        selectionColor: Theme.primaryContainer
                        selectedTextColor: Theme.primaryContainerContent
                        font.family: Theme.fontMono
                        font.pixelSize: 12
                        Accessible.name: qsTr("Device ID")
                    }
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
}
