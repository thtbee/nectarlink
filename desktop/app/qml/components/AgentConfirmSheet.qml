// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// An AI agent asks to do something that needs the user's say-so (send a
// text, call, delete...). Shows exactly what will happen. Deny is the safe
// default; Allow only works after a moment, so a stray Enter can't approve.
Sheet {
    id: sheet
    readonly property var request: {
        try { return Agents.confirmation.length > 0 ? JSON.parse(Agents.confirmation) : null } catch (e) { return null }
    }
    property bool armed: false

    opened: request !== null
    dismissable: false
    cardWidth: 480
    onRequestChanged: {
        armed = false
        if (request !== null) armTimer.restart()
    }

    Timer { id: armTimer; interval: 1000; onTriggered: sheet.armed = true }
    Shortcut {
        sequence: "Esc"
        enabled: sheet.opened && sheet.request !== null
        onActivated: Agents.answer(sheet.request.id, "deny")
    }

    Column {
        width: parent.width
        spacing: 14
        visible: sheet.request !== null

        Txt {
            width: parent.width
            role: "headline"
            wrapMode: Text.WordWrap
            text: sheet.request ? qsTr("%1 wants to: %2").arg(sheet.request.client).arg(sheet.request.title) : ""
        }
        Txt {
            width: parent.width
            muted: true
            wrapMode: Text.WordWrap
            visible: sheet.request !== null && !!sheet.request.device
            text: sheet.request && sheet.request.device ? qsTr("On %1").arg(sheet.request.device) : ""
        }

        Rectangle {
            width: parent.width
            height: Math.min(detailsColumn.implicitHeight + 24, 280)
            radius: Theme.radiusMd
            color: Theme.surfaceContainerHigh
            visible: sheet.request !== null && sheet.request.details.length > 0
            Flickable {
                anchors.fill: parent
                anchors.margins: 12
                contentHeight: detailsColumn.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: detailsColumn
                    width: parent.width
                    spacing: 8
                    Repeater {
                        model: sheet.request ? sheet.request.details : []
                        Column {
                            required property var modelData
                            width: detailsColumn.width
                            spacing: 2
                            Txt { text: modelData.label; role: "label"; muted: true }
                            Txt {
                                width: parent.width
                                text: modelData.value
                                wrapMode: Text.WrapAnywhere
                                textFormat: Text.PlainText
                            }
                        }
                    }
                }
            }
        }

        Txt {
            width: parent.width
            role: "caption"
            muted: true
            wrapMode: Text.WordWrap
            text: qsTr("This request came from an AI agent, not from you. Only allow it if you asked for it.")
        }

        Row {
            anchors.right: parent.right
            spacing: 8
            Button {
                variant: "tonal"
                text: qsTr("Deny")
                onClicked: Agents.answer(sheet.request.id, "deny")
            }
            Button {
                variant: "text"
                visible: sheet.request !== null && sheet.request.canRemember
                enabled: sheet.armed
                text: qsTr("Always allow for this agent")
                onClicked: Agents.answer(sheet.request.id, "always")
            }
            Button {
                enabled: sheet.armed
                text: qsTr("Allow once")
                onClicked: Agents.answer(sheet.request.id, "allow")
            }
        }
    }
}
