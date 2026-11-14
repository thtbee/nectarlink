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
    // Checks for features that need setup (ids "linux-..."), as opposed to
    // what stands between the phone and this PC.
    readonly property bool connectionWell: checks.length > 0
        && checks.every(c => c.outcome === "ok" || c.id.startsWith("linux-"))

    function fixLabel(fix) {
        switch (fix) {
        case "firewall": return qsTr("Allow")
        case "network-settings": return qsTr("Open settings")
        case "reconnect": return qsTr("Reconnect")
        case "start-adb": return qsTr("Start adb")
        case "get-adb": return qsTr("Set up USB")
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
                  ? qsTr("Checking network and firewall…")
                  : sheet.allWell
                    ? qsTr("Everything looks right.")
                    : sheet.connectionWell
                      ? qsTr("Your phone can reach this PC. A few features need a step from you:")
                      : qsTr("Issues blocking your phone from reaching this PC:")
        }

        Flickable {
            width: parent.width
            height: Math.min(checksColumn.height, 360)
            contentWidth: width
            contentHeight: checksColumn.height
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            Column {
                id: checksColumn
                width: parent.width
                spacing: 2
                opacity: AppController.doctorBusy ? 0.55 : 1
                Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
                Repeater {
                    model: sheet.checks
                    delegate: Rectangle {
                        id: row
                        required property var modelData
                        readonly property bool hasFix: row.modelData.fix !== undefined && row.modelData.fix !== ""
                        width: parent.width
                        height: Math.max(badge.height, words.height, actionsRow.height) + 20
                        radius: Theme.radiusMd
                        color: "transparent"

                        Rectangle {
                            id: badge
                            x: 8
                            anchors.verticalCenter: parent.verticalCenter
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
                            anchors.right: actionsRow.visible ? actionsRow.left : parent.right
                            anchors.rightMargin: actionsRow.visible ? 12 : 8
                            anchors.verticalCenter: parent.verticalCenter
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
                        Row {
                            id: actionsRow
                            anchors.right: parent.right
                            anchors.rightMargin: 6
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 6
                            visible: row.hasFix

                            Button {
                                visible: row.modelData.fix === "firewall"
                                enabled: !AppController.doctorBusy
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Firewall settings")
                                onClicked: AppController.doctorFix("firewall-settings")
                            }
                            Button {
                                id: fixButton
                                // A fix this sheet has no words for gets no blank button.
                                visible: sheet.fixLabel(row.modelData.fix).length > 0
                                enabled: !AppController.doctorBusy
                                variant: row.modelData.outcome === "fail" ? "fill" : "tonal"
                                size: "sm"
                                text: sheet.fixLabel(row.modelData.fix)
                                onClicked: {
                                    // Getting the USB tools is a page of its own (Settings › Install on a phone).
                                    if (row.modelData.fix === "get-adb") {
                                        sheet.close()
                                        AppController.triggerHighlight("settings:install_phone")
                                    } else {
                                        AppController.doctorFix(row.modelData.fix)
                                    }
                                }
                            }
                        }
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
