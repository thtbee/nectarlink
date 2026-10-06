// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The Connection Doctor: what stands between the phone and this PC, worst
// first, each with a fix when one can be made from here.
Sheet {
    id: sheet
    cardWidth: 520

    readonly property var checks: { try { return JSON.parse(AppController.doctorChecks) } catch (e) { return [] } }
    readonly property bool allWell: checks.length > 0 && checks.every(c => c.outcome === "ok")

    function fixLabel(fix) {
        switch (fix) {
        case "firewall": return qsTr("Allow")
        case "network-settings": return qsTr("Open settings")
        case "reconnect": return qsTr("Reconnect")
        default: return ""
        }
    }

    Column {
        width: parent.width
        spacing: 16

        Item {
            width: parent.width
            height: Math.max(heading.height, busy.height)
            Txt {
                id: heading
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Connection Doctor")
                role: "headline"
            }
            Spinner {
                id: busy
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                visible: AppController.doctorBusy
            }
        }
        Txt {
            width: parent.width
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: AppController.doctorBusy && sheet.checks.length === 0
                  ? qsTr("Checking this PC's network and firewall…")
                  : sheet.allWell
                    ? qsTr("Everything looks right. If your phone still can't connect, open Nectarlink on it and keep both on the same network.")
                    : qsTr("Here's what may keep your phone from reaching this PC.")
        }

        Column {
            width: parent.width
            spacing: 2
            opacity: AppController.doctorBusy ? 0.55 : 1
            Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
            Repeater {
                model: sheet.checks
                delegate: Item {
                    id: row
                    required property var modelData
                    width: parent.width
                    height: Math.max(badge.height, words.height, fixButton.height) + 20

                    Rectangle {
                        id: badge
                        x: 0; y: 12
                        width: 28; height: 28
                        radius: 14
                        color: row.modelData.outcome === "ok" ? Theme.secondaryContainer
                             : row.modelData.outcome === "warn" ? Qt.alpha(Theme.warning, 0.18)
                             : Qt.alpha(Theme.error, 0.16)
                        Icon {
                            anchors.centerIn: parent
                            width: 16; height: 16
                            path: row.modelData.outcome === "ok" ? Icons.check : Icons.warning
                            color: row.modelData.outcome === "ok" ? Theme.secondaryContainerContent
                                 : row.modelData.outcome === "warn" ? Theme.warning : Theme.error
                        }
                    }
                    Column {
                        id: words
                        anchors.left: badge.right
                        anchors.leftMargin: 14
                        anchors.right: fixButton.visible ? fixButton.left : parent.right
                        anchors.rightMargin: fixButton.visible ? 12 : 0
                        y: 12
                        spacing: 3
                        Txt { width: parent.width; text: row.modelData.title; role: "title"; size: 14; wrapMode: Text.WordWrap }
                        Txt {
                            width: parent.width
                            visible: row.modelData.detail.length > 0
                            text: row.modelData.detail
                            role: "bodySmall"
                            muted: true
                            wrapMode: Text.WordWrap
                        }
                    }
                    Button {
                        id: fixButton
                        anchors.right: parent.right
                        y: 10
                        visible: row.modelData.fix !== undefined
                        enabled: !AppController.doctorBusy
                        variant: row.modelData.outcome === "fail" ? "fill" : "tonal"
                        size: "sm"
                        text: sheet.fixLabel(row.modelData.fix)
                        onClicked: AppController.doctorFix(row.modelData.fix)
                    }
                }
            }
        }

        Row {
            anchors.right: parent.right
            spacing: 10
            Button {
                variant: "text"
                text: qsTr("Check again")
                enabled: !AppController.doctorBusy
                onClicked: AppController.runDoctor()
            }
            Button { text: qsTr("Done"); onClicked: sheet.close() }
        }
    }
}
