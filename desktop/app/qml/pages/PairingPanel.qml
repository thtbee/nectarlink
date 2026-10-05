// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Pairing a phone: a QR code to scan (refreshed automatically), nearby
// pairing with a 6-digit code check, and the outcome. While `running`, this
// PC is in pairing mode.
Item {
    id: panel
    property bool running: false
    // Show "Cancel" (in a dialog) or not (first run).
    property bool cancellable: true
    signal finished
    signal cancelled

    property bool showNearby: false
    readonly property string state_: Pairing.state

    implicitHeight: content.implicitHeight

    onRunningChanged: {
        showNearby = false
        if (running)
            Pairing.start()
    }
    Component.onCompleted: if (running) Pairing.start()

    // Leave the success message up briefly, then close.
    Timer {
        interval: 1400
        running: panel.state_ === "paired" && panel.running
        onTriggered: panel.finished()
    }

    // Seconds left on the current code, for the "refreshes in" hint.
    property int secondsLeft: 0
    Timer {
        interval: 1000
        repeat: true
        triggeredOnStart: true
        running: panel.running && panel.state_ === "hosting"
        onTriggered: panel.secondsLeft = Math.max(0, Math.round((Pairing.expiresAt - Date.now()) / 1000))
    }

    Column {
        id: content
        width: parent.width
        spacing: 18

        // ---- Header ----
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "displaySmall"
            size: 26
            wrapMode: Text.WordWrap
            text: {
                switch (panel.state_) {
                case "comparing": return qsTr("Check the code")
                case "paired": return qsTr("Paired")
                case "failed": return qsTr("Pairing didn't work")
                default: return panel.showNearby ? qsTr("Pair nearby") : qsTr("Scan with your phone")
                }
            }
        }

        // ---- QR code ----
        Column {
            width: parent.width
            spacing: 14
            visible: (panel.state_ === "hosting" || panel.state_ === "starting" || panel.state_ === "idle") && !panel.showNearby

            Item {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 236; height: 236
                QrCode {
                    anchors.fill: parent
                    visible: panel.state_ === "hosting"
                    path: Pairing.qrPath
                    modules: Pairing.qrSize
                    Accessible.role: Accessible.Graphic
                    Accessible.name: qsTr("Pairing QR code")
                }
                Spinner {
                    anchors.centerIn: parent
                    visible: panel.state_ !== "hosting"
                    width: 28; height: 28
                }
            }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 14
                Repeater {
                    model: [qsTr("Install Nectarlink"), qsTr("Tap “Pair with PC”"), qsTr("Scan this code")]
                    delegate: Row {
                        required property int index
                        required property string modelData
                        spacing: 6
                        Rectangle {
                            width: 20; height: 20; radius: Theme.graphite ? Theme.radiusXs : 10
                            color: Theme.graphite ? "transparent" : Theme.secondaryContainer
                            border.width: Theme.graphite ? 1 : 0
                            border.color: Theme.outlineVariant
                            Txt { anchors.centerIn: parent; text: index + 1; role: "caption"; weight: 700 }
                        }
                        Txt { anchors.verticalCenter: parent.verticalCenter; text: modelData; role: "bodySmall"; muted: true }
                    }
                }
            }
            Txt {
                anchors.horizontalCenter: parent.horizontalCenter
                role: "caption"
                muted: true
                visible: panel.state_ === "hosting"
                text: qsTr("Code refreshes in %1:%2 · Works on this network")
                      .arg(Math.floor(panel.secondsLeft / 60)).arg(("0" + panel.secondsLeft % 60).slice(-2))
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                variant: "tonal"
                size: "sm"
                iconPath: Icons.wifi
                text: qsTr("Pair nearby instead")
                onClicked: panel.showNearby = true
            }
        }

        // ---- Nearby devices ----
        Column {
            width: parent.width
            spacing: 10
            visible: panel.showNearby && (panel.state_ === "hosting" || panel.state_ === "starting")

            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("On your phone, open Nectarlink and choose “Pair nearby”. It shows up here.")
            }
            Repeater {
                model: Pairing.nearby
                delegate: Rectangle {
                    required property var modelData
                    width: parent.width
                    height: 56
                    radius: Theme.radiusMd
                    color: Theme.tileColor
                    border.width: Theme.graphite ? 1 : 0
                    border.color: Theme.outlineVariant
                    Avatar {
                        id: deviceIcon
                        anchors.left: parent.left; anchors.leftMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        iconPath: Icons.phone
                    }
                    Txt {
                        anchors.left: deviceIcon.right; anchors.leftMargin: 12
                        anchors.right: pairButton.left; anchors.rightMargin: 12
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.name
                        role: "title"
                    }
                    Button {
                        id: pairButton
                        anchors.right: parent.right; anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        size: "sm"
                        text: qsTr("Pair")
                        onClicked: Pairing.pairNearby(modelData.id)
                    }
                }
            }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 8
                visible: Pairing.nearby.length === 0
                Spinner { width: 16; height: 16; anchors.verticalCenter: parent.verticalCenter }
                Txt { text: qsTr("Looking for devices…"); role: "bodySmall"; muted: true }
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                variant: "text"
                size: "sm"
                text: qsTr("Show the QR code")
                onClicked: panel.showNearby = false
            }
        }

        // ---- Working ----
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 10
            visible: panel.state_ === "connecting" || panel.state_ === "confirmed"
            Spinner { width: 18; height: 18; anchors.verticalCenter: parent.verticalCenter }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                role: "body"
                text: panel.state_ === "connecting"
                      ? qsTr("Connecting to %1…").arg(Pairing.peerName)
                      : qsTr("Waiting for %1…").arg(Pairing.peerName)
            }
        }

        // ---- Code check ----
        Column {
            width: parent.width
            spacing: 16
            visible: panel.state_ === "comparing" || panel.state_ === "confirmed"
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("Make sure %1 shows the same code. This keeps anyone else from pairing in between.")
                      .arg(Pairing.peerName.length > 0 ? Pairing.peerName : qsTr("your phone"))
            }
            CodeDigits { anchors.horizontalCenter: parent.horizontalCenter; code: Pairing.code }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 10
                visible: panel.state_ === "comparing"
                Button { variant: "outline"; text: qsTr("They don't match"); onClicked: Pairing.confirm(false) }
                Button { text: qsTr("Codes match"); onClicked: Pairing.confirm(true) }
            }
        }

        // ---- Outcome ----
        Column {
            width: parent.width
            spacing: 12
            visible: panel.state_ === "paired" || panel.state_ === "failed"
            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56; height: 56
                emphasized: panel.state_ === "paired"
                iconPath: panel.state_ === "paired" ? Icons.check : Icons.warning
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: panel.state_ === "failed"
                text: {
                    if (panel.state_ === "paired")
                        return qsTr("%1 is paired with this PC.").arg(Pairing.peerName)
                    switch (Pairing.failure) {
                    case "rejected": return qsTr("The codes didn't match, so nothing was paired. Try again.")
                    case "declined": return qsTr("Pairing was declined on one of the devices.")
                    case "unreachable": return qsTr("Couldn't reach the phone. Make sure both devices are on the same network.")
                    case "expired": return qsTr("The code expired.")
                    default: return qsTr("Something went wrong. Try again.")
                    }
                }
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: panel.state_ === "failed"
                text: qsTr("Try again")
                iconPath: Icons.refresh
                onClicked: { panel.showNearby = false; Pairing.start() }
            }
        }

        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: panel.cancellable && panel.state_ !== "paired"
            variant: "text"
            text: qsTr("Cancel")
            onClicked: panel.cancelled()
        }
    }
}
