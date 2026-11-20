// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// After a crash: a small card in the main window (not a dialog, and never
// on a minimized start) offering to send a crash report. Once per crash per
// version; "Don't ask about crashes" turns it off for good.
Rectangle {
    id: card

    property bool eligible: false
    readonly property bool sent: Report.crashState === "sent"

    anchors.left: parent ? parent.left : undefined
    anchors.bottom: parent ? parent.bottom : undefined
    anchors.leftMargin: 20
    anchors.bottomMargin: 20
    width: Math.min(400, parent ? parent.width - 40 : 400)
    height: content.height + 32
    radius: Theme.radiusLg
    color: Theme.surfaceContainerHigh
    border.width: 1
    border.color: Theme.outlineVariant
    visible: eligible && Report.crash !== ""
    opacity: visible ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }

    Accessible.role: Accessible.AlertMessage
    Accessible.name: title.text

    // Asked once the app is up, so a crash from the last run is found.
    onEligibleChanged: if (eligible) Report.checkCrash()

    Timer {
        // "Thanks" shows a moment, then the card goes.
        running: card.sent
        interval: 2500
        onTriggered: Report.crashDone(false)
    }

    Column {
        id: content
        x: 16
        y: 16
        width: card.width - 32
        spacing: 10

        Txt {
            id: title
            width: parent.width
            role: "title"
            wrapMode: Text.WordWrap
            text: card.sent ? qsTr("Thanks, the crash report is sent") : qsTr("NectarLink closed unexpectedly")
        }
        Txt {
            width: parent.width
            visible: !card.sent
            role: "bodySmall"
            muted: true
            wrapMode: Text.WordWrap
            text: Report.crashFixedIn !== ""
                ? qsTr("This crash is fixed in NectarLink %1. Update to get the fix.").arg(Report.crashFixedIn)
                : Report.crashState === "failed"
                ? qsTr("The crash report couldn't be sent. Use Look first to send it another way.")
                : qsTr("A crash report helps fix it. It says where in NectarLink it happened and which versions you run, nothing personal.")
        }
        Flow {
            width: parent.width
            spacing: 8
            visible: !card.sent
            Button {
                visible: Report.crashFixedIn !== ""
                text: qsTr("Update")
                onClicked: {
                    AppController.checkForUpdates()
                    Report.crashDone(false)
                }
            }
            Button {
                visible: Report.crashFixedIn === ""
                text: qsTr("Send")
                enabled: Report.crashState === ""
                onClicked: Report.crashSend()
            }
            Button {
                visible: Report.crashFixedIn === ""
                variant: "tonal"
                text: qsTr("Look first")
                enabled: Report.crashState !== "sending"
                onClicked: Report.crashDone(true)
            }
            Button {
                variant: "text"
                text: qsTr("Not now")
                enabled: Report.crashState !== "sending"
                onClicked: Report.crashDone(false)
            }
        }
        Txt {
            visible: !card.sent
            role: "bodySmall"
            color: Theme.primary
            text: qsTr("Don't ask about crashes")
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: Report.crashOff()
            }
            Accessible.role: Accessible.Link
            Accessible.name: text
            Accessible.onPressAction: Report.crashOff()
        }
    }
}
