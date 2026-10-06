// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Home: the paired phone at a glance (status, battery, power level) and
// quick actions. Actions reflect the capability matrix: what isn't
// available shows what unlocks it.
Item {
    id: page
    property bool active: true
    property int current: 0
    // The device shown, for dropped files.
    property string currentDeviceId
    property string currentDeviceName
    property bool currentCanReceive: false
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

    // Files dragged onto the page are sent to the device on screen.
    DropArea {
        id: drop
        anchors.fill: parent
        enabled: page.currentCanReceive
        onEntered: (drag) => drag.accepted = drag.hasUrls
        onDropped: (drag) => {
            if (drag.hasUrls) {
                TransferList.send(page.currentDeviceId, drag.urls.map(url => url.toString()))
                drag.accept(Qt.CopyAction)
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: Theme.contentPadding / 2
        radius: Theme.radiusXl
        visible: opacity > 0
        opacity: drop.containsDrag ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        color: Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b, 0.92)
        border.width: 2
        border.color: Theme.primary
        Column {
            anchors.centerIn: parent
            spacing: 12
            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 40; height: 40
                stroke: 1.6
                path: Icons.send
                color: Theme.primaryContainerContent
            }
            Txt {
                anchors.horizontalCenter: parent.horizontalCenter
                role: "headline"
                color: Theme.primaryContainerContent
                text: qsTr("Drop to send to %1").arg(page.currentDeviceName)
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

        // Dropped files go to the device on screen (and Messages shows it).
        // Kept while the page is hidden.
        Binding { target: page; property: "currentDeviceId"; value: home.deviceId; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding { target: page; property: "currentDeviceName"; value: home.name; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding {
            target: page
            property: "currentCanReceive"
            value: home.feature("files.send").state === "available"
            when: home.visible
        }

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
                    subtitle: Preferences.autoClipboard ? qsTr("Syncs automatically") : qsTr("To the phone")
                    iconPath: Icons.clipboard
                    feature: home.feature("clipboard.pc_to_phone")
                    onClicked: AppController.sendClipboard(home.deviceId)
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Mirror screen")
                    subtitle: qsTr("See it on this PC")
                    iconPath: Icons.mirror
                    feature: home.feature("mirroring.view")
                    active: Mirror.device === home.deviceId && Mirror.phase !== "" && Mirror.phase !== "ended"
                    onClicked: Mirror.start(home.deviceId)
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Send files")
                    subtitle: qsTr("Or drop them here")
                    iconPath: Icons.send
                    feature: home.feature("files.send")
                    onClicked: filePicker.open()
                    sideIcon: Icons.folder
                    sideLabel: qsTr("Send a folder")
                    onSideClicked: folderPicker.open()
                }
            }
            FileDialog {
                id: filePicker
                title: qsTr("Send to %1").arg(home.name)
                fileMode: FileDialog.OpenFiles
                onAccepted: TransferList.send(home.deviceId, selectedFiles.map(url => url.toString()))
            }
            FolderDialog {
                id: folderPicker
                title: qsTr("Send a folder to %1").arg(home.name)
                onAccepted: TransferList.send(home.deviceId, [selectedFolder.toString()])
            }

            // ---- Transfers ----
            Card {
                width: parent.width
                visible: TransferList.count > 0
                Column {
                    width: parent.width
                    spacing: 4
                    Item {
                        width: parent.width
                        height: Math.max(transfersHeader.height, clearTransfers.height)
                        Txt {
                            id: transfersHeader
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: qsTr("Transfers")
                        }
                        Button {
                            id: clearTransfers
                            anchors.right: parent.right
                            visible: TransferList.count > TransferList.active
                            variant: "text"
                            size: "sm"
                            text: qsTr("Clear finished")
                            onClicked: TransferList.clearFinished()
                        }
                    }
                    ListView {
                        id: transferList
                        width: parent.width
                        height: contentHeight
                        interactive: false
                        model: TransferList
                        delegate: TransferItem { width: transferList.width }
                        add: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 200 }
                        }
                        displaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 220; easing.type: Easing.OutCubic }
                        }
                    }
                }
            }

            // ---- Notifications ----
            Card {
                id: feed
                width: parent.width
                readonly property var state: home.feature("notifications.mirror")
                readonly property int count: NotificationList.count >= 0 ? NotificationList.countFor(home.deviceId) : 0

                Column {
                    width: parent.width
                    spacing: 8
                    Item {
                        width: parent.width
                        height: Math.max(header.height, clearAll.height)
                        Txt {
                            id: header
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: feed.count > 0 ? qsTr("Notifications · %1").arg(feed.count) : qsTr("Notifications")
                        }
                        Button {
                            anchors.right: clearAll.visible ? clearAll.left : parent.right
                            anchors.rightMargin: clearAll.visible ? 8 : 0
                            anchors.verticalCenter: parent.verticalCenter
                            visible: NotificationList.historyEnabled && NotificationHistory.count > 0
                            variant: "text"
                            size: "sm"
                            iconPath: Icons.history
                            text: qsTr("History")
                            onClicked: {
                                historySearch.text = ""
                                historySheet.open()
                            }
                        }
                        Button {
                            id: clearAll
                            anchors.right: parent.right
                            visible: feed.count > 0
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.close
                            text: qsTr("Clear all")
                            onClicked: NotificationList.dismissAll(home.deviceId)
                        }
                    }
                    // Ticks every minute so "5 min" ages without new events.
                    Timer {
                        id: feedClock
                        property int minutes: 0
                        interval: 60000
                        repeat: true
                        running: feed.count > 0
                        onTriggered: minutes++
                    }
                    // The page scrolls; this list only lays out and animates:
                    // new notifications fade and slide in, dismissed ones fade
                    // out, and the rest glide into place.
                    ListView {
                        id: list
                        width: parent.width
                        height: contentHeight
                        interactive: false
                        model: NotificationList
                        delegate: NotificationItem {
                            width: list.width
                            clock: feedClock.minutes
                            visible: deviceId === home.deviceId
                            height: visible ? implicitHeight : 0
                            onOptionsRequested: appSheet.openFor(app, appName)
                        }
                        add: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 220; easing.type: Easing.OutCubic }
                            NumberAnimation {
                                property: "y"
                                from: ViewTransition.destination.y - 10
                                to: ViewTransition.destination.y
                                duration: 260
                                easing.type: Easing.OutCubic
                            }
                        }
                        addDisplaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 240; easing.type: Easing.OutCubic }
                        }
                        remove: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; to: 0; duration: 160 }
                        }
                        removeDisplaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 240; easing.type: Easing.OutCubic }
                        }
                        Behavior on height {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { duration: 240; easing.type: Easing.OutCubic }
                        }
                    }
                    // Empty: say why, and what turns it on.
                    Txt {
                        width: parent.width
                        visible: feed.count === 0 && !lockedHint.visible
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: feed.state.state === "available"
                              ? qsTr("Notifications from %1 show up here and as Windows notifications.").arg(home.name)
                              : home.lastSeen > 0 || home.online
                                ? qsTr("Notifications from %1 aren't available yet.").arg(home.name)
                                : qsTr("Notifications show up here once %1 has connected.").arg(home.name)
                    }
                    LockChip { id: lockedHint; visible: feed.count === 0 && label.length > 0; feature: feed.state }
                    // Windows hides every app's notifications when they're
                    // turned off in Settings; say so instead of staying quiet.
                    Rectangle {
                        width: parent.width
                        height: toastsOff.height + 20
                        visible: !AppController.toastsEnabled && feed.state.state === "available"
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: Theme.graphite ? 1 : 0
                        border.color: Theme.outlineVariant
                        Row {
                            id: toastsOff
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 24
                            spacing: 12
                            Icon { anchors.verticalCenter: parent.verticalCenter; path: Icons.bell; color: Theme.surfaceContentVariant }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 32 - turnOn.width - 24
                                role: "bodySmall"
                                wrapMode: Text.WordWrap
                                text: qsTr("Windows notifications are turned off, so these only show up here.")
                            }
                            Button {
                                id: turnOn
                                anchors.verticalCenter: parent.verticalCenter
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Turn on")
                                onClicked: Qt.openUrlExternally("ms-settings:notifications")
                            }
                        }
                    }
                }
            }
        }

        // ---- Details ----
        Column {
            width: home.sideWidth
            spacing: Theme.gutter

            // The call in progress on the phone.
            Binding { target: PhoneCall; property: "device"; value: home.deviceId; when: home.visible; restoreMode: Binding.RestoreNone }
            CallCard {
                width: home.sideWidth
                visible: PhoneCall.active && PhoneCall.device === home.deviceId
            }

            // What plays on the phone (its first player).
            Repeater {
                model: MediaList
                delegate: NowPlaying {
                    required property int index
                    width: home.sideWidth
                    visible: deviceId === home.deviceId
                             && MediaList.revision >= 0 && index === MediaList.firstFor(home.deviceId)
                }
            }

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
                    Button {
                        visible: !home.online
                        variant: "tonal"
                        size: "sm"
                        text: qsTr("Check the connection")
                        onClicked: AppController.runDoctor()
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

    // What one app's notifications do on this PC.
    Sheet {
        id: appSheet
        property string app
        property string appName
        property string rule: "show"
        cardWidth: 420
        function openFor(app, name) {
            appSheet.app = app
            appSheet.appName = name
            appSheet.rule = NotificationList.appRule(app)
            open()
        }
        Column {
            width: parent.width
            spacing: 16
            Txt { width: parent.width; text: qsTr("%1 notifications").arg(appSheet.appName); role: "headline"; wrapMode: Text.WordWrap }
            Segmented {
                options: [
                    { value: "show", label: qsTr("Show") },
                    { value: "quiet", label: qsTr("No pop-ups") },
                    { value: "hidden", label: qsTr("Hide") }
                ]
                value: appSheet.rule
                onPicked: (value) => {
                    appSheet.rule = value
                    NotificationList.setAppRule(appSheet.app, value)
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: appSheet.rule === "hidden"
                      ? qsTr("Nothing from %1 shows on this PC. It still shows on your phone.").arg(appSheet.appName)
                      : appSheet.rule === "quiet"
                        ? qsTr("Notifications from %1 show here in Nectarlink, without Windows pop-ups or sounds.").arg(appSheet.appName)
                        : qsTr("Notifications from %1 show here and as Windows notifications.").arg(appSheet.appName)
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: qsTr("You can change this for every app in Settings.")
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: appSheet.close() }
            }
        }
    }

    // Notifications from the last day that are gone from the phone.
    Sheet {
        id: historySheet
        cardWidth: 560
        Column {
            width: parent.width
            spacing: 12
            Item {
                width: parent.width
                height: Math.max(historyTitle.height, clearHistory.height)
                Txt { id: historyTitle; anchors.verticalCenter: parent.verticalCenter; text: qsTr("History"); role: "headline" }
                Button {
                    id: clearHistory
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    visible: NotificationHistory.count > 0
                    variant: "text"
                    size: "sm"
                    text: qsTr("Clear history")
                    onClicked: NotificationList.clearHistory()
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: NotificationHistory.count > 0
                      ? qsTr("Notifications that went away on your phone, or that you dismissed, for a day.")
                      : qsTr("Nothing here yet. Notifications that go away on your phone are kept here for a day.")
            }
            // Search: by app, title or text, ignoring case.
            Rectangle {
                width: parent.width
                height: 40
                visible: NotificationHistory.count > 0
                radius: Theme.graphite ? Theme.radiusSm : height / 2
                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                border.width: historySearch.activeFocus ? 2 : 1
                border.color: historySearch.activeFocus ? Theme.primary : Theme.outlineVariant
                Icon {
                    id: searchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18; height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: historySearch
                    readonly property string query: text.trim().toLocaleLowerCase()
                    function matches(fields) {
                        return query.length === 0 || fields.join(" ").toLocaleLowerCase().indexOf(query) >= 0
                    }
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 10
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    clip: true
                    Keys.onEscapePressed: (event) => {
                        // Clears first; a second Escape closes the sheet.
                        event.accepted = text.length > 0
                        text = ""
                    }
                    Accessible.name: qsTr("Search history")
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: historySearch.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Search history")
                    }
                }
            }
            Flickable {
                width: parent.width
                height: Math.min(historyColumn.height, page.height - 260)
                contentHeight: historyColumn.height
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: historyColumn
                    width: parent.width
                    Repeater {
                        model: NotificationHistory
                        delegate: HistoryItem {
                            width: historyColumn.width
                            visible: historySearch.matches([appName, title, text, sub])
                        }
                    }
                }
            }
            Txt {
                width: parent.width
                visible: historySearch.query.length > 0 && historyColumn.height === 0
                horizontalAlignment: Text.AlignHCenter
                role: "body"
                muted: true
                text: qsTr("Nothing matches “%1”.").arg(historySearch.text.trim())
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: historySheet.close() }
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
        // A second, smaller action in the corner (e.g. "Send a folder").
        property string sideIcon
        property string sideLabel
        signal clicked
        signal sideClicked

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
        IconButton {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 8
            visible: tile.sideIcon.length > 0 && tile.available
            iconPath: tile.sideIcon
            label: tile.sideLabel
            iconColor: tile.ink
            onClicked: tile.sideClicked()
        }
    }
}
