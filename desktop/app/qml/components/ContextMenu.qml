// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A small menu over its parent (declare it last, filling the page), opened at
// a point with `openAt(source, x, y, entries)`. Entries are
// `{ key, label, icon, danger }`; picking one closes the menu and sends `chosen(key)`.
// Escape, a click outside or picking one closes it.
Item {
    id: menu
    property var entries: []
    property bool opened: false
    // Where it was opened, in this item's own coordinates.
    property point origin: Qt.point(0, 0)
    signal chosen(string key)

    function openAt(source, x, y, list) {
        entries = list
        origin = source.mapToItem(menu, x, y)
        opened = true
        card.forceActiveFocus(Qt.PopupFocusReason)
    }
    function close() {
        opened = false
    }

    anchors.fill: parent
    z: 70
    visible: opened

    Shortcut {
        sequence: "Esc"
        enabled: menu.opened
        onActivated: menu.close()
    }
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onPressed: menu.close()
        onWheel: (wheel) => wheel.accepted = false
    }
    // A press goes through a MouseArea to the pointer handlers of the items below, so a click
    // on an entry was also a click on, say, the picture under it, whose own handler opened it.
    // Taking the press for the menu cancels theirs.
    TapHandler {
        acceptedButtons: Qt.AllButtons
        gesturePolicy: TapHandler.WithinBounds
    }

    Rectangle {
        id: card
        // Below and right of the point, moved back inside the page where there's no room.
        x: Math.max(8, Math.min(menu.origin.x, menu.width - width - 8))
        y: Math.max(8, Math.min(menu.origin.y, menu.height - height - 8))
        width: 260
        height: column.height + 10
        radius: Theme.radiusMd
        color: Theme.graphite ? Theme.surface : Theme.surfaceContainerHighest
        border.width: 1
        border.color: Theme.outlineVariant
        Accessible.role: Accessible.PopupMenu
        Accessible.name: qsTr("Actions")
        // Clicks on the card are its own.
        MouseArea { anchors.fill: parent }

        Column {
            id: column
            x: 5
            y: 5
            width: parent.width - 10
            spacing: 2
            Repeater {
                model: menu.entries
                delegate: Rectangle {
                    id: entry
                    required property var modelData
                    function choose() {
                        menu.close()
                        menu.chosen(modelData.key)
                    }
                    width: column.width
                    height: 34
                    radius: Theme.radiusSm
                    activeFocusOnTab: true
                    color: hover.hovered ? Theme.surfaceContainerHigh : "transparent"
                    border.width: Theme.focusVisible(entry) ? 2 : 0
                    border.color: Theme.primary
                    Accessible.role: Accessible.MenuItem
                    Accessible.name: modelData.label
                    Accessible.onPressAction: entry.choose()
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            entry.choose()
                            event.accepted = true
                        }
                    }
                    Row {
                        x: 10
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 10
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 16; height: 16
                            path: entry.modelData.icon || ""
                            color: entry.modelData.danger ? Theme.error : Theme.surfaceContentVariant
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "bodySmall"
                            color: entry.modelData.danger ? Theme.error : Theme.surfaceContent
                            text: entry.modelData.label
                        }
                    }
                    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: entry.choose() }
                }
            }
        }
    }
}
