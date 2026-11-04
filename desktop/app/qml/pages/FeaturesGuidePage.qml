// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Layouts
import app.nectarlink

// In-app Features Guide page: searchable and filterable reference to every
// NectarLink feature, live capability status when a phone is connected,
// "Show me" navigation, and "Learn more" reader sheet with handbook tabs.
Item {
    id: guide

    property bool active: true
    property string deviceId: ""
    property string deviceName: ""
    property bool deviceOnline: false
    readonly property bool hasConnectedPhone: guide.deviceId !== "" && guide.deviceOnline

    signal showMeRequested(string targetId)
    signal restartWalkthroughRequested()

    property string searchQuery: ""
    property string selectedArea: "all"
    property string activeDocKey: "overview"

    property var deviceTogglesMap: ({
        notifications: true,
        clipboard: true,
        files: true,
        sms: true,
        calls: true,
        media: true,
        storage: false,
        remote_input: false,
        pc_mirror: false
    })

    function openDoc(key) {
        activeDocKey = key
        AppController.openGuideDoc(key)
    }

    function syncSelectedDevice() {
        let _caps = AppController.capsRevision
        let _toggles = AppController.togglesRevision
        let count = DeviceList.count
        if (count <= 0) {
            deviceOnline = false
            return
        }
        // A phone that was unpaired since isn't the one to show.
        let stillThere = false
        for (let i = 0; i < count; i++) {
            if (DeviceList.deviceIdAt(i) === guide.deviceId)
                stillThere = true
        }
        if (!stillThere)
            guide.deviceId = ""
        let targetId = guide.deviceId.length > 0 ? guide.deviceId : (DeviceList.deviceIdAt(0) || "")
        if (guide.deviceId.length === 0 && targetId.length > 0) {
            guide.deviceId = targetId
            guide.deviceName = DeviceList.deviceNameAt(0) || qsTr("Phone")
        }
        if (targetId !== "") {
            let st = AppController.featureState(targetId, "files")
            let reason = st ? (st.reason || "") : ""
            deviceOnline = reason.toLowerCase().indexOf("offline") === -1
            let list = AppController.deviceToggles(targetId)
            let next = {
                notifications: true,
                clipboard: true,
                files: true,
                sms: true,
                calls: true,
                media: true,
                storage: false,
                remote_input: false,
                pc_mirror: false
            }
            for (let i = 0; i < list.length; i++) {
                let entry = list[i]
                if (entry && entry.name !== undefined)
                    next[entry.name] = Boolean(entry.on)
            }
            deviceTogglesMap = next
        }
    }

    onDeviceIdChanged: syncSelectedDevice()
    Component.onCompleted: syncSelectedDevice()

    Connections {
        target: AppController
        function onHasDevicesChanged() { guide.syncSelectedDevice() }
        function onCapsRevisionChanged() { guide.syncSelectedDevice() }
        function onTogglesRevisionChanged() { guide.syncSelectedDevice() }
    }

    readonly property var guideChapters: [
        { key: "overview", label: qsTr("Overview") },
        { key: "setup", label: qsTr("Setup") },
        { key: "permissions", label: qsTr("Permissions") },
        { key: "elevated", label: qsTr("Power levels") },
        { key: "troubleshooting", label: qsTr("Troubleshooting") },
        { key: "faq", label: qsTr("FAQ") }
    ]

    readonly property var allFeatures: [
        {
            id: "notifications",
            name: qsTr("Notifications"),
            icon: Icons.bell,
            area: "notifications_calls",
            capKey: "notifications",
            toggleKey: "notifications",
            summary: qsTr("Mirror phone notifications to your PC with inline replies, action buttons, and dismissal sync."),
            whereLabel: qsTr("Home › Notifications"),
            needsLabel: qsTr("Notification Access on phone"),
            targetId: "home:notifications",
            docKey: "notifications"
        },
        {
            id: "messages",
            name: qsTr("Messages"),
            icon: Icons.messages,
            area: "messages",
            capKey: "sms",
            toggleKey: "sms",
            summary: qsTr("Read and send SMS and MMS threads, or reply to notification-backed chat conversations."),
            whereLabel: qsTr("Left rail › Messages"),
            needsLabel: qsTr("SMS and Contacts permissions on phone"),
            targetId: "messages:main",
            docKey: "messages"
        },
        {
            id: "calls",
            name: qsTr("Calls"),
            icon: Icons.call,
            area: "notifications_calls",
            capKey: "calls",
            toggleKey: "calls",
            summary: qsTr("Answer or decline incoming calls, browse call history and contacts, and dial from your PC."),
            whereLabel: qsTr("Left rail › Calls"),
            needsLabel: qsTr("Phone, Call Log, and Contacts permissions on phone"),
            targetId: "calls:main",
            docKey: "calls"
        },
        {
            id: "now_playing",
            name: qsTr("Now playing"),
            icon: Icons.music,
            area: "notifications_calls",
            capKey: "media_view",
            toggleKey: "media",
            summary: qsTr("See active track artwork and control playback between your phone and PC."),
            whereLabel: qsTr("Home › Media player"),
            needsLabel: qsTr("Notification Access on phone"),
            targetId: "deck:main",
            docKey: "deck"
        },
        {
            id: "clipboard",
            name: qsTr("Clipboard"),
            icon: Icons.clipboard,
            area: "clipboard_files",
            capKey: "clipboard_send",
            toggleKey: "clipboard",
            summary: qsTr("Sync copied text, HTML, images, and files with a 50-item local clipboard history."),
            whereLabel: qsTr("Home › Send clipboard"),
            needsLabel: qsTr("Clipboard toggle on Home; Elevated or quick tile for background phone copy"),
            targetId: "home:clipboard",
            docKey: "clipboard"
        },
        {
            id: "transfers",
            name: qsTr("Send files"),
            icon: Icons.send,
            area: "clipboard_files",
            capKey: "files",
            toggleKey: "files",
            summary: qsTr("Send files, folders, and links to your phone over your local Wi-Fi network."),
            whereLabel: qsTr("Home › Send files"),
            needsLabel: qsTr("Send files toggle on Home"),
            targetId: "home:transfers",
            docKey: "transfers"
        },
        {
            id: "send_queue",
            name: qsTr("Send queue"),
            icon: Icons.send,
            area: "clipboard_files",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            localAlways: true,
            summary: qsTr("Queue files and links while your phone is away so they send automatically on reconnect."),
            whereLabel: qsTr("Home › Send queue"),
            needsLabel: qsTr("Always available on PC"),
            targetId: "home:transfers",
            docKey: "transfers"
        },
        {
            id: "photos",
            name: qsTr("Photos"),
            icon: Icons.photo,
            area: "clipboard_files",
            capKey: "photos",
            toggleKey: "files",
            summary: qsTr("Browse recent phone photos and screenshots on demand, copy to clipboard, or drag into apps."),
            whereLabel: qsTr("Left rail › Photos"),
            needsLabel: qsTr("Photos and Media permission on phone"),
            targetId: "photos:main",
            docKey: "photos"
        },
        {
            id: "mirroring",
            name: qsTr("Screen mirroring"),
            icon: Icons.mirror,
            area: "mirroring_apps",
            capKey: "mirror_view",
            toggleKey: "pc_mirror",
            summary: qsTr("Stream your phone display in a low-latency desktop window with mouse and keyboard input."),
            whereLabel: qsTr("Home › Mirror screen"),
            needsLabel: qsTr("MediaProjection approval on phone; Assist or Elevated for touch input"),
            targetId: "home:mirror",
            docKey: "mirroring"
        },
        {
            id: "app_streams",
            name: qsTr("App streams"),
            icon: Icons.apps,
            area: "mirroring_apps",
            capKey: "mirror_view",
            toggleKey: "",
            summary: qsTr("Open individual Android apps in their own virtual display windows on your PC."),
            whereLabel: qsTr("Apps"),
            needsLabel: qsTr("Elevated (Wireless debugging) enabled on phone"),
            targetId: "apps:",
            docKey: "mirroring"
        },
        {
            id: "pc_mirror",
            name: qsTr("Reverse PC screen mirroring"),
            icon: Icons.desktop,
            area: "mirroring_apps",
            capKey: "pc_mirror",
            toggleKey: "pc_mirror",
            summary: qsTr("Stream this PC monitor to your phone and control the cursor or laser pointer from your phone."),
            whereLabel: qsTr("Phone app › Home, and Settings › Devices"),
            needsLabel: qsTr("Opt-in Screen mirroring toggle on Home"),
            targetId: "home:pc_mirror",
            docKey: "pc-mirror"
        },
        {
            id: "webcam",
            name: qsTr("Phone as webcam"),
            icon: Icons.video,
            area: "camera_audio",
            capKey: "webcam",
            toggleKey: "",
            summary: qsTr("Use your phone camera as the NectarLink Virtual Camera in Zoom, Teams, Meet, and OBS."),
            whereLabel: qsTr("Home › Phone as webcam"),
            needsLabel: qsTr("Camera permission on phone and Virtual Camera registered on PC"),
            targetId: "settings:camera_audio:webcam",
            docKey: "webcam"
        },
        {
            id: "continuity_camera",
            name: qsTr("Continuity camera"),
            icon: Icons.camera,
            area: "camera_audio",
            capKey: "webcam",
            toggleKey: "",
            summary: qsTr("Capture a photo or scan a document with your phone and paste it into the active PC window."),
            whereLabel: qsTr("Home › Continuity Camera (Ctrl+Alt+C / Ctrl+Alt+D)"),
            needsLabel: qsTr("Camera permission on phone"),
            targetId: "home:continuity",
            docKey: "continuity-camera"
        },
        {
            id: "speaker",
            name: qsTr("Phone as speaker"),
            icon: Icons.speaker,
            area: "camera_audio",
            capKey: "",
            toggleKey: "",
            prefKey: "streamPcAudioToPhone",
            summary: qsTr("Stream PC system audio to your phone in real time."),
            whereLabel: qsTr("Home › Phone as speaker"),
            needsLabel: qsTr("Phone as speaker enabled in Settings"),
            targetId: "settings:camera_audio:speaker",
            docKey: "speaker"
        },
        {
            id: "recordings",
            name: qsTr("Voice recorder"),
            icon: Icons.mic,
            area: "camera_audio",
            capKey: "files",
            toggleKey: "files",
            summary: qsTr("Record voice notes on your phone and save them automatically to your PC in M4A, MP3, WAV, or FLAC."),
            whereLabel: qsTr("Settings › Files & Sync › Voice recordings"),
            needsLabel: qsTr("Microphone permission on phone"),
            targetId: "settings:files_sync:recordings",
            docKey: "recordings"
        },
        {
            id: "deck",
            name: qsTr("Deck"),
            icon: Icons.deck,
            area: "camera_audio",
            capKey: "media_view",
            toggleKey: "media",
            summary: qsTr("Control Now playing media, switch PC audio outputs, and customize shortcut tiles for your phone."),
            whereLabel: qsTr("Left rail › Deck"),
            needsLabel: qsTr("Paired phone"),
            targetId: "deck:main",
            docKey: "deck"
        },
        {
            id: "desk_mode",
            name: qsTr("Desk mode"),
            icon: Icons.history,
            area: "surfaces",
            capKey: "media_view",
            toggleKey: "media",
            summary: qsTr("Turn your phone or PC window into a glanceable desk display with clock, media, and live status."),
            whereLabel: qsTr("Left rail › Deck › Desk mode"),
            needsLabel: qsTr("Paired phone"),
            targetId: "deck:main",
            docKey: "deck"
        },
        {
            id: "remote_input",
            name: qsTr("Remote input"),
            icon: Icons.laptop,
            area: "input_control",
            capKey: "remote_input",
            toggleKey: "remote_input",
            summary: qsTr("Use your phone as a wireless trackpad, keyboard, media remote, and presentation pointer."),
            whereLabel: qsTr("Phone app › Home › Control PC"),
            needsLabel: qsTr("Opt-in Remote input toggle on Home"),
            targetId: "home:remote_input",
            docKey: "remote-input"
        },
        {
            id: "type_on_phone",
            name: qsTr("Type on phone"),
            icon: Icons.keyboard,
            area: "input_control",
            capKey: "mirror_control",
            toggleKey: "",
            summary: qsTr("Send live keystrokes and pasted text from your PC keyboard to your phone without screen mirroring."),
            whereLabel: qsTr("Home › Type on phone"),
            needsLabel: qsTr("Assist (Accessibility) or Elevated (Wireless debugging) on phone"),
            targetId: "home:type_on_phone",
            docKey: "type-on-phone"
        },
        {
            id: "quick_actions",
            name: qsTr("Phone controls"),
            icon: Icons.flashlight,
            area: "input_control",
            capKey: "find_device",
            toggleKey: "",
            summary: qsTr("Toggle Do Not Disturb, ringer mode, flashlight, volume, brightness, Wi-Fi, Bluetooth, and Find my phone."),
            whereLabel: qsTr("Home › Phone controls"),
            needsLabel: qsTr("Elevated (Wireless debugging) required only for Wi-Fi and Bluetooth toggles"),
            targetId: "home:phone_controls",
            docKey: "phone-controls"
        },
        {
            id: "shelf",
            name: qsTr("Drop shelf"),
            icon: Icons.shelf,
            area: "surfaces",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            localAlways: true,
            summary: qsTr("Slide-out top screen-edge shelf with your latest phone photo, clipboard items, and file drop zone."),
            whereLabel: qsTr("Top screen edge or Ctrl+Alt+S"),
            needsLabel: qsTr("Always available on PC"),
            targetId: "settings:shortcuts:shelf",
            docKey: "shelf"
        },
        {
            id: "nectar_island",
            name: qsTr("Nectar Island"),
            icon: Icons.island,
            area: "surfaces",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            prefKey: "nectarIsland",
            summary: qsTr("Compact floating pill at the top center of your monitor for calls, transfers, media, and alerts."),
            whereLabel: qsTr("Settings › General › Nectar Island"),
            needsLabel: qsTr("Nectar Island enabled in Settings"),
            targetId: "settings:general:island",
            docKey: "nectar-island"
        },
        {
            id: "timeline",
            name: qsTr("Timeline"),
            icon: Icons.history,
            area: "surfaces",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            localAlways: true,
            summary: qsTr("Chronological history of shared files, photos, clipboard items, calls, and voice recordings."),
            whereLabel: qsTr("Top bar › Timeline"),
            needsLabel: qsTr("Always available on PC"),
            targetId: "timeline:main",
            docKey: "timeline"
        },
        {
            id: "command_palette",
            name: qsTr("Command palette"),
            icon: Icons.search,
            area: "surfaces",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            localAlways: true,
            summary: qsTr("Keyboard-first search over actions, contacts, message threads, mirrorable apps, and quick toggles."),
            whereLabel: qsTr("Top bar › Commands, or Ctrl+K (Ctrl+Alt+Space anywhere)"),
            needsLabel: qsTr("Always available on PC"),
            targetId: "settings:shortcuts:palette",
            docKey: "command-palette"
        },
        {
            id: "folder_sync",
            name: qsTr("Share PC folders"),
            icon: Icons.folder,
            area: "clipboard_files",
            capKey: "storage",
            toggleKey: "storage",
            pcLocal: true,
            summary: qsTr("Share selected PC folders with your phone or sync folders in one-way or two-way mode."),
            whereLabel: qsTr("Settings › Files & Sync › Synced folders"),
            needsLabel: qsTr("Opt-in Share PC folders toggle on Home"),
            targetId: "settings:files_sync:folder_sync",
            docKey: "folder-sync"
        },
        {
            id: "wol",
            name: qsTr("Wake PC"),
            icon: Icons.power,
            area: "automation_network",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            wolCheck: true,
            summary: qsTr("Let your paired phone wake this PC from sleep over your local network using Wake-on-LAN."),
            whereLabel: qsTr("Settings › Connection & About › Wake from your phone"),
            needsLabel: qsTr("Network adapter with Wake-on-LAN enabled in Windows"),
            targetId: "settings:connection:wol",
            docKey: "wol"
        },
        {
            id: "doctor",
            name: qsTr("Connection Doctor"),
            icon: Icons.bolt,
            area: "automation_network",
            capKey: "",
            toggleKey: "",
            pcLocal: true,
            localAlways: true,
            summary: qsTr("Diagnose and fix Windows Firewall rules, network profile type, and local discovery issues."),
            whereLabel: qsTr("Settings › Connection & About › Connection Doctor"),
            needsLabel: qsTr("Always available on PC"),
            targetId: "doctor:open",
            docKey: "doctor"
        }
    ]

    property alias featuresModel: guide.allFeatures

    readonly property var areaChips: [
        { id: "all", label: qsTr("All") },
        { id: "notifications_calls", label: qsTr("Notifications & Calls") },
        { id: "messages", label: qsTr("Messages") },
        { id: "clipboard_files", label: qsTr("Clipboard & Files") },
        { id: "mirroring_apps", label: qsTr("Mirroring & Apps") },
        { id: "camera_audio", label: qsTr("Camera & Audio") },
        { id: "input_control", label: qsTr("Input & Control") },
        { id: "surfaces", label: qsTr("Desktop Surfaces") },
        { id: "automation_network", label: qsTr("Automation & Network") }
    ]

    readonly property var filteredFeatures: {
        let q = searchQuery.trim().toLowerCase()
        let area = selectedArea
        let out = []
        for (let i = 0; i < allFeatures.length; i++) {
            let item = allFeatures[i]
            if (area !== "all" && item.area !== area)
                continue
            if (q !== "") {
                let hay = (item.name + " " + item.summary + " " + item.whereLabel + " " + item.needsLabel).toLowerCase()
                if (hay.indexOf(q) === -1)
                    continue
            }
            out.push(item)
        }
        return out
    }

    function statusForFeature(item) {
        let _caps = AppController.capsRevision
        let _toggles = AppController.togglesRevision
        if (item.localAlways)
            return { label: qsTr("Ready on PC"), tone: "available", visible: true }
        if (item.wolCheck) {
            let st = AppController.wakeState
            if (st === "enabled")
                return { label: qsTr("Ready on PC"), tone: "available", visible: true }
            return { label: qsTr("Off in Settings"), tone: "off", visible: true }
        }
        if (item.localsendCheck) {
            return AppController.localsendEnabled
                ? { label: qsTr("Ready on PC"), tone: "available", visible: true }
                : { label: qsTr("Off in Settings"), tone: "off", visible: true }
        }
        if (item.prefKey && item.prefKey !== "") {
            let on = Boolean(Preferences[item.prefKey])
            return on
                ? { label: qsTr("Ready on PC"), tone: "available", visible: true }
                : { label: qsTr("Off in Settings"), tone: "off", visible: true }
        }
        if (item.pcLocal && item.toggleKey === "storage") {
            if (!guide.hasConnectedPhone)
                return { label: qsTr("Opt-in on Home"), tone: "off", visible: true }
        }
        if (!guide.hasConnectedPhone)
            return { label: "", tone: "neutral", visible: false }
        if (item.toggleKey && item.toggleKey !== "") {
            if (!guide.deviceTogglesMap[item.toggleKey])
                return { label: qsTr("Off on Home"), tone: "off", visible: true }
        }
        if (item.capKey && item.capKey !== "") {
            // The names AppController.featureState gives (bridge/app.rs).
            let st = AppController.featureState(guide.deviceId, item.capKey)
            let stateName = st ? (st.state || "") : ""
            let actionName = st ? (st.action || "") : ""
            let target = st ? (st.target || "") : ""
            if (stateName === "available")
                return { label: qsTr("Ready"), tone: "available", visible: true }
            if (stateName === "partial")
                return { label: qsTr("Basic mode"), tone: "basic", visible: true }
            if (stateName === "locked") {
                if (actionName === "enableToggle")
                    return { label: qsTr("Off on Home"), tone: "off", visible: true }
                if (actionName === "raisePower" && target === "elevated")
                    return { label: qsTr("Needs Elevated"), tone: "basic", visible: true }
                if (actionName === "raisePower")
                    return { label: qsTr("Needs Assist"), tone: "basic", visible: true }
                if (actionName === "updateApp")
                    return { label: qsTr("Needs an update"), tone: "warn", visible: true }
                return { label: qsTr("Needs phone permission"), tone: "warn", visible: true }
            }
            if (stateName === "unsupported")
                return { label: qsTr("Not on this phone"), tone: "off", visible: true }
            // Not known yet: nothing rather than a guess.
            return { label: "", tone: "neutral", visible: false }
        }
        return { label: qsTr("Ready"), tone: "available", visible: true }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 28
        anchors.rightMargin: 28
        anchors.topMargin: 16
        anchors.bottomMargin: 20
        spacing: 14

        // C.1 & C.5: Clean header without duplicate "Features guide" heading,
        // with compact "Read the full guide ›" link and "Setup walkthrough" action.
        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            Txt {
                Layout.fillWidth: true
                text: qsTr("Every feature in NectarLink, what it needs, and where to find it.")
                role: "body"
                color: Theme.surfaceContentVariant
                wrapMode: Text.WordWrap
            }

            Rectangle {
                implicitWidth: fullGuideRow.implicitWidth + 16
                implicitHeight: 32
                radius: Theme.radiusSm
                color: fullGuideHover.hovered
                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.12)
                    : "transparent"

                HoverHandler {
                    id: fullGuideHover
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: guide.openDoc("overview")
                }

                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Read the full guide")

                RowLayout {
                    id: fullGuideRow
                    anchors.centerIn: parent
                    spacing: 6

                    Icon {
                        path: Icons.book
                        width: 15
                        height: 15
                        color: Theme.primary
                    }

                    Txt {
                        text: qsTr("Read the full guide ›")
                        role: "label"
                        color: Theme.primary
                    }
                }
            }

            Button {
                text: qsTr("Setup walkthrough")
                iconPath: Icons.compass
                variant: "tonal"
                size: "sm"
                Accessible.name: qsTr("Open the 7-step setup walkthrough")
                onClicked: guide.restartWalkthroughRequested()
            }
        }

        // C.2: Single calm banner when no phone is connected
        Rectangle {
            Layout.fillWidth: true
            implicitWidth: 640
            implicitHeight: 42
            radius: Theme.radiusMd
            visible: !guide.hasConnectedPhone
            color: Theme.surfaceContainerLow
            border.width: 1
            border.color: Theme.outlineVariant

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                spacing: 10

                Icon {
                    path: Icons.phone
                    width: 16
                    height: 16
                    color: Theme.primary
                }

                Txt {
                    Layout.fillWidth: true
                    text: qsTr("Connect your phone to see which features are ready")
                    role: "body"
                    color: Theme.surfaceContent
                    elide: Text.ElideRight
                }
            }
        }

        // Search input + area filter chips
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 10

            Rectangle {
                Layout.fillWidth: true
                height: 40
                implicitWidth: 640
                implicitHeight: 40
                radius: Theme.radiusMd
                color: Theme.surfaceContainerLow
                border.width: searchInput.activeFocus ? 2 : 1
                border.color: searchInput.activeFocus ? Theme.primary : Theme.outlineVariant

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    spacing: 10

                    Icon {
                        path: Icons.search
                        width: 16
                        height: 16
                        color: Theme.surfaceContentVariant
                    }

                    TextInput {
                        id: searchInput
                        Layout.fillWidth: true
                        color: Theme.surfaceContent
                        font.family: Theme.fontUi
                        font.pixelSize: 13
                        selectByMouse: true
                        clip: true
                        Accessible.name: qsTr("Search features guide")
                        onTextChanged: guide.searchQuery = text

                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Search features by name, location, or permission...")
                            role: "body"
                            muted: true
                            visible: searchInput.text.length === 0
                        }
                    }

                    IconButton {
                        iconPath: Icons.close
                        iconSize: 13
                        label: qsTr("Clear search")
                        visible: searchInput.text.length > 0
                        onClicked: {
                            searchInput.text = ""
                            guide.searchQuery = ""
                        }
                    }
                }
            }

            Flow {
                Layout.fillWidth: true
                spacing: 8

                Repeater {
                    model: guide.areaChips
                    delegate: Rectangle {
                        required property var modelData
                        readonly property bool selected: guide.selectedArea === modelData.id
                        height: 30
                        width: chipText.implicitWidth + 24
                        implicitWidth: chipText.implicitWidth + 24
                        implicitHeight: 30
                        radius: 15
                        color: selected ? Theme.secondaryContainer : Theme.surfaceContainerLow
                        border.width: 1
                        border.color: selected ? Theme.primary : Theme.outlineVariant

                        Txt {
                            id: chipText
                            anchors.centerIn: parent
                            text: parent.modelData.label
                            role: "caption"
                            weight: parent.selected ? 600 : 500
                            color: parent.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("Filter by %1").arg(modelData.label)
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: guide.selectedArea = parent.modelData.id
                        }
                    }
                }
            }
        }

        // Scrollable list of feature cards
        ListView {
            id: featureList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 12
            boundsBehavior: Flickable.StopAtBounds
            model: guide.filteredFeatures

            delegate: Card {
                required property var modelData
                width: featureList.width
                height: implicitHeight
                padding: 16

                readonly property var statusInfo: guide.statusForFeature(modelData)

                ColumnLayout {
                    width: parent.width
                    spacing: 10

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12

                        Rectangle {
                            width: 38
                            height: 38
                            implicitWidth: 38
                            implicitHeight: 38
                            radius: Theme.radiusMd
                            color: Theme.secondaryContainer

                            Icon {
                                anchors.centerIn: parent
                                path: modelData.icon
                                width: 18
                                height: 18
                                color: Theme.secondaryContainerContent
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 10

                                Txt {
                                    text: modelData.name
                                    role: "title"
                                }

                                // C.2: Live status badge only when phone is connected or feature is PC-local
                                Rectangle {
                                    visible: Boolean(statusInfo.visible && statusInfo.label !== "")
                                    height: 22
                                    width: badgeLabel.implicitWidth + 16
                                    implicitWidth: badgeLabel.implicitWidth + 16
                                    implicitHeight: 22
                                    radius: 11
                                    color: statusInfo.tone === "available"
                                        ? Theme.secondaryContainer
                                        : (statusInfo.tone === "warn"
                                            ? Qt.rgba(Theme.warning.r, Theme.warning.g, Theme.warning.b, 0.16)
                                            : Theme.surfaceContainerHigh)
                                    border.width: 1
                                    border.color: statusInfo.tone === "available"
                                        ? Theme.primary
                                        : (statusInfo.tone === "warn" ? Theme.warning : Theme.outlineVariant)

                                    Txt {
                                        id: badgeLabel
                                        anchors.centerIn: parent
                                        text: statusInfo.label
                                        role: "caption"
                                        weight: 600
                                        color: statusInfo.tone === "available"
                                            ? Theme.secondaryContainerContent
                                            : (statusInfo.tone === "warn" ? Theme.warning : Theme.surfaceContentVariant)
                                    }
                                }

                                Item { Layout.fillWidth: true }
                            }

                            Txt {
                                Layout.fillWidth: true
                                text: modelData.summary
                                role: "bodySmall"
                                muted: true
                                wrapMode: Text.WordWrap
                            }
                        }

                        // C.3: Quiet tonal "Show me" button + clean text link "Learn more ›"
                        RowLayout {
                            spacing: 10

                            Rectangle {
                                id: showBtn
                                implicitWidth: showLabel.implicitWidth + 24
                                implicitHeight: 30
                                radius: Theme.radiusSm
                                color: showHover.hovered
                                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.18)
                                    : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.10)

                                HoverHandler {
                                    id: showHover
                                    cursorShape: Qt.PointingHandCursor
                                }

                                TapHandler {
                                    onTapped: guide.showMeRequested(modelData.targetId)
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("Show me %1").arg(modelData.name)

                                Txt {
                                    id: showLabel
                                    anchors.centerIn: parent
                                    text: qsTr("Show me")
                                    role: "label"
                                    color: Theme.primary
                                }
                            }

                            Rectangle {
                                id: learnBtn
                                implicitWidth: learnLabel.implicitWidth + 12
                                implicitHeight: 30
                                radius: Theme.radiusSm
                                color: "transparent"

                                HoverHandler {
                                    id: learnHover
                                    cursorShape: Qt.PointingHandCursor
                                }

                                TapHandler {
                                    onTapped: guide.openDoc(modelData.docKey)
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: qsTr("Learn more about %1").arg(modelData.name)

                                Txt {
                                    id: learnLabel
                                    anchors.centerIn: parent
                                    text: qsTr("Learn more ›")
                                    role: "label"
                                    color: learnHover.hovered ? Theme.primary : Theme.surfaceContentVariant
                                }
                            }
                        }
                    }

                    Divider { Layout.fillWidth: true }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 20

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 6

                            Txt {
                                text: qsTr("Where:")
                                role: "caption"
                                weight: 600
                                muted: true
                            }

                            Txt {
                                Layout.fillWidth: true
                                text: modelData.whereLabel
                                role: "caption"
                                muted: true
                                elide: Text.ElideRight
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 6

                            Txt {
                                text: qsTr("Needs:")
                                role: "caption"
                                weight: 600
                                muted: true
                            }

                            Txt {
                                Layout.fillWidth: true
                                text: modelData.needsLabel
                                role: "caption"
                                muted: true
                                elide: Text.ElideRight
                            }
                        }
                    }
                }
            }

            // Empty search state
            ColumnLayout {
                anchors.centerIn: parent
                spacing: 8
                visible: guide.filteredFeatures.length === 0

                Txt {
                    Layout.alignment: Qt.AlignHCenter
                    text: qsTr("No matching features")
                    role: "title"
                }

                Txt {
                    Layout.alignment: Qt.AlignHCenter
                    text: qsTr("Try another search term or select All above.")
                    role: "bodySmall"
                    muted: true
                }
            }
        }
    }

    // C.5: In-app reader sheet with topic tab bar for Overview, Setup, Permissions,
    // Power levels, Troubleshooting, and FAQ
    Sheet {
        id: readerSheet
        cardWidth: 640
        // Not a binding: dismissing sets `opened`, which would break it, and
        // the next guide wouldn't open.
        Connections {
            target: AppController
            function onGuideDocOpenChanged() {
                if (AppController.guideDocOpen)
                    readerSheet.open()
                else
                    readerSheet.close()
            }
        }
        onOpenedChanged: {
            if (!opened && AppController.guideDocOpen)
                AppController.closeGuideDoc()
        }

        ColumnLayout {
            width: parent.width
            spacing: 14

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Rectangle {
                    width: 36
                    height: 36
                    implicitWidth: 36
                    implicitHeight: 36
                    radius: Theme.radiusMd
                    color: Theme.secondaryContainer

                    Icon {
                        anchors.centerIn: parent
                        path: Icons.book
                        width: 18
                        height: 18
                        color: Theme.secondaryContainerContent
                    }
                }

                Txt {
                    Layout.fillWidth: true
                    text: AppController.guideDocTitle
                    role: "headline"
                    wrapMode: Text.WordWrap
                }

                IconButton {
                    iconPath: Icons.close
                    label: qsTr("Close guide")
                    onClicked: AppController.closeGuideDoc()
                }
            }

            // Topic tabs inside reader sheet
            Flow {
                Layout.fillWidth: true
                spacing: 6

                Repeater {
                    model: guide.guideChapters
                    delegate: Rectangle {
                        required property var modelData
                        readonly property bool activeTab: guide.activeDocKey === modelData.key
                        implicitWidth: tabTxt.implicitWidth + 20
                        implicitHeight: 28
                        radius: 14
                        color: activeTab
                            ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.16)
                            : Theme.surfaceContainerLow
                        border.width: 1
                        border.color: activeTab ? Theme.primary : Theme.outlineVariant

                        HoverHandler {
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            onTapped: guide.openDoc(modelData.key)
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("Read %1").arg(modelData.label + " chapter")

                        Txt {
                            id: tabTxt
                            anchors.centerIn: parent
                            text: modelData.label
                            role: "caption"
                            weight: activeTab ? 600 : 500
                            color: activeTab ? Theme.primary : Theme.surfaceContentVariant
                        }
                    }
                }
            }

            Divider { Layout.fillWidth: true }

            Flickable {
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(380, docBodyText.implicitHeight + 16)
                contentWidth: width
                contentHeight: docBodyText.implicitHeight + 8
                clip: true
                boundsBehavior: Flickable.StopAtBounds

                Txt {
                    id: docBodyText
                    width: parent.width
                    text: AppController.guideDocBody
                    role: "body"
                    muted: true
                    wrapMode: Text.WordWrap
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button {
                    text: qsTr("Close")
                    variant: "tonal"
                    size: "sm"
                    onClicked: AppController.closeGuideDoc()
                }
            }
        }
    }
}
