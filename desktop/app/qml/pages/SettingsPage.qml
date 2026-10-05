// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
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
        clipboard: qsTr("Clipboard"),
        photos: qsTr("Photos"),
        pc_actions: qsTr("Lock and sleep this PC"),
        remote_files: qsTr("Browse this PC's files while away")
    })

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
                        Row {
                            spacing: 10
                            Repeater {
                                model: Object.keys(Tokens.data.themes.bloom.seeds)
                                delegate: Rectangle {
                                    id: swatch
                                    required property string modelData
                                    readonly property bool selected: Theme.seed === modelData
                                    width: 26; height: 26; radius: 13
                                    color: Tokens.data.themes.bloom.seeds[modelData].seed
                                    border.width: selected ? 2 : 0
                                    border.color: Theme.surfaceContent
                                    Accessible.role: Accessible.RadioButton
                                    Accessible.name: modelData
                                    Accessible.checked: selected
                                    Accessible.onPressAction: Preferences.seed = modelData
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
