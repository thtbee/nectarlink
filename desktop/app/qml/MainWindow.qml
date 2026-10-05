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
    backdrop: Preferences.backdrop
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

    // Window background: translucent over Mica, opaque otherwise.
    Rectangle {
        anchors.fill: parent
        color: Preferences.backdrop ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.7 : 0.62)
                                    : Theme.surface
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
    }

    // ---- Navigation rail ----
    Rectangle {
        id: rail
        visible: !window.welcome
        width: Theme.railWidth
        height: parent.height
        color: Preferences.backdrop ? "transparent" : Theme.railColor

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
                text: window.page === "home" ? qsTr("Home") : qsTr("Settings")
                role: "headline"
                size: 20
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
                anchors.fill: parent
                active: window.page === "home"
                onPairRequested: window.openPairing()
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

    CaptionButtons {
        id: captionButtons
        window: window
        anchors.top: parent.top
        anchors.right: parent.right
    }

    Toast { id: toast }
    Connections {
        target: AppController
        function onToast(message) { toast.show(message) }
    }
}
