// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import app.nectarlink

Sheet {
    id: sheet
    cardWidth: 540

    property string snapshotJson: "null"
    readonly property var snapshot: JSON.parse(snapshotJson)
    readonly property var probes: JSON.parse(AppController.relayProbes || "[]")
    property string editError: ""
    readonly property var regions: [
        { key: "north_america_east", title: qsTr("North America · East") },
        { key: "north_america_west", title: qsTr("North America · West") },
        { key: "europe", title: qsTr("Europe") },
        { key: "asia_pacific", title: qsTr("Asia-Pacific") },
        { key: "custom", title: qsTr("Custom") }
    ]

    function edit(url, action) {
        const result = AppController.editRelay(url, action)
        switch (result) {
        case "":
            editError = ""
            snapshotJson = AppController.relaySnapshot()
            return true
        case "invalid":
            editError = qsTr("Enter an HTTP or HTTPS relay address without credentials, a path or a query.")
            break
        case "duplicate":
            editError = qsTr("This relay is already in the list.")
            break
        case "limit":
            editError = qsTr("The list can hold up to 16 custom relays.")
            break
        case "unavailable":
            editError = qsTr("The connection service is unavailable.")
            break
        default:
            editError = qsTr("Couldn't save the relay settings.")
        }
        return false
    }

    function addRelay() {
        if (address.text.trim().length > 0 && edit(address.text, "add"))
            address.clear()
    }

    onOpenedChanged: {
        if (opened) {
            snapshotJson = AppController.relaySnapshot()
            editError = ""
        }
    }

    Timer {
        interval: 2000
        repeat: true
        running: sheet.opened && sheet.visible
        onTriggered: sheet.snapshotJson = AppController.relaySnapshot()
    }

    Column {
        width: parent.width
        spacing: 10

        Row {
            width: parent.width
            spacing: 8
            Txt {
                width: parent.width - closeButton.width - parent.spacing
                anchors.verticalCenter: parent.verticalCenter
                role: "headline"
                text: qsTr("Iroh relay servers")
            }
            Button {
                id: closeButton
                variant: "text"
                text: qsTr("Close")
                onClicked: sheet.close()
            }
        }
        Txt {
            width: parent.width
            wrapMode: Text.Wrap
            muted: true
            visible: sheet.snapshot === null || !Preferences.awayMode
            text: sheet.snapshot === null ? qsTr("Waiting for the connection service…")
                                          : qsTr("Turn on Connect over mobile data to use your enabled relays.")
        }
        Txt {
            width: parent.width
            wrapMode: Text.Wrap
            muted: true
            text: qsTr("The switches control which servers iroh can select as a home relay for this device.")
        }
        Txt { role: "label"; text: qsTr("Home relays") }
        Txt {
            visible: !sheet.snapshot || sheet.snapshot.home.length === 0
            text: qsTr("No home relay selected")
            muted: true
        }
        Repeater {
            model: sheet.snapshot ? sheet.snapshot.home : []
            delegate: Txt {
                required property var modelData
                width: parent.width
                wrapMode: Text.Wrap
                text: modelData.connected ? qsTr("Connected · %1").arg(modelData.url)
                      : modelData.failed ? qsTr("Connection failed · %1").arg(modelData.url)
                                         : qsTr("Connecting · %1").arg(modelData.url)
            }
        }
        Txt { role: "label"; text: qsTr("Device traffic via relay") }
        Txt {
            visible: !sheet.snapshot || sheet.snapshot.routes.length === 0
            text: qsTr("No device traffic routed through a relay")
            muted: true
        }
        Repeater {
            model: sheet.snapshot ? sheet.snapshot.routes : []
            delegate: Txt {
                required property var modelData
                width: parent.width
                wrapMode: Text.Wrap
                text: qsTr("%1 via %2").arg(modelData.device).arg(modelData.url)
            }
        }
        Divider { width: parent.width }

        Button {
            variant: "tonal"
            text: AppController.relayCheckBusy ? qsTr("Checking relays…") : qsTr("Check all relays")
            enabled: sheet.snapshot !== null && !AppController.relayCheckBusy
            onClicked: AppController.checkRelays()
        }
        Txt { role: "label"; text: qsTr("Custom relay address") }
        Row {
            width: parent.width
            spacing: 8
            Rectangle {
                width: parent.width - addButton.width - parent.spacing
                height: 40
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                border.width: address.activeFocus ? 2 : 1
                border.color: address.activeFocus ? Theme.primary : Theme.outlineVariant
                Txt {
                    anchors.fill: parent
                    anchors.margins: 10
                    text: "https://relay.example.com"
                    muted: true
                    visible: address.text.length === 0
                }
                TextInput {
                    id: address
                    anchors.fill: parent
                    anchors.margins: 10
                    color: Theme.surfaceContent
                    font.pixelSize: 14
                    maximumLength: 1024
                    selectByMouse: true
                    clip: true
                    Accessible.name: qsTr("Custom relay address")
                    onAccepted: sheet.addRelay()
                }
            }
            Button {
                id: addButton
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Add relay")
                enabled: sheet.snapshot !== null && address.text.trim().length > 0
                onClicked: sheet.addRelay()
            }
        }
        Txt {
            width: parent.width
            wrapMode: Text.Wrap
            visible: text.length > 0
            text: sheet.editError
        }

        Repeater {
            model: sheet.regions
            delegate: Column {
                id: group
                required property var modelData
                readonly property var servers: sheet.snapshot
                    ? sheet.snapshot.servers.filter(server => server.region === modelData.key) : []
                width: parent.width
                spacing: 6
                visible: servers.length > 0
                Divider { width: parent.width }
                Txt { role: "label"; text: group.modelData.title }
                Repeater {
                    model: group.servers
                    delegate: Row {
                        id: serverRow
                        required property var modelData
                        readonly property var probe: sheet.probes.find(result => result.url === modelData.url)
                        width: group.width
                        spacing: 8
                        Column {
                            width: serverRow.width - controls.width - serverRow.spacing
                            Txt {
                                width: parent.width
                                wrapMode: Text.Wrap
                                text: serverRow.modelData.provider
                                    ? qsTr("%1 · %2").arg(serverRow.modelData.provider).arg(serverRow.modelData.url)
                                    : serverRow.modelData.url
                            }
                            Row {
                                spacing: 5
                                StatusDot {
                                    anchors.verticalCenter: parent.verticalCenter
                                    online: !AppController.relayCheckBusy
                                        && !!serverRow.probe && serverRow.probe.rtt_ms !== null
                                }
                                Txt {
                                    muted: true
                                    role: "bodySmall"
                                    text: AppController.relayCheckBusy ? qsTr("Checking…")
                                          : !serverRow.probe ? qsTr("Not checked")
                                          : serverRow.probe.rtt_ms !== null ? qsTr("%1 ms").arg(serverRow.probe.rtt_ms)
                                          : serverRow.probe.timed_out ? qsTr("Timed out") : qsTr("Unreachable")
                                }
                            }
                        }
                        Row {
                            id: controls
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 6
                            Button {
                                visible: group.modelData.key === "custom"
                                variant: "text"
                                size: "sm"
                                text: qsTr("Remove")
                                onClicked: sheet.edit(serverRow.modelData.url, "remove")
                            }
                            Toggle {
                                anchors.verticalCenter: parent.verticalCenter
                                label: qsTr("Enable %1").arg(serverRow.modelData.url)
                                checked: serverRow.modelData.enabled
                                onToggled: function(enabled) {
                                    sheet.edit(serverRow.modelData.url, enabled ? "enable" : "disable")
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
