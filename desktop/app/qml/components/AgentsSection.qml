// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Settings > AI agents: the switch, the agents with their permissions, how
// to connect each one, and what they did.
Column {
    id: section
    spacing: Theme.gutter

    readonly property var clients: { try { return JSON.parse(Agents.clients) } catch (e) { return [] } }
    readonly property var activity: { try { return JSON.parse(Agents.activity) } catch (e) { return [] } }
    readonly property var clientKinds: [
        { value: "claude-code", label: "Claude Code" },
        { value: "claude-desktop", label: "Claude Desktop" },
        { value: "codex", label: "Codex" },
        { value: "cursor", label: "Cursor" },
        { value: "vscode", label: "VS Code" },
        { value: "windsurf", label: "Windsurf" },
        { value: "gemini", label: "Gemini CLI" },
        { value: "antigravity", label: "Antigravity" },
        { value: "zed", label: "Zed" },
        { value: "cli", label: qsTr("Script or CLI") }
    ]
    // What an agent may be allowed, area by area. `write: false` areas can only be read.
    readonly property var scopes: [
        { id: "devices", label: qsTr("Devices"), write: true },
        { id: "notifications", label: qsTr("Notifications"), write: true },
        { id: "notifications.sensitive", label: qsTr("One-time codes and private notifications"), write: false },
        { id: "messages", label: qsTr("Messages"), write: true },
        { id: "calls", label: qsTr("Calls"), write: true },
        { id: "contacts", label: qsTr("Contacts"), write: false },
        { id: "clipboard", label: qsTr("Clipboard"), write: true },
        { id: "files", label: qsTr("Files and Timeline"), write: true },
        { id: "media", label: qsTr("Media"), write: true },
        { id: "photos", label: qsTr("Photos"), write: true },
        { id: "storage", label: qsTr("Phone storage"), write: true },
        { id: "phone_settings", label: qsTr("Phone quick settings"), write: true },
        { id: "mirroring", label: qsTr("Phone app windows"), write: true },
        { id: "deck", label: qsTr("Deck"), write: true },
        { id: "pc_actions", label: qsTr("Lock, sleep, shut down and wake PCs"), write: true }
    ]
    property string newKind: "claude-code"
    property string newPreset: "read_only"
    property string addError: ""
    property string shownLabel: ""
    property string shownKind: ""
    // The agent whose permissions are open.
    property string editing: ""
    readonly property bool canSetUp: shownKind.length > 0 && Agents.canSetUp(shownKind)
    readonly property var setupStatus: { try { return Agents.setupStatus.length > 0 ? JSON.parse(Agents.setupStatus) : null } catch (e) { return null } }
    property bool settingUp: false
    onSetupStatusChanged: if (setupStatus !== null) settingUp = false

    function accessOf(grants, scope) {
        if (grants.indexOf(scope + ":write") >= 0) return "write"
        if (grants.indexOf(scope + ":read") >= 0) return "read"
        return "off"
    }
    function showConnect(label, kind) {
        Agents.clearSetupStatus()
        settingUp = false
        shownLabel = label
        shownKind = kind
    }

    function kindName(kind) {
        for (let i = 0; i < clientKinds.length; i++)
            if (clientKinds[i].value === kind) return clientKinds[i].label
        return kind
    }
    function freeLabel(kind) {
        let label = kind, n = 2
        const taken = (l) => clients.some(c => c.label === l)
        while (taken(label)) label = kind + "-" + (n++)
        return label
    }
    function presetName(p) {
        return p === "everyday" ? qsTr("Everyday") : p === "read_only" ? qsTr("Read only") : qsTr("Custom")
    }
    function decisionName(d) {
        switch (d) {
        case "ok": return qsTr("done")
        case "confirm_denied": return qsTr("denied by you")
        case "confirm_timeout": return qsTr("not approved in time")
        case "confirm_pending": return qsTr("waiting for you")
        case "scope_denied": return qsTr("not permitted")
        case "read_only": return qsTr("blocked, read-only")
        case "toggle_denied": return qsTr("off for that device")
        case "rate_limited": return qsTr("too many requests")
        default: return qsTr("failed")
        }
    }

    Card {
        width: parent.width
        Column {
            width: parent.width
            ListRow {
                width: parent.width
                iconPath: Icons.terminal
                title: qsTr("Let AI agents use NectarLink")
                description: Preferences.agentAccessEnabled
                    ? qsTr("On. Agents only get the permissions you give each one, and sensitive actions always ask you first.")
                    : qsTr("Claude Code, Codex, Cursor and other AI assistants on this PC can read and act through NectarLink, with your approval.")
                Toggle {
                    label: qsTr("Let AI agents use NectarLink")
                    checked: Preferences.agentAccessEnabled
                    onToggled: (on) => Preferences.agentAccessEnabled = on
                }
            }
            Divider { width: parent.width; visible: Preferences.agentAccessEnabled }
            ListRow {
                width: parent.width
                visible: Preferences.agentAccessEnabled
                iconPath: Icons.lock
                title: qsTr("Read-only for every agent")
                description: qsTr("Agents can look, but can't send, change or delete anything.")
                Toggle {
                    label: qsTr("Read-only for every agent")
                    checked: Preferences.agentReadOnly
                    onToggled: (on) => Preferences.agentReadOnly = on
                }
            }
        }
    }

    Card {
        width: parent.width
        visible: Preferences.agentAccessEnabled
        Column {
            width: parent.width
            spacing: 12
            Txt { text: qsTr("Add an agent"); role: "title" }
            Flow {
                width: parent.width
                spacing: 6
                Repeater {
                    model: section.clientKinds
                    Chip {
                        required property var modelData
                        interactive: true
                        text: modelData.label
                        selected: section.newKind === modelData.value
                        onClicked: section.newKind = modelData.value
                    }
                }
            }
            Segmented {
                options: [
                    { value: "read_only", label: qsTr("Read only") },
                    { value: "everyday", label: qsTr("Everyday") }
                ]
                value: section.newPreset
                onPicked: (value) => section.newPreset = value
            }
            Txt {
                width: parent.width
                role: "bodySmall"
                muted: true
                wrapMode: Text.WordWrap
                text: section.newPreset === "everyday"
                      ? qsTr("Reads everything except the clipboard and one-time codes. Can send messages, files and notifications, control media, copy photos, open phone apps on this PC and change the phone's quick settings. Sending a text, replying, calling or deleting always asks you first.")
                      : qsTr("Reads devices, notifications, messages, calls, contacts, photos, files and media. Can't change anything. The clipboard and one-time codes stay private.")
            }
            Row {
                spacing: 10
                Button {
                    text: qsTr("Add %1").arg(section.kindName(section.newKind))
                    onClicked: {
                        const label = section.freeLabel(section.newKind)
                        section.addError = Agents.addAgent(label, section.kindName(section.newKind), section.newKind === "cli" ? "cli" : "mcp-stdio", section.newPreset)
                        if (section.addError.length === 0)
                            section.showConnect(label, section.newKind)
                    }
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: section.addError.length > 0
                    text: section.addError
                    color: Theme.error
                }
            }
        }
    }

    Card {
        width: parent.width
        visible: Preferences.agentAccessEnabled && section.shownLabel.length > 0
        Column {
            width: parent.width
            spacing: 10
            Txt {
                width: parent.width
                role: "title"
                wrapMode: Text.WordWrap
                text: qsTr("Connect %1").arg(section.kindName(section.shownKind))
            }
            Row {
                visible: section.canSetUp
                spacing: 10
                Button {
                    text: qsTr("Add to %1").arg(section.kindName(section.shownKind))
                    enabled: !section.settingUp
                    onClicked: {
                        section.settingUp = true
                        Agents.setUp(section.shownLabel, section.shownKind)
                    }
                }
                Spinner {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: section.settingUp
                }
            }
            Txt {
                readonly property var status: section.setupStatus !== null && section.setupStatus.kind === section.shownKind ? section.setupStatus : null
                width: parent.width
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                visible: status !== null
                color: status !== null && !status.ok ? Theme.error : Theme.surfaceContent
                text: status === null ? ""
                      : status.ok ? qsTr("Done. Restart %1 so it picks up NectarLink.").arg(section.kindName(section.shownKind))
                      : status.message
            }
            Txt {
                width: parent.width
                role: "bodySmall"
                muted: true
                wrapMode: Text.WordWrap
                text: section.canSetUp ? qsTr("Or add it yourself:")
                      : section.shownKind === "claude-code" || section.shownKind === "gemini"
                      ? qsTr("Run this in a terminal:")
                      : section.shownKind === "codex" ? qsTr("Add this to ~/.codex/config.toml:")
                      : section.shownKind === "cli" ? qsTr("Run commands like this one. `nectarlink-agent actions` lists the rest.")
                      : qsTr("Add this to the app's MCP settings:")
            }
            Rectangle {
                width: parent.width
                height: snippetText.implicitHeight + 20
                radius: Theme.radiusSm
                color: Theme.surfaceContainerHigh
                Txt {
                    id: snippetText
                    anchors.fill: parent
                    anchors.margins: 10
                    role: "code"
                    wrapMode: Text.WrapAnywhere
                    textFormat: Text.PlainText
                    text: section.shownLabel.length > 0 ? Agents.snippet(section.shownLabel, section.shownKind) : ""
                }
            }
            Row {
                spacing: 8
                Button {
                    variant: "tonal"
                    size: "sm"
                    text: qsTr("Copy")
                    onClicked: AppController.copyText(snippetText.text)
                }
                Button {
                    variant: "text"
                    size: "sm"
                    text: qsTr("Done")
                    onClicked: {
                        section.shownLabel = ""
                        Agents.clearSetupStatus()
                    }
                }
            }
        }
    }

    Card {
        width: parent.width
        visible: Preferences.agentAccessEnabled && section.clients.length > 0
        Column {
            width: parent.width
            Txt { text: qsTr("Agents"); role: "title"; bottomPadding: 8 }
            Repeater {
                model: section.clients
                Column {
                    required property var modelData
                    required property int index
                    width: parent.width
                    Divider { width: parent.width; visible: index > 0 }
                    ListRow {
                        width: parent.width
                        iconPath: Icons.terminal
                        title: modelData.name + (modelData.paused ? " · " + qsTr("paused") : "")
                        description: section.presetName(modelData.preset)
                                     + (modelData.readOnly ? " · " + qsTr("read-only") : "")
                                     + " · " + (modelData.lastUsed > 0
                                                ? qsTr("last used %1").arg(new Date(modelData.lastUsed * 1000).toLocaleString(Qt.locale(), Locale.ShortFormat))
                                                : qsTr("not connected yet"))
                        Row {
                            spacing: 6
                            Button {
                                variant: "text"
                                size: "sm"
                                text: qsTr("Connect")
                                onClicked: section.showConnect(modelData.label, modelData.label.replace(/-[0-9]+$/, ""))
                            }
                            Button {
                                variant: "text"
                                size: "sm"
                                text: qsTr("Permissions")
                                onClicked: section.editing = section.editing === modelData.id ? "" : modelData.id
                            }
                            Button {
                                variant: "text"
                                size: "sm"
                                text: modelData.paused ? qsTr("Resume") : qsTr("Pause")
                                onClicked: Agents.setPaused(modelData.id, !modelData.paused)
                            }
                            Button {
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Remove")
                                onClicked: Agents.remove(modelData.id)
                            }
                        }
                    }
                    Column {
                        id: permissions
                        readonly property var grants: modelData.grants
                        readonly property string clientId: modelData.id
                        width: parent.width
                        visible: section.editing === modelData.id
                        spacing: 6
                        bottomPadding: 12
                        Txt {
                            width: parent.width
                            role: "bodySmall"
                            muted: true
                            wrapMode: Text.WordWrap
                            text: qsTr("Each area can only narrow what a device's own permissions allow. Sensitive actions still ask you every time.")
                        }
                        Repeater {
                            model: section.scopes
                            Item {
                                required property var modelData
                                width: permissions.width
                                height: Math.max(scopeName.implicitHeight, scopeLevel.implicitHeight)
                                Txt {
                                    id: scopeName
                                    anchors.left: parent.left
                                    anchors.right: scopeLevel.left
                                    anchors.rightMargin: 12
                                    anchors.verticalCenter: parent.verticalCenter
                                    wrapMode: Text.WordWrap
                                    text: modelData.label
                                }
                                Segmented {
                                    id: scopeLevel
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    options: modelData.write
                                        ? [{ value: "off", label: qsTr("Off") }, { value: "read", label: qsTr("Read") }, { value: "write", label: qsTr("Read and act") }]
                                        : [{ value: "off", label: qsTr("Off") }, { value: "read", label: qsTr("Read") }]
                                    value: section.accessOf(permissions.grants, modelData.id)
                                    onPicked: (value) => Agents.setAccess(permissions.clientId, modelData.id, value)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Card {
        width: parent.width
        visible: Preferences.agentAccessEnabled
        Column {
            width: parent.width
            spacing: 6
            Row {
                width: parent.width
                Txt { text: qsTr("Activity"); role: "title"; width: parent.width - clearButton.width }
                Button {
                    id: clearButton
                    variant: "text"
                    size: "sm"
                    visible: section.activity.length > 0
                    text: qsTr("Clear")
                    onClicked: Agents.clearActivity()
                }
            }
            Txt {
                visible: section.activity.length === 0
                muted: true
                text: qsTr("Nothing yet.")
            }
            Repeater {
                model: section.activity.slice(0, 30)
                Txt {
                    required property var modelData
                    width: parent.width
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    text: new Date(modelData.ts * 1000).toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
                          + "  " + modelData.client + "  " + modelData.action
                          + (modelData.device ? " (" + modelData.device + ")" : "")
                          + "  " + section.decisionName(modelData.decision)
                }
            }
        }
    }
}
