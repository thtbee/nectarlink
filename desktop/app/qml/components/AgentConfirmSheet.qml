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

    // What the agent asks for, in the user's language. Only actions that can
    // ask the user are here; anything else shows the title it came with.
    function titleFor(action, fallback) {
        switch (action) {
        case "notification.dismiss": return qsTr("Dismiss a notification")
        case "notification.action": return qsTr("Press a notification button")
        case "notification.reply": return qsTr("Reply to a notification")
        case "sms.send": return qsTr("Send a text message")
        case "chat.reply": return qsTr("Reply in a chat")
        case "call.dial": return qsTr("Place a call")
        case "call.control": return qsTr("Control the current call")
        case "clipboard.history_delete": return qsTr("Delete a clipboard entry")
        case "files.send": return qsTr("Send files to a device")
        case "link.open": return qsTr("Open a link on a device")
        case "pc.lock": return qsTr("Lock this PC")
        case "pc.power": return qsTr("Lock, sleep or shut down a PC")
        case "pc.wake": return qsTr("Wake a PC")
        case "phone.settings_set": return qsTr("Change a phone quick setting")
        case "photos.save": return qsTr("Copy photos to this PC")
        case "photos.delete": return qsTr("Delete photos")
        case "storage.save": return qsTr("Copy a phone file to this PC")
        case "storage.upload": return qsTr("Put a file on the phone")
        case "storage.mkdir": return qsTr("Make a folder on the phone")
        case "storage.rename": return qsTr("Rename or move a phone file")
        case "storage.delete": return qsTr("Delete a phone file")
        case "mirror.open": return qsTr("Open a phone app on this PC")
        case "app.uninstall": return qsTr("Uninstall a phone app")
        case "deck.run": return qsTr("Press a Deck tile")
        case "transfer.accept": return qsTr("Accept incoming files")
        default: return fallback
        }
    }

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
            text: sheet.request ? qsTr("%1 wants to: %2").arg(sheet.request.client).arg(sheet.titleFor(sheet.request.action, sheet.request.title)) : ""
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
