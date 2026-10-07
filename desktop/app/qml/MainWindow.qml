// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The main window: navigation rail, title bar and pages. Before any device
// is paired it shows the welcome (pairing) screen instead.
NativeWindow {
    id: window
    signal closeRequested

    readonly property bool welcome: AppController.status === "ready" && !AppController.hasDevices
    property string page: "home"

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

    // QQuickWindow's `closing` is a revisioned member, hidden on subclasses
    // in a 1.0 module; Connections reaches it through the meta-object.
    Connections {
        target: window
        function onClosing(close) {
            close.accepted = true
            window.closeRequested()
        }
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

            HomePage {
                id: homePage
                anchors.fill: parent
                active: window.page === "home"
                onPairRequested: window.openPairing()
            }
            MessagesPage {
                anchors.fill: parent
                active: window.page === "messages"
                deviceId: homePage.currentDeviceId
                deviceName: homePage.currentDeviceName
            }
            CallsPage {
                anchors.fill: parent
                active: window.page === "calls"
                deviceId: homePage.currentDeviceId
                deviceName: homePage.currentDeviceName
                onTextRequested: (number, name) => {
                    Messages.startChat(homePage.currentDeviceId, number, name)
                    window.page = "messages"
                }
            }
            PhotosPage {
                anchors.fill: parent
                active: window.page === "photos"
                deviceId: homePage.currentDeviceId
                deviceName: homePage.currentDeviceName
            }
            DeckPage {
                anchors.fill: parent
                active: window.page === "deck"
            }
            SettingsPage {
                anchors.fill: parent
                active: window.page === "settings"
                onPairRequested: window.openPairing()
            }
        }
    }

    // ---- First run ----
    WelcomePage {
        anchors.fill: parent
        visible: window.welcome || AppController.status !== "ready"
    }

    // ---- Overlays ----
    Sheet {
        id: pairingSheet
        cardWidth: 480
        onOpenedChanged: if (!opened) Pairing.cancel()
        PairingPanel {
            width: parent.width
            running: pairingSheet.opened
            onFinished: pairingSheet.close()
            onCancelled: pairingSheet.close()
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

    DoctorSheet { id: doctorSheet }
    Connections {
        target: AppController
        function onDoctorRequested() { doctorSheet.open() }
    }

    Toast { id: toast }
    Connections {
        target: AppController
        function onToast(message) { toast.show(message) }
    }
}
