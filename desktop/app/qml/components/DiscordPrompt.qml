// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Asks, now and then, to come chat on the Discord server: only after an
// update, at most once a month, and never again after joining or after
// it was shown three times (settings.rs). It waits for the star prompt,
// which goes first, and comes a little after the window is in use.
Item {
    id: prompt

    // Whether it may appear now (the window is ready and nothing else asks).
    property bool eligible: false
    property int delayMs: 20000
    signal joined

    readonly property bool open: internal.phase === "open"

    anchors.right: parent ? parent.right : undefined
    anchors.bottom: parent ? parent.bottom : undefined
    anchors.rightMargin: 20
    anchors.bottomMargin: 20
    width: pill.width
    height: pill.height
    visible: internal.shown > 0
    opacity: internal.shown
    transform: Translate { y: Theme.reduceMotion ? 0 : (1 - internal.shown) * 8 }

    Accessible.role: Accessible.AlertMessage
    Accessible.name: question.text

    function join() {
        if (internal.phase !== "open")
            return
        Preferences.communityPromptDone = true
        prompt.joined()
        close()
    }

    function dismiss() {
        if (internal.phase !== "open")
            return
        close()
    }

    function close() {
        internal.phase = "gone"
    }

    QtObject {
        id: internal
        property string phase: "waiting"
        property real shown: phase === "open" ? 1 : 0
        Behavior on shown { NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic } }
    }

    Timer {
        interval: prompt.delayMs
        running: prompt.eligible && internal.phase === "waiting"
            && Preferences.communityPromptDue && !Preferences.communityPromptDone
        onTriggered: {
            // Shown counts, answered or not.
            Preferences.communityPromptDue = false
            Preferences.communityPromptCount += 1
            internal.phase = "open"
        }
    }

    Rectangle {
        id: pill
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 48
        width: row.implicitWidth
        radius: Theme.pill(height)
        color: Theme.surfaceContainerHigh
        border.width: 1
        border.color: Theme.outlineVariant

        // Holds clicks, so they don't reach the page underneath.
        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
        }

        Row {
            id: row
            anchors.verticalCenter: parent.verticalCenter
            leftPadding: 9
            rightPadding: 8
            spacing: 10

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 30
                height: 30
                radius: 15
                color: Theme.primaryContainer
                Icon {
                    anchors.centerIn: parent
                    width: 16
                    height: 16
                    path: Icons.messages
                    color: Theme.primaryContainerContent
                }
            }

            Txt {
                id: question
                anchors.verticalCenter: parent.verticalCenter
                role: "body"
                color: Theme.surfaceContent
                text: qsTr("Come chat on Discord?")
                rightPadding: 2
            }

            Button {
                objectName: "joinButton"
                anchors.verticalCenter: parent.verticalCenter
                size: "sm"
                text: qsTr("Join Discord")
                enabled: prompt.open
                onClicked: prompt.join()
            }

            IconButton {
                objectName: "closeButton"
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.close
                label: qsTr("Dismiss")
                enabled: prompt.open
                onClicked: prompt.dismiss()
            }
        }
    }
}
