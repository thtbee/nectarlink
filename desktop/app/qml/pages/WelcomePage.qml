// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// First run: pair a phone. Also covers the moment before the core is ready
// and the (rare) case where it failed to start.
Item {
    id: page

    readonly property string status: AppController.status

    Column {
        anchors.centerIn: parent
        width: Math.min(460, parent.width - 48)
        spacing: 22

        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 52; height: 52
            radius: Theme.radiusMd
            color: Theme.graphite ? Theme.surfaceContent : Theme.primary
            Icon {
                anchors.centerIn: parent
                width: 26; height: 26
                path: Icons.mark
                color: Theme.graphite ? Theme.surface : Theme.primaryContent
            }
        }
        Column {
            width: parent.width
            spacing: 6
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Welcome to Nectarlink")
                role: "displaySmall"
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: page.status === "failed"
                      ? qsTr("Nectarlink couldn't start.")
                      : qsTr("Your phone and this PC, together. Pair once and they find each other from then on.")
            }
        }

        Card {
            width: parent.width
            padding: 24
            visible: page.status === "ready"
            PairingPanel {
                width: parent.width
                cancellable: false
                running: page.visible && page.status === "ready"
            }
        }

        Spinner {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.status === "starting"
            width: 28; height: 28
        }

        Column {
            width: parent.width
            spacing: 12
            visible: page.status === "failed"
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "code"
                size: 12
                muted: true
                text: AppController.error
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                variant: "tonal"
                iconPath: Icons.folder
                text: qsTr("Open the logs folder")
                onClicked: Qt.openUrlExternally(AppController.logsUrl())
            }
        }
    }
}
