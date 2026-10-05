// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Home: the paired phone at a glance (status, battery, power level) and
// quick actions. Actions reflect the capability matrix: what isn't
// available shows what unlocks it.
Item {
    id: page
    property bool active: true
    property int current: 0
    signal pairRequested

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    onCurrentChanged: if (current >= DeviceList.count) current = 0
    Connections {
        target: DeviceList
        function onCountChanged() { if (page.current >= DeviceList.count) page.current = Math.max(0, DeviceList.count - 1) }
    }

    function relativeTime(unixSeconds) {
        if (!unixSeconds)
            return qsTr("never")
        const minutes = Math.round((Date.now() / 1000 - unixSeconds) / 60)
        if (minutes < 1) return qsTr("just now")
        if (minutes < 60) return qsTr("%n min ago", "", minutes)
        const hours = Math.round(minutes / 60)
        if (hours < 24) return qsTr("%n h ago", "", hours)
        return new Date(unixSeconds * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    Flickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.contentPadding * 2
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: column
            x: Theme.contentPadding
            y: Theme.contentPadding
            width: parent.width - Theme.contentPadding * 2
            spacing: Theme.gutter

            // Device switcher (only with more than one device).
            Row {
                spacing: 8
                visible: DeviceList.count > 1
                Repeater {
                    model: DeviceList
                    delegate: Chip {
                        required property int index
                        required property string name
                        required property string kind
                        text: name
                        iconPath: kind === "tablet" || kind === "phone" ? Icons.phone : Icons.laptop
                        selected: index === page.current
                        TapHandler { onTapped: page.current = index }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                    }
                }
            }

            Repeater {
                model: DeviceList
                delegate: DeviceHome {
                    required property int index
                    width: column.width
                    visible: index === page.current
                }
            }
        }
    }

    component DeviceHome: Row {
        id: home
        required property string deviceId
        required property string name
        required property string kind
        required property string osVersion
        required property string model
        required property bool online
        required property string path
        required property int rttMs
        required property int battery
        required property bool charging
        required property string power
        required property real lastSeen
        required property real pairedAt

        // Re-read capabilities whenever any matrix changes.
        function feature(id) {
            return AppController.capsRevision >= 0 ? AppController.featureState(deviceId, id) : ({})
        }
        property bool ringing: false
        Timer { id: ringTimeout; interval: 30000; onTriggered: home.ringing = false }

        spacing: Theme.gutter
        readonly property real sideWidth: 300

        Column {
            width: home.width - home.sideWidth - home.spacing
            spacing: Theme.gutter

            // ---- Hero ----
            Rectangle {
                width: parent.width
                height: 190
                radius: Theme.radiusXl
                color: Theme.heroColor
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant

                Rectangle {
                    id: phoneArt
                    x: 24
                    anchors.verticalCenter: parent.verticalCenter
                    width: 86; height: 150
                    radius: 20
                    border.width: 3
                    border.color: Theme.graphite ? Theme.surfaceContent : Theme.primaryContainerContent
                    gradient: Gradient {
                        GradientStop { position: 0; color: Theme.graphite ? Theme.surfaceContainerHigh : Theme.primary }
                        GradientStop { position: 1; color: Theme.graphite ? Theme.surfaceContainerHighest : Qt.darker(Theme.primary, 2.0) }
                    }
                    Rectangle {
                        anchors.horizontalCenter: parent.horizontalCenter
                        y: 8
                        width: 22; height: 5; radius: 2.5
                        color: phoneArt.border.color
                    }
                }
                Column {
                    anchors.left: phoneArt.right
                    anchors.leftMargin: 24
                    anchors.right: parent.right
                    anchors.rightMargin: 24
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8
                    Txt {
                        width: parent.width
                        text: home.name
                        role: "displaySmall"
                        color: Theme.heroContent
                    }
                    Txt {
                        width: parent.width
                        role: "body"
                        color: Theme.heroContent
                        opacity: 0.78
                        text: [home.model, home.osVersion.length > 0 ? qsTr("Android %1").arg(home.osVersion) : ""]
                              .filter(s => s.length > 0).join(" · ")
                    }
                    Flow {
                        width: parent.width
                        spacing: 8
                        Chip {
                            visible: home.battery >= 0
                            iconPath: Icons.battery
                            text: home.charging ? qsTr("%1% · charging").arg(home.battery) : qsTr("%1%").arg(home.battery)
                        }
                        Chip {
                            iconPath: home.online ? Icons.wifi : Icons.unlink
                            text: home.online
                                  ? (home.rttMs >= 1
                                     ? (home.path === "relay" ? qsTr("Away · %1 ms") : qsTr("Wi‑Fi · %1 ms")).arg(home.rttMs)
                                     : (home.path === "relay" ? qsTr("Away") : qsTr("Wi‑Fi")))
                                  : (home.lastSeen > 0 ? qsTr("Offline · seen %1").arg(page.relativeTime(home.lastSeen))
                                                       : qsTr("Not connected yet"))
                        }
                        Chip {
                            visible: home.power === "assist" || home.power === "elevated"
                            iconPath: Icons.sparkle
                            text: home.power === "elevated" ? qsTr("Elevated") : qsTr("Assist")
                        }
                    }
                }
            }

            // ---- Quick actions ----
            Grid {
                id: actions
                width: parent.width
                columns: width > 640 ? 4 : 2
                spacing: 12
                readonly property real tileWidth: (width - spacing * (columns - 1)) / columns

                ActionTile {
                    width: actions.tileWidth
                    title: home.ringing ? qsTr("Stop ringing") : qsTr("Find phone")
                    subtitle: qsTr("Rings even on silent")
                    iconPath: Icons.ring
                    feature: home.feature("device.find_phone")
                    active: home.ringing
                    onClicked: {
                        home.ringing = !home.ringing
                        if (home.ringing) ringTimeout.restart()
                        AppController.ring(home.deviceId, home.ringing)
                    }
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Send clipboard")
                    subtitle: qsTr("To the phone")
                    iconPath: Icons.clipboard
                    feature: home.feature("clipboard.pc_to_phone")
                    ready: false
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Mirror screen")
                    subtitle: qsTr("View and control")
                    iconPath: Icons.mirror
                    feature: home.feature("mirroring.view")
                    ready: false
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Send files")
                    subtitle: qsTr("Or drop anywhere")
                    iconPath: Icons.send
                    feature: home.feature("files.send")
                    ready: false
                }
            }
        }

        // ---- Details ----
        Column {
            width: home.sideWidth
            spacing: Theme.gutter

            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 12
                    Txt { text: qsTr("Connection"); role: "label"; muted: true }
                    Row {
                        spacing: 10
                        StatusDot { anchors.verticalCenter: parent.verticalCenter; online: home.online }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "body"
                            text: home.online
                                  ? (home.path === "relay" ? qsTr("Connected while away") : qsTr("Connected on this network"))
                                  : qsTr("Not connected")
                        }
                    }
                    Txt {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        role: "bodySmall"
                        muted: true
                        text: home.online
                              ? (home.rttMs >= 1 ? qsTr("Round trip %1 ms. End-to-end encrypted.").arg(home.rttMs)
                                                 : qsTr("Round trip under 1 ms. End-to-end encrypted."))
                              : home.lastSeen > 0
                                ? qsTr("Last seen %1. They reconnect on their own when both are on the same network.")
                                      .arg(page.relativeTime(home.lastSeen))
                                : qsTr("Open Nectarlink on %1. They connect on their own when both are on the same network.")
                                      .arg(home.name)
                    }
                }
            }
            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 12
                    Txt { text: qsTr("This PC"); role: "label"; muted: true }
                    Txt { width: parent.width; text: AppController.deviceName; role: "title" }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        text: qsTr("Paired with %1 since %2")
                              .arg(home.name)
                              .arg(new Date(home.pairedAt * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat))
                        wrapMode: Text.WordWrap
                    }
                    Button {
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.plus
                        text: qsTr("Pair another device")
                        onClicked: page.pairRequested()
                    }
                }
            }
        }
    }

    // A quick action. Unavailable actions are dimmed and say what unlocks them.
    component ActionTile: Rectangle {
        id: tile
        property string title
        property string subtitle
        property string iconPath
        property var feature: ({})
        property bool active: false
        // False until this PC implements the action (shown as coming soon).
        property bool ready: true
        signal clicked

        readonly property bool available: feature.state === "available" && ready

        height: 108
        radius: Theme.radiusLg
        color: tile.active ? Theme.primary : Theme.tileColor
        border.width: Theme.graphite ? 1 : 0
        border.color: Theme.outlineVariant
        scale: tap.pressed && available ? Theme.pressScale : 1
        Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
        transform: Translate {
            y: hover.hovered && tile.available && !tap.pressed ? -Theme.hoverLift : 0
            Behavior on y { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        }

        Accessible.role: Accessible.Button
        Accessible.name: title
        Accessible.description: available ? subtitle : lock.label
        Accessible.onPressAction: if (tile.available) tile.clicked()

        readonly property color ink: tile.active ? Theme.primaryContent : Theme.surfaceContent
        Column {
            anchors.fill: parent
            anchors.margins: 14
            spacing: 12
            Rectangle {
                width: 36; height: 36
                radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
                color: tile.active ? Qt.rgba(1, 1, 1, 0.18) : (Theme.graphite ? "transparent" : Theme.secondaryContainer)
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                opacity: tile.available ? 1 : 0.55
                Icon {
                    anchors.centerIn: parent
                    path: tile.iconPath
                    color: tile.active ? Theme.primaryContent : Theme.secondaryContainerContent
                }
            }
            Column {
                width: parent.width
                spacing: 3
                Txt { width: parent.width; text: tile.title; role: "title"; size: 14; color: tile.ink; opacity: tile.available ? 1 : 0.6 }
                Txt { width: parent.width; visible: tile.available; text: tile.subtitle; role: "bodySmall"; color: tile.ink; opacity: 0.75 }
                // Not built yet, or capabilities unknown until the device has
                // connected once.
                Txt {
                    width: parent.width
                    visible: !tile.available && !lock.visible
                    text: tile.ready ? qsTr("When connected") : qsTr("Coming soon")
                    role: "bodySmall"
                    muted: true
                }
                LockChip { id: lock; visible: tile.ready && !tile.available && label.length > 0; feature: tile.feature }
            }
        }
        HoverHandler { id: hover; cursorShape: tile.available ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { id: tap; enabled: tile.available; onTapped: tile.clicked() }
    }
}
