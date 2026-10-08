// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The main window: navigation rail, title bar and pages. Before any device
// is paired it shows the welcome (pairing) screen instead.
NativeWindow {
    id: window

    readonly property bool welcome: AppController.status === "ready" && !AppController.hasDevices
    property string page: AppController.currentPage.length > 0 ? AppController.currentPage : "home"
    onPageChanged: if (AppController.currentPage !== page) AppController.currentPage = page
    readonly property int currentDeviceIndex: DeviceList.count > 0
        ? Math.min(Math.max(0, AppController.currentDevice), DeviceList.count - 1) : 0
    readonly property string currentDeviceId: DeviceList.count > 0
        ? DeviceList.deviceIdAt(currentDeviceIndex) : ""
    readonly property string currentDeviceName: DeviceList.count > 0
        ? DeviceList.deviceNameAt(currentDeviceIndex) : ""

    width: 1100
    height: 720
    minimumWidth: Theme.layout.minWindow[0]
    minimumHeight: Theme.layout.minWindow[1]
    title: "Nectarlink"
    // Mica is applied by Windows a moment after it's requested. Showing
    // through before it's there reveals the bare desktop (a flash), so the
    // window turns translucent only once Mica is in place, and opaque again
    // before Mica is removed.
    property bool micaApplied: false
    property bool micaVisible: false
    backdrop: micaApplied
    Timer {
        id: micaSettle
        interval: 280
        onTriggered: window.micaVisible = Preferences.backdrop
    }
    Timer {
        id: micaRelease
        interval: Theme.fadeNormal + 60
        onTriggered: window.micaApplied = Preferences.backdrop
    }
    function followBackdrop() {
        if (Preferences.backdrop) {
            micaRelease.stop()
            micaApplied = true
            micaSettle.restart()
        } else {
            micaSettle.stop()
            micaVisible = false
            micaRelease.restart()
        }
    }
    Connections {
        target: Preferences
        function onBackdropChanged() { window.followBackdrop() }
    }
    Component.onCompleted: followBackdrop()
    darkFrame: Theme.dark
    captionHeight: Theme.topBarHeight

    Connections {
        target: window
        // Back from Settings, maybe with Windows notifications turned on.
        function onActiveChanged() {
            if (window.active)
                AppController.refreshToastsEnabled()
        }
    }

    function openPairing() { pairingSheet.open() }

    // Window background: a light veil of the theme over Mica (so the
    // desktop's colors come through, as Mica is meant to), opaque otherwise.
    Rectangle {
        anchors.fill: parent
        color: window.micaVisible ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.45 : 0.32)
                                  : Theme.surface
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
    }

    // ---- Navigation rail ----
    Rectangle {
        id: rail
        visible: !window.welcome
        width: Theme.railWidth
        height: parent.height
        color: window.micaVisible ? "transparent" : Theme.railColor

        Rectangle {
            id: mark
            anchors.horizontalCenter: parent.horizontalCenter
            y: 16
            width: 38; height: 38
            radius: Theme.radiusSm
            color: Theme.graphite ? Theme.surfaceContent : Theme.primary
            Icon {
                anchors.centerIn: parent
                path: Icons.mark
                color: Theme.graphite ? Theme.surface : Theme.primaryContent
            }
        }
        Column {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: mark.bottom
            anchors.topMargin: 22
            spacing: 6
            NavItem {
                iconPath: Icons.home
                text: qsTr("Home")
                selected: window.page === "home"
                onClicked: window.page = "home"
            }
            NavItem {
                iconPath: Icons.messages
                text: qsTr("Messages")
                selected: window.page === "messages"
                onClicked: window.page = "messages"
            }
            NavItem {
                iconPath: Icons.call
                text: qsTr("Calls")
                selected: window.page === "calls"
                onClicked: window.page = "calls"
            }
            NavItem {
                iconPath: Icons.photo
                text: qsTr("Photos")
                selected: window.page === "photos"
                onClicked: window.page = "photos"
            }
            NavItem {
                iconPath: Icons.deck
                text: qsTr("Deck")
                selected: window.page === "deck"
                onClicked: window.page = "deck"
            }
            NavItem {
                iconPath: Icons.settings
                text: qsTr("Settings")
                selected: window.page === "settings"
                onClicked: window.page = "settings"
            }
        }
        IconButton {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 18
            iconPath: Icons.plus
            label: qsTr("Pair a new device")
            tonal: true
            onClicked: window.openPairing()
        }
        Divider { anchors.right: parent.right; width: 1; height: parent.height }
    }

    // ---- Title bar and pages ----
    Item {
        id: main
        visible: !window.welcome
        anchors.left: rail.right
        anchors.right: parent.right
        height: parent.height

        Item {
            id: topBar
            width: parent.width - captionButtons.width
            height: Theme.topBarHeight
            Txt {
                id: pageTitle
                anchors.left: parent.left
                anchors.leftMargin: Theme.contentPadding
                anchors.verticalCenter: parent.verticalCenter
                text: window.page === "home" ? qsTr("Home")
                    : window.page === "messages" ? qsTr("Messages")
                    : window.page === "calls" ? qsTr("Calls")
                    : window.page === "photos" ? qsTr("Photos")
                    : window.page === "deck" ? qsTr("Deck")
                    : qsTr("Settings")
                role: "headline"
                size: 20
            }
            // Reconnects and syncs with the phones (the title bar lets
            // clicks through here).
            IconButton {
                id: refresh
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                visible: window.page === "home" && AppController.hasDevices
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                onClicked: {
                    AppController.syncNow()
                    if (!Theme.reduceMotion) spin.restart()
                }
                RotationAnimation on rotation {
                    id: spin
                    running: false
                    from: 0; to: 360
                    duration: 700
                    easing.type: Easing.OutCubic
                }
                Component.onCompleted: window.addCaptionHole(refresh)
            }
        }
        Divider { anchors.top: topBar.bottom; width: parent.width }

        Item {
            id: pages
            anchors.top: topBar.bottom
            anchors.bottom: parent.bottom
            width: parent.width
            clip: true

            Loader {
                id: pageLoader
                anchors.fill: parent
                active: !window.welcome && AppController.status === "ready"
                source: {
                    if (!active) return ""
                    switch (window.page) {
                    case "messages": return "qrc:/qt/qml/app/nectarlink/qml/pages/MessagesPage.qml"
                    case "calls": return "qrc:/qt/qml/app/nectarlink/qml/pages/CallsPage.qml"
                    case "photos": return "qrc:/qt/qml/app/nectarlink/qml/pages/PhotosPage.qml"
                    case "deck": return "qrc:/qt/qml/app/nectarlink/qml/pages/DeckPage.qml"
                    case "settings": return "qrc:/qt/qml/app/nectarlink/qml/pages/SettingsPage.qml"
                    default: return "qrc:/qt/qml/app/nectarlink/qml/pages/HomePage.qml"
                    }
                }
                onLoaded: {
                    if ("deviceId" in item)
                        item.deviceId = Qt.binding(() => window.currentDeviceId)
                    if ("deviceName" in item)
                        item.deviceName = Qt.binding(() => window.currentDeviceName)
                    item.active = true
                    if (item.pairRequested)
                        item.pairRequested.connect(window.openPairing)
                    if (item.textRequested) {
                        item.textRequested.connect((number, name) => {
                            Messages.startChat(window.currentDeviceId, number, name)
                            window.page = "messages"
                        })
                    }
                }
            }
        }
    }

    // ---- First run ----
    Loader {
        anchors.fill: parent
        active: window.welcome || AppController.status !== "ready"
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/pages/WelcomePage.qml" : ""
    }

    // ---- Overlays ----
    Sheet {
        id: pairingSheet
        cardWidth: 480
        onOpenedChanged: if (!opened) Pairing.cancel()
        Loader {
            id: pairingLoader
            width: parent.width
            active: pairingSheet.opened
            source: active ? "qrc:/qt/qml/app/nectarlink/qml/pages/PairingPanel.qml" : ""
            onLoaded: {
                item.width = Qt.binding(() => pairingLoader.width)
                item.running = Qt.binding(() => pairingSheet.opened)
                item.finished.connect(() => pairingSheet.close())
                item.cancelled.connect(() => pairingSheet.close())
            }
        }
    }

    Sheet {
        id: ringingSheet
        opened: AppController.ringingFrom.length > 0
        dismissable: false
        cardWidth: 380
        Column {
            width: parent.width
            spacing: 16
            Avatar { anchors.horizontalCenter: parent.horizontalCenter; width: 56; height: 56; iconPath: Icons.ring; emphasized: true }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("%1 is ringing this PC").arg(AppController.ringingFrom)
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Stop ringing")
                onClicked: AppController.stopRinging()
            }
        }
    }

    Sheet {
        id: remotePromptSheet
        opened: AppController.remotePromptDeviceId.length > 0
        cardWidth: 420
        onOpenedChanged: {
            if (!opened && AppController.remotePromptDeviceId.length > 0)
                AppController.dismissRemotePrompt()
        }
        Column {
            width: parent.width
            spacing: 16
            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56; height: 56
                iconPath: Icons.phone
                emphasized: true
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Allow %1 to control this PC?").arg(AppController.remotePromptDeviceName)
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("%1 wants to move the mouse, type, and control presentations on this PC. You can change this anytime in Settings.").arg(AppController.remotePromptDeviceName)
                role: "body"
                muted: true
                wrapMode: Text.WordWrap
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Not now")
                    onClicked: AppController.dismissRemotePrompt()
                }
                Button {
                    text: qsTr("Allow")
                    onClicked: AppController.allowRemoteInput(AppController.remotePromptDeviceId)
                }
            }
        }
    }

    CaptionButtons {
        id: captionButtons
        window: window
        anchors.top: parent.top
        anchors.right: parent.right
    }

    Loader {
        id: doctorLoader
        anchors.fill: parent
        active: false
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/components/DoctorSheet.qml" : ""
        onLoaded: item.open()
    }
    Connections {
        target: AppController
        function onDoctorRequested() {
            if (doctorLoader.active && doctorLoader.item)
                doctorLoader.item.open()
            else
                doctorLoader.active = true
        }
    }

    Toast {
        id: toast
        onActionTriggered: AppController.runClipSuggestion()
    }
    Connections {
        target: AppController
        function onToast(message) { toast.show(message) }
        function onToastWithAction(message, actionLabel) { toast.showWithAction(message, actionLabel) }
        function onCurrentPageChanged() {
            if (AppController.currentPage.length > 0 && window.page !== AppController.currentPage)
                window.page = AppController.currentPage
        }
        function onPendingDialChanged() {
            if (AppController.pendingDial.length > 0 && window.page !== "calls")
                window.page = "calls"
        }
    }
}
