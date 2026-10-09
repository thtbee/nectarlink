// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Fast, keyboard-first Command Palette modal overlay (PLAN.md §3.9 / §3.11 P1).
// Loaded lazily via a Loader in MainWindow.qml only while open, so it costs
// zero memory and zero CPU while closed.
Item {
    id: palette
    anchors.fill: parent
    z: 150

    property int selectedIndex: 0
    readonly property var items: {
        const raw = AppController.commandPaletteResults
        if (!raw || raw.length === 0)
            return []
        try {
            return JSON.parse(raw)
        } catch (e) {
            return []
        }
    }

    onItemsChanged: {
        if (selectedIndex >= items.length)
            selectedIndex = Math.max(0, items.length - 1)
    }

    function runSelected() {
        if (items.length === 0)
            return
        const idx = Math.max(0, Math.min(selectedIndex, items.length - 1))
        const item = items[idx]
        if (item && item.id)
            AppController.runCommandPalette(item.id)
    }

    Component.onCompleted: {
        searchInput.forceActiveFocus()
        Qt.callLater(() => AppController.noteCommandPaletteShown())
    }

    Shortcut {
        sequence: "Esc"
        enabled: palette.visible
        onActivated: AppController.closeCommandPalette()
    }

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, Theme.dark ? 0.52 : 0.28)
        TapHandler {
            onTapped: (eventPoint) => {
                const p = card.mapFromItem(scrim, eventPoint.position)
                const inside = p.x >= 0 && p.y >= 0 && p.x <= card.width && p.y <= card.height
                if (!inside)
                    AppController.closeCommandPalette()
            }
        }
    }

    Rectangle {
        id: card
        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.max(56, Math.round(parent.height * 0.12))
        width: Math.min(580, parent.width - 48)
        height: cardColumn.height
        radius: Theme.radiusLg
        color: Theme.surfaceContainerHigh
        border.width: 1
        border.color: Theme.outlineVariant
        clip: true

        Column {
            id: cardColumn
            width: parent.width

            // ---- Search bar ----
            Item {
                width: parent.width
                height: 54

                Icon {
                    id: searchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18
                    height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }

                Item {
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 12
                    anchors.right: escBadge.left
                    anchors.rightMargin: 10
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom

                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: searchInput.text.length === 0
                        text: qsTr("Type a command, contact, or app…")
                        role: "body"
                        muted: true
                    }

                    TextInput {
                        id: searchInput
                        anchors.fill: parent
                        verticalAlignment: TextInput.AlignVCenter
                        font.family: Theme.fontUi
                        font.pixelSize: 15
                        color: Theme.surfaceContent
                        selectionColor: Theme.primaryContainer
                        selectedTextColor: Theme.primaryContainerContent
                        focus: true
                        clip: true
                        Accessible.name: qsTr("Command palette search")

                        onTextChanged: {
                            palette.selectedIndex = 0
                            AppController.searchCommandPalette(searchInput.text)
                        }

                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Down) {
                                if (palette.items.length > 0) {
                                    palette.selectedIndex = (palette.selectedIndex + 1) % palette.items.length
                                    resultsList.positionViewAtIndex(palette.selectedIndex, ListView.Contain)
                                }
                                event.accepted = true
                            } else if (event.key === Qt.Key_Up) {
                                if (palette.items.length > 0) {
                                    palette.selectedIndex = (palette.selectedIndex - 1 + palette.items.length) % palette.items.length
                                    resultsList.positionViewAtIndex(palette.selectedIndex, ListView.Contain)
                                }
                                event.accepted = true
                            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                                palette.runSelected()
                                event.accepted = true
                            } else if (event.key === Qt.Key_Escape) {
                                AppController.closeCommandPalette()
                                event.accepted = true
                            }
                        }
                    }
                }

                Rectangle {
                    id: escBadge
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: escLabel.implicitWidth + 12
                    height: 22
                    radius: Theme.radiusXs
                    color: Theme.surfaceContainerHighest
                    border.width: 1
                    border.color: Theme.outlineVariant
                    activeFocusOnTab: true

                    function activate() {
                        if (searchInput.text.length > 0) {
                            searchInput.text = ""
                            searchInput.forceActiveFocus()
                        } else {
                            AppController.closeCommandPalette()
                        }
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: searchInput.text.length > 0 ? qsTr("Clear search") : qsTr("Close command palette")
                    Accessible.onPressAction: escBadge.activate()

                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            escBadge.activate()
                            event.accepted = true
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -3
                        radius: parent.radius + 3
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        visible: Theme.focusVisible(escBadge)
                    }

                    Txt {
                        id: escLabel
                        anchors.centerIn: parent
                        text: searchInput.text.length > 0 ? qsTr("Clear") : qsTr("Esc")
                        role: "caption"
                        size: 11
                        muted: true
                    }

                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: escBadge.activate() }
                }
            }

            Divider { width: parent.width }

            // ---- Results list ----
            Item {
                width: parent.width
                height: palette.items.length > 0 ? Math.min(resultsList.contentHeight + 12, 360) : 72

                ListView {
                    id: resultsList
                    anchors.fill: parent
                    anchors.margins: 6
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    visible: palette.items.length > 0
                    model: palette.items
                    spacing: 2

                    delegate: Rectangle {
                        id: row
                        required property var modelData
                        required property int index
                        readonly property bool selected: index === palette.selectedIndex
                        readonly property bool isEnabled: modelData.enabled !== false

                        width: resultsList.width
                        height: 52
                        radius: Theme.radiusSm
                        color: selected
                            ? Theme.secondaryContainer
                            : rowHover.hovered
                              ? Theme.surfaceContainerHighest
                              : "transparent"

                        Accessible.role: Accessible.ListItem
                        Accessible.name: (modelData.title || "") + (modelData.subtitle ? ", " + modelData.subtitle : "")
                        Accessible.onPressAction: {
                            palette.selectedIndex = row.index
                            AppController.runCommandPalette(row.modelData.id)
                        }

                        HoverHandler {
                            id: rowHover
                            cursorShape: Qt.PointingHandCursor
                            onHoveredChanged: {
                                if (hovered)
                                    palette.selectedIndex = row.index
                            }
                        }

                        TapHandler {
                            onTapped: {
                                palette.selectedIndex = row.index
                                AppController.runCommandPalette(row.modelData.id)
                            }
                        }

                        Rectangle {
                            id: iconBox
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            width: 32
                            height: 32
                            radius: Theme.radiusSm
                            color: row.selected ? Theme.primaryContainer : Theme.surfaceContainerHighest

                            Icon {
                                anchors.centerIn: parent
                                width: 16
                                height: 16
                                path: (row.modelData.icon && Icons[row.modelData.icon])
                                    ? Icons[row.modelData.icon]
                                    : Icons.sparkle
                                color: row.selected ? Theme.primaryContainerContent : Theme.surfaceContent
                            }
                        }

                        Column {
                            anchors.left: iconBox.right
                            anchors.leftMargin: 12
                            anchors.right: badgePill.visible ? badgePill.left : parent.right
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 1
                            opacity: row.isEnabled ? 1.0 : 0.6

                            Txt {
                                width: parent.width
                                text: row.modelData.title || ""
                                role: "label"
                                size: 14
                                color: row.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                            }

                            Txt {
                                width: parent.width
                                visible: (row.modelData.subtitle || "").length > 0
                                text: row.modelData.subtitle || ""
                                role: "caption"
                                size: 12
                                muted: !row.selected
                                color: row.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                            }
                        }

                        Rectangle {
                            id: badgePill
                            readonly property string badgeText: (row.modelData.badge && row.modelData.badge.length > 0)
                                ? row.modelData.badge
                                : (row.modelData.category || "")
                            visible: badgeText.length > 0
                            anchors.right: parent.right
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: badgeLabel.implicitWidth + 14
                            height: 22
                            radius: Theme.pill(height)
                            color: row.selected ? Theme.surfaceContainerHigh : Theme.surfaceContainerHighest

                            Txt {
                                id: badgeLabel
                                anchors.centerIn: parent
                                text: badgePill.badgeText
                                role: "caption"
                                size: 11
                                muted: true
                            }
                        }
                    }
                }

                Txt {
                    anchors.centerIn: parent
                    visible: palette.items.length === 0
                    text: qsTr("No matching commands")
                    role: "body"
                    muted: true
                }
            }

            Divider { width: parent.width }

            // ---- Footer ----
            Item {
                width: parent.width
                height: 32

                Txt {
                    anchors.left: parent.left
                    anchors.leftMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("↑↓ to navigate · Enter to run · Esc to close")
                    role: "caption"
                    size: 11
                    muted: true
                }

                Txt {
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    text: {
                        const hk = Preferences.commandPaletteHotkey.length > 0
                            ? Preferences.commandPaletteHotkey
                            : "Ctrl+K"
                        if (AppController.commandPaletteOpenMs <= 0)
                            return hk
                        const msText = AppController.commandPaletteOpenMs < 10
                            ? AppController.commandPaletteOpenMs.toFixed(1)
                            : String(Math.round(AppController.commandPaletteOpenMs))
                        return qsTr("%1 · %2 ms").arg(hk).arg(msText)
                    }
                    role: "caption"
                    size: 11
                    muted: true
                }
            }
        }
    }
}
