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
    property int current: AppController.currentDevice
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

    onCurrentChanged: {
        if (current >= DeviceList.count) current = 0
        if (AppController.currentDevice !== current)
            AppController.currentDevice = current
    }
    Connections {
        target: DeviceList
        function onCountChanged() {
            if (page.current >= DeviceList.count)
                page.current = Math.max(0, DeviceList.count - 1)
        }
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
                    sideIcon: Preferences.clipboardHistory ? Icons.history : ""
                    sideLabel: qsTr("Clipboard history")
                    sideAlwaysAvailable: true
                    onSideClicked: {
                        clipboardSearch.text = ""
                        clipboardSheet.open()
                    }
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Mirror screen")
                    subtitle: qsTr("See it on this PC")
                    iconPath: Icons.mirror
                    feature: home.feature("mirroring.view")
                    // The phone's screen is showing (its session is 0).
                    active: {
                        try {
                            return JSON.parse(Mirror.windows).some(w => w.device === home.deviceId && w.session === 0
                                                                       && w.phase !== "ended")
                        } catch (e) {
                            return false
                        }
                    }
                    onClicked: Mirror.start(home.deviceId)
                    sideIcon: home.feature("mirroring.app_windows").state === "available" ? Icons.apps : ""
                    sideLabel: qsTr("Open an app")
                    onSideClicked: appsSheet.openFor(home.deviceId, home.name)
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

            // Active webcam session on this phone.
            Card {
                id: webcamHomeCard
                width: parent.width
                readonly property var webcamFeature: home.feature("camera.webcam")
                readonly property bool isThisPhone: (Webcam.phase === "streaming" || Webcam.phase === "asking")
                                                    && Webcam.activeDevice === home.deviceId
                visible: isThisPhone
                Column {
                    width: parent.width
                    spacing: 10
                    Row {
                        spacing: 8
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            path: Icons.video
                            color: Theme.primary
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Webcam")
                            role: "label"
                        }
                    }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: Webcam.statusText.length > 0
                              ? Webcam.statusText
                              : qsTr("Using %1's camera on this PC.").arg(home.name)
                    }
                    Button {
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.close
                        text: qsTr("Stop webcam")
                        onClicked: Webcam.stop()
                    }
                }
            }

            // Quick settings on the phone (toggles, plus expandable ringer/volume/brightness).
            Card {
                id: phoneControls
                width: parent.width
                visible: home.online && state !== null
                property bool expanded: false
                property bool confirmWifiOff: false
                property var hoveredLockedFeature: null
                readonly property var state: {
                    if (AppController.phoneTogglesRevision < 0)
                        return null
                    const raw = AppController.phoneToggles(home.deviceId)
                    if (!raw || raw.length === 0)
                        return null
                    try {
                        return JSON.parse(raw)
                    } catch (e) {
                        return null
                    }
                }
                readonly property bool hasFlashlight: state !== null
                    && state.flashlight !== undefined && state.flashlight !== null
                readonly property var dndFeature: home.feature("toggles.dnd")
                readonly property var flashlightFeature: home.feature("toggles.flashlight")
                readonly property var ringerFeature: home.feature("toggles.ringer")
                readonly property var volumeFeature: home.feature("toggles.volume")
                readonly property var brightnessFeature: home.feature("toggles.brightness")
                readonly property var wifiFeature: home.feature("toggles.wifi")
                readonly property var bluetoothFeature: home.feature("toggles.bluetooth")
                readonly property var webcamFeature: home.feature("camera.webcam")
                readonly property var activeLockFeature: {
                    if (hoveredLockedFeature && hoveredLockedFeature.state === "locked")
                        return hoveredLockedFeature
                    return ({})
                }
                onVisibleChanged: if (!visible) confirmWifiOff = false

                Column {
                    width: parent.width
                    spacing: 12

                    Item {
                        width: parent.width
                        height: 24
                        Txt {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Phone controls")
                            role: "label"
                            muted: true
                        }
                        Button {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "text"
                            size: "sm"
                            text: phoneControls.expanded ? qsTr("Less") : qsTr("Sound & display")
                            onClicked: phoneControls.expanded = !phoneControls.expanded
                        }
                    }

                    Row {
                        id: toggleRow
                        width: parent.width
                        spacing: 6
                        readonly property int buttonCount: phoneControls.hasFlashlight ? 4 : 3
                        readonly property real buttonWidth: (width - spacing * (buttonCount - 1)) / buttonCount

                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.moon
                            shortTitle: qsTr("DND")
                            label: qsTr("Do Not Disturb")
                            active: phoneControls.state ? Boolean(phoneControls.state.dnd) : false
                            feature: phoneControls.dndFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: AppController.setPhoneToggle(home.deviceId, "dnd", String(!active))
                        }
                        QuickToggleButton {
                            visible: phoneControls.hasFlashlight
                            width: toggleRow.buttonWidth
                            iconPath: Icons.flashlight
                            shortTitle: qsTr("Torch")
                            label: qsTr("Flashlight")
                            active: phoneControls.state !== null && phoneControls.state.flashlight === true
                            feature: phoneControls.flashlightFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: AppController.setPhoneToggle(home.deviceId, "flashlight", String(!active))
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.wifi
                            shortTitle: qsTr("Wi‑Fi")
                            label: qsTr("Wi‑Fi")
                            active: phoneControls.state ? Boolean(phoneControls.state.wifi) : false
                            feature: phoneControls.wifiFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: {
                                if (active)
                                    phoneControls.confirmWifiOff = true
                                else
                                    AppController.setPhoneToggle(home.deviceId, "wifi", "true")
                            }
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.bluetooth
                            shortTitle: qsTr("BT")
                            label: qsTr("Bluetooth")
                            active: phoneControls.state ? Boolean(phoneControls.state.bluetooth) : false
                            feature: phoneControls.bluetoothFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: AppController.setPhoneToggle(home.deviceId, "bluetooth", String(!active))
                        }
                    }

                    LockChip {
                        visible: label.length > 0
                        feature: phoneControls.activeLockFeature
                        maxWidth: parent.width
                    }

                    // Confirm before turning Wi-Fi off, since that can sever the link.
                    Rectangle {
                        width: parent.width
                        height: wifiConfirmCol.height + 20
                        visible: phoneControls.confirmWifiOff && phoneControls.state !== null && Boolean(phoneControls.state.wifi)
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: 1
                        border.color: Theme.outlineVariant
                        Column {
                            id: wifiConfirmCol
                            x: 12
                            y: 10
                            width: parent.width - 24
                            spacing: 8
                            Txt {
                                width: parent.width
                                role: "bodySmall"
                                wrapMode: Text.WordWrap
                                text: qsTr("Turning Wi‑Fi off on %1 will disconnect it if Wi‑Fi is how it reaches this PC.")
                                      .arg(home.name)
                            }
                            Row {
                                anchors.right: parent.right
                                spacing: 8
                                Button {
                                    variant: "text"
                                    size: "sm"
                                    text: qsTr("Cancel")
                                    onClicked: phoneControls.confirmWifiOff = false
                                }
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    text: qsTr("Turn off")
                                    onClicked: {
                                        phoneControls.confirmWifiOff = false
                                        AppController.setPhoneToggle(home.deviceId, "wifi", "false")
                                    }
                                }
                            }
                        }
                    }

                    Column {
                        width: parent.width
                        spacing: 12
                        visible: phoneControls.expanded

                        // Ringer mode segmented control.
                        Column {
                            width: parent.width
                            spacing: 6
                            Segmented {
                                width: parent.width
                                enabled: phoneControls.ringerFeature.state === "available"
                                         || phoneControls.ringerFeature.state === "partial"
                                opacity: enabled ? 1 : 0.5
                                options: [
                                    { value: "ring", label: qsTr("Ring") },
                                    { value: "vibrate", label: qsTr("Vibrate") },
                                    { value: "silent", label: qsTr("Silent") }
                                ]
                                value: phoneControls.state ? phoneControls.state.ringer : "ring"
                                onPicked: (mode) => AppController.setPhoneToggle(home.deviceId, "ringer", mode)
                            }
                            LockChip {
                                visible: phoneControls.ringerFeature.state === "locked" && label.length > 0
                                feature: phoneControls.ringerFeature
                                maxWidth: parent.width
                            }
                        }

                        // Media volume slider.
                        ToggleSlider {
                            width: parent.width
                            deviceId: home.deviceId
                            toggleId: "volume"
                            title: qsTr("Volume")
                            iconPath: shownValue === 0 ? Icons.soundOff : Icons.speaker
                            phoneValue: phoneControls.state ? phoneControls.state.volume : 0
                            feature: phoneControls.volumeFeature
                        }

                        // Screen brightness slider.
                        ToggleSlider {
                            width: parent.width
                            deviceId: home.deviceId
                            toggleId: "brightness"
                            title: qsTr("Brightness")
                            iconPath: Icons.brightness
                            phoneValue: phoneControls.state ? phoneControls.state.brightness : 0
                            feature: phoneControls.brightnessFeature
                        }
                    }
                }
            }

            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 10
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
                              ? (home.rttMs >= 1 ? qsTr("Round trip %1 ms · End-to-end encrypted").arg(home.rttMs)
                                                 : qsTr("Round trip under 1 ms · End-to-end encrypted"))
                              : home.lastSeen > 0
                                ? qsTr("Last seen %1. Reconnects automatically on the same network.")
                                      .arg(page.relativeTime(home.lastSeen))
                                : qsTr("Open Nectarlink on %1 to connect on this network.")
                                      .arg(home.name)
                    }
                    Button {
                        visible: !home.online
                        variant: "tonal"
                        size: "sm"
                        text: qsTr("Check the connection")
                        onClicked: AppController.runDoctor()
                    }
                    Divider {
                        width: parent.width
                        visible: home.online && !webcamHomeCard.isThisPhone
                                 && (webcamHomeCard.webcamFeature.state === "available"
                                     || webcamHomeCard.webcamFeature.action === "enableAddon")
                    }
                    Item {
                        width: parent.width
                        height: 32
                        visible: home.online && !webcamHomeCard.isThisPhone
                                 && (webcamHomeCard.webcamFeature.state === "available"
                                     || webcamHomeCard.webcamFeature.action === "enableAddon")
                        Row {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 16; height: 16
                                path: Icons.video
                                color: Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                text: qsTr("Phone as webcam")
                            }
                        }
                        Button {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            size: "sm"
                            text: qsTr("Start")
                            onClicked: Webcam.start(home.deviceId)
                        }
                    }
                    Divider { width: parent.width }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        text: qsTr("This PC (%1) · Paired %2")
                              .arg(AppController.deviceName)
                              .arg(new Date(home.pairedAt * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat))
                        wrapMode: Text.WordWrap
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
    AppsSheet {
        id: appsSheet
    }

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

    Sheet {
        id: clipboardSheet
        cardWidth: 560
        readonly property var items: {
            try {
                return JSON.parse(AppController.clipboardHistory)
            } catch (e) {
                return []
            }
        }
        Column {
            width: parent.width
            spacing: 12
            Item {
                width: parent.width
                height: Math.max(clipboardTitle.height, clearClipboard.height)
                Txt { id: clipboardTitle; anchors.verticalCenter: parent.verticalCenter; text: qsTr("Clipboard history"); role: "headline" }
                Button {
                    id: clearClipboard
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    visible: clipboardSheet.items.length > 0
                    variant: "text"
                    size: "sm"
                    text: qsTr("Clear history")
                    onClicked: AppController.clearClipboardHistory()
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: clipboardSheet.items.length > 0
                      ? qsTr("Your last 50 clips with your phone, kept encrypted on this PC.")
                      : qsTr("Nothing here yet. Text and images copied between this PC and your phone show up here.")
            }
            Rectangle {
                width: parent.width
                height: 40
                visible: clipboardSheet.items.length > 0
                radius: Theme.graphite ? Theme.radiusSm : height / 2
                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                border.width: clipboardSearch.activeFocus ? 2 : 1
                border.color: clipboardSearch.activeFocus ? Theme.primary : Theme.outlineVariant
                Icon {
                    id: clipSearchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18; height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: clipboardSearch
                    readonly property string query: text.trim().toLocaleLowerCase()
                    function matches(fields) {
                        return query.length === 0 || fields.join(" ").toLocaleLowerCase().indexOf(query) >= 0
                    }
                    anchors.left: clipSearchIcon.right
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
                        event.accepted = text.length > 0
                        text = ""
                    }
                    Accessible.name: qsTr("Search clipboard history")
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: clipboardSearch.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Search clipboard history")
                    }
                }
            }
            Flickable {
                width: parent.width
                height: Math.min(clipboardColumn.height, page.height - 260)
                contentHeight: clipboardColumn.height
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: clipboardColumn
                    width: parent.width
                    Repeater {
                        model: clipboardSheet.items
                        delegate: Item {
                            id: clipRow
                            required property var modelData
                            required property int index
                            readonly property string sourceLabel: modelData.incoming
                                ? (modelData.deviceName ? qsTr("From %1").arg(modelData.deviceName) : qsTr("From phone"))
                                : (modelData.deviceName ? qsTr("Sent to %1").arg(modelData.deviceName) : qsTr("From this PC"))
                            readonly property string timeLabel: page.relativeTime(modelData.timestamp || 0)
                            width: clipboardColumn.width
                            visible: clipboardSearch.matches([
                                modelData.text || "",
                                sourceLabel,
                                modelData.kind === "image" ? qsTr("Image") : ""
                            ])
                            height: visible ? Math.max(clipIconBox.height, clipContent.height, clipActions.height) + 20 : 0

                            Divider {
                                width: parent.width
                                visible: clipRow.index > 0
                            }

                            Item {
                                id: clipIconBox
                                x: 4; y: 12
                                width: 28; height: 28
                                Icon {
                                    anchors.centerIn: parent
                                    width: 18; height: 18
                                    path: clipRow.modelData.pinned
                                          ? Icons.pin
                                          : (clipRow.modelData.incoming ? Icons.phone : Icons.laptop)
                                    color: clipRow.modelData.pinned ? Theme.primary : Theme.surfaceContentVariant
                                }
                            }

                            Column {
                                id: clipContent
                                anchors.left: clipIconBox.right
                                anchors.leftMargin: 12
                                anchors.right: clipActions.left
                                anchors.rightMargin: 8
                                y: 10
                                spacing: 4
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    text: [
                                        clipRow.sourceLabel,
                                        clipRow.timeLabel,
                                        clipRow.modelData.pinned ? qsTr("Pinned") : ""
                                    ].filter(s => s.length > 0).join(" · ")
                                    elide: Text.ElideRight
                                }
                                Txt {
                                    width: parent.width
                                    visible: clipRow.modelData.kind === "text" && Boolean(clipRow.modelData.text)
                                    role: "bodySmall"
                                    text: clipRow.modelData.text || ""
                                    wrapMode: Text.Wrap
                                    maximumLineCount: 3
                                    elide: Text.ElideRight
                                }
                                Rectangle {
                                    visible: clipRow.modelData.kind === "image"
                                    width: Math.min(parent.width, 180)
                                    height: 88
                                    radius: Theme.radiusSm
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant
                                    clip: true
                                    Image {
                                        anchors.fill: parent
                                        anchors.margins: 4
                                        source: clipRow.modelData.imageDataUrl || ""
                                        fillMode: Image.PreserveAspectFit
                                        asynchronous: true
                                        mipmap: true
                                    }
                                }
                            }

                            Row {
                                id: clipActions
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 4
                                IconButton {
                                    iconPath: Icons.copy
                                    label: qsTr("Copy")
                                    onClicked: AppController.copyClipboardHistory(clipRow.modelData.id)
                                }
                                IconButton {
                                    iconPath: Icons.pin
                                    iconColor: clipRow.modelData.pinned ? Theme.primary : Theme.surfaceContentVariant
                                    label: clipRow.modelData.pinned ? qsTr("Unpin") : qsTr("Pin")
                                    onClicked: AppController.pinClipboardHistory(clipRow.modelData.id, !clipRow.modelData.pinned)
                                }
                                IconButton {
                                    iconPath: Icons.trash
                                    label: qsTr("Delete")
                                    onClicked: AppController.deleteClipboardHistory(clipRow.modelData.id)
                                }
                            }
                        }
                    }
                }
            }
            Txt {
                width: parent.width
                visible: clipboardSearch.query.length > 0 && clipboardColumn.height === 0
                horizontalAlignment: Text.AlignHCenter
                role: "body"
                muted: true
                text: qsTr("Nothing matches “%1”.").arg(clipboardSearch.text.trim())
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: clipboardSheet.close() }
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
        property bool sideAlwaysAvailable: false
        signal clicked
        signal sideClicked

        readonly property bool available: feature.state === "available" && ready

        height: 108
        activeFocusOnTab: available
        radius: Theme.radiusLg
        color: tile.active ? Theme.primary : Theme.tileColor
        border.width: Theme.focusVisible(tile) ? 2 : (Theme.graphite ? 1 : 0)
        border.color: Theme.focusVisible(tile) ? Theme.primary : Theme.outlineVariant
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
        Keys.onPressed: (event) => {
            if (tile.available && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                tile.clicked()
                event.accepted = true
            }
        }

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
            visible: tile.sideIcon.length > 0 && (tile.available || tile.sideAlwaysAvailable)
            iconPath: tile.sideIcon
            label: tile.sideLabel
            iconColor: tile.ink
            onClicked: tile.sideClicked()
        }
    }

    // Compact icon button for a phone quick toggle (DND, Flashlight, Wi-Fi, Bluetooth).
    component QuickToggleButton: FocusScope {
        id: qbtn
        property string iconPath
        property string shortTitle
        property string label
        property bool active: false
        property var feature: ({})
        readonly property bool available: feature.state === "available"
        readonly property alias hovered: qhover.hovered
        signal clicked

        height: 52
        activeFocusOnTab: available

        Accessible.role: Accessible.CheckBox
        Accessible.name: label
        Accessible.checked: active
        Accessible.onPressAction: if (qbtn.available) qbtn.clicked()
        Keys.onPressed: (event) => {
            if (qbtn.available && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                qbtn.clicked()
                event.accepted = true
            }
        }

        readonly property color fillColor: {
            if (active && available)
                return Theme.graphite ? Theme.surfaceContent : Theme.primary
            if (active && !available)
                return Theme.graphite ? Theme.surfaceContainerHigh : Theme.secondaryContainer
            return Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
        }
        readonly property color contentColor: {
            if (active && available)
                return Theme.graphite ? Theme.surface : Theme.primaryContent
            if (active && !available)
                return Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
            return available ? Theme.surfaceContent : Theme.surfaceContentVariant
        }

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusMd
            color: qbtn.fillColor
            border.width: Theme.focusVisible(qbtn) ? 2 : (Theme.graphite || (qbtn.active && !qbtn.available) ? 1 : 0)
            border.color: Theme.focusVisible(qbtn) ? Theme.primary : Theme.outlineVariant
            opacity: qbtn.available || qbtn.active ? 1 : 0.55
            scale: qtap.pressed && qbtn.available ? Theme.pressScale : 1
            Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
            Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

            Column {
                anchors.centerIn: parent
                width: parent.width - 4
                spacing: 3
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 17; height: 17
                    path: qbtn.iconPath
                    color: qbtn.contentColor
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    role: "caption"
                    size: Theme.graphite ? 9 : 11
                    font.letterSpacing: Theme.graphite ? 0 : (graphiteLabel ? Theme.labelStyle.tracking : spec.tracking) * size
                    color: qbtn.contentColor
                    text: qbtn.shortTitle
                    elide: Text.ElideRight
                }
            }

            Icon {
                visible: !qbtn.available
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 4
                width: 10; height: 10
                path: Icons.lock
                color: qbtn.contentColor
                opacity: 0.7
            }
        }

        HoverHandler { id: qhover; cursorShape: qbtn.available ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { id: qtap; enabled: qbtn.available; onTapped: qbtn.clicked() }
    }

    // Debounced 0..=100 slider for a phone setting (media volume, screen brightness).
    component ToggleSlider: Column {
        id: slider
        property string deviceId
        property string toggleId
        property string title
        property string iconPath
        property int phoneValue: 0
        property var feature: ({})
        readonly property bool available: feature.state === "available"
        property int dragValue: -1
        readonly property int shownValue: dragValue >= 0 ? dragValue : Math.max(0, Math.min(100, phoneValue))

        spacing: 4

        Timer {
            id: sendDebounce
            interval: 100
            onTriggered: if (slider.dragValue >= 0) {
                AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(slider.dragValue))
            }
        }
        Timer {
            id: settle
            interval: 400
            onTriggered: if (!dragArea.pressed) slider.dragValue = -1
        }

        Item {
            width: parent.width
            height: 20
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 15; height: 15
                    path: slider.iconPath
                    color: slider.available ? Theme.surfaceContent : Theme.surfaceContentVariant
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    role: "bodySmall"
                    muted: !slider.available
                    text: slider.title
                }
            }
            Txt {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                role: "caption"
                muted: true
                text: qsTr("%1%").arg(slider.shownValue)
            }
        }

        FocusScope {
            id: barScope
            width: parent.width
            height: 18
            activeFocusOnTab: slider.available
            Accessible.role: Accessible.Slider
            Accessible.name: slider.title
            Accessible.description: qsTr("%1%").arg(slider.shownValue)
            Accessible.onPressAction: if (slider.available) {
                const next = slider.shownValue >= 80 ? 25 : slider.shownValue + 20
                slider.dragValue = next
                settle.restart()
                AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
            }
            Keys.onPressed: (event) => {
                if (!slider.available) return
                if (event.key === Qt.Key_Left || event.key === Qt.Key_Down) {
                    const next = Math.max(0, slider.shownValue - 5)
                    slider.dragValue = next
                    settle.restart()
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
                    event.accepted = true
                } else if (event.key === Qt.Key_Right || event.key === Qt.Key_Up) {
                    const next = Math.min(100, slider.shownValue + 5)
                    slider.dragValue = next
                    settle.restart()
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
                    event.accepted = true
                }
            }

            readonly property real fraction: slider.shownValue / 100.0

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 6
                radius: 3
                color: Theme.surfaceContainerHighest
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                opacity: slider.available ? 1 : 0.45

                Rectangle {
                    width: parent.width * barScope.fraction
                    height: parent.height
                    radius: parent.radius
                    color: slider.available
                           ? (Theme.graphite ? Theme.surfaceContent : Theme.primary)
                           : Theme.surfaceContentVariant
                }
            }

            Rectangle {
                visible: slider.available
                x: Math.max(0, Math.min(barScope.width - width, barScope.width * barScope.fraction - width / 2))
                anchors.verticalCenter: parent.verticalCenter
                width: 14; height: 14; radius: 7
                color: Theme.graphite ? Theme.surfaceContent : Theme.primary
                border.width: Theme.focusVisible(barScope) ? 2 : 0
                border.color: Theme.surface
            }

            MouseArea {
                id: dragArea
                anchors.fill: parent
                anchors.margins: -4
                enabled: slider.available
                hoverEnabled: true
                cursorShape: slider.available ? Qt.PointingHandCursor : Qt.ArrowCursor
                function levelAt(x) {
                    return Math.round(Math.max(0, Math.min(1, (x - 4) / barScope.width)) * 100)
                }
                onPressed: (mouse) => {
                    settle.stop()
                    slider.dragValue = levelAt(mouse.x)
                    if (!sendDebounce.running) sendDebounce.start()
                }
                onPositionChanged: (mouse) => {
                    if (pressed) {
                        slider.dragValue = levelAt(mouse.x)
                        if (!sendDebounce.running) sendDebounce.start()
                    }
                }
                onReleased: (mouse) => {
                    sendDebounce.stop()
                    const finalVal = levelAt(mouse.x)
                    slider.dragValue = finalVal
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(finalVal))
                    settle.restart()
                }
                onCanceled: {
                    sendDebounce.stop()
                    slider.dragValue = -1
                }
            }
        }

        LockChip {
            visible: !slider.available && label.length > 0 && slider.feature.action !== "enableToggle"
            feature: slider.feature
            maxWidth: parent.width
        }
    }
}
