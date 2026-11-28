// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The paired devices in a card by the navigation rail, to pick the one the
// pages show. A click anywhere else, or Escape, closes it.
Item {
    id: menu
    property bool opened: false
    // The device shown (a row of DeviceList).
    property int current: 0
    // What's waiting on each device, by device ID.
    property var badges: ({})
    signal picked(int index)
    signal pairRequested

    function open() {
        opened = true
        card.forceActiveFocus(Qt.PopupFocusReason)
    }
    function close() { opened = false }

    anchors.fill: parent
    visible: opened || fade.running
    z: 95

    Shortcut {
        sequence: "Esc"
        enabled: menu.opened
        onActivated: menu.close()
    }

    MouseArea {
        anchors.fill: parent
        enabled: menu.opened
        onClicked: menu.close()
    }

    GlassShadow {
        anchors.fill: card
        radius: card.radius
        elevation: 1.35
        opacity: card.opacity
        scale: card.scale
        transformOrigin: card.transformOrigin
    }

    Rectangle {
        id: card
        x: Theme.railWidth + 8
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 18
        width: 280
        height: list.contentHeight + pair.height + 16
        radius: Theme.radiusLg
        color: Theme.glass
            ? Qt.rgba(Theme.surfaceContainerHigh.r, Theme.surfaceContainerHigh.g, Theme.surfaceContainerHigh.b, Theme.dark ? 0.84 : 0.90)
            : Theme.surfaceContainerHigh
        border.width: Theme.glass ? 0 : 1
        border.color: Theme.outlineVariant
        opacity: menu.opened ? 1 : 0
        scale: menu.opened || Theme.reduceMotion ? 1 : 0.96
        transformOrigin: Item.BottomLeft
        Behavior on opacity { NumberAnimation { id: fade; duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
        Accessible.role: Accessible.PopupMenu
        Accessible.name: qsTr("Devices")

        GlassRim { radius: card.radius; z: 10 }

        // Clicks on the card are its own.
        MouseArea { anchors.fill: parent }

        ListView {
            id: list
            x: 8; y: 8
            width: parent.width - 16
            height: contentHeight
            interactive: false
            model: DeviceList
            delegate: Item {
                id: row
                required property int index
                required property string deviceId
                required property string name
                required property string kind
                required property string model
                required property bool online
                width: list.width
                height: 52
                activeFocusOnTab: true
                readonly property bool selected: index === menu.current
                Accessible.role: Accessible.MenuItem
                readonly property int waiting: menu.badges[deviceId] || 0
                Accessible.name: name + (online ? "" : ", " + qsTr("offline"))
                    + (waiting > 0 ? ", " + qsTr("%n new", "", waiting) : "")
                Accessible.checked: selected
                Accessible.onPressAction: row.pick()
                function pick() {
                    menu.picked(index)
                    menu.close()
                }
                Keys.onPressed: (event) => {
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                        row.pick()
                        event.accepted = true
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.radiusMd
                    color: row.selected ? Theme.secondaryContainer
                        : (hover.hovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.08) : "transparent")
                    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 180; easing.type: Easing.OutCubic } }
                    border.width: Theme.focusVisible(row) ? 2 : 0
                    border.color: Theme.primary
                }
                Icon {
                    id: kindIcon
                    x: 12
                    anchors.verticalCenter: parent.verticalCenter
                    path: row.kind === "tablet" || row.kind === "phone" ? Icons.phone : Icons.laptop
                    color: row.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                }
                Column {
                    anchors.left: kindIcon.right
                    anchors.leftMargin: 12
                    anchors.right: waitingBadge.visible ? waitingBadge.left : status.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    Txt {
                        width: parent.width
                        text: row.name
                        role: "body"
                        weight: row.selected ? 600 : 500
                        elide: Text.ElideRight
                    }
                    Txt {
                        width: parent.width
                        visible: row.model.length > 0
                        text: row.model
                        role: "bodySmall"
                        muted: true
                        elide: Text.ElideRight
                    }
                }
                Badge {
                    id: waitingBadge
                    anchors.right: status.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    count: row.waiting
                    ring: row.selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh
                }
                StatusDot {
                    id: status
                    anchors.right: parent.right
                    anchors.rightMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    online: row.online
                }
                HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: row.pick() }
            }
        }

        Item {
            id: pair
            anchors.top: list.bottom
            x: 8
            width: parent.width - 16
            height: 44
            activeFocusOnTab: true
            Accessible.role: Accessible.MenuItem
            Accessible.name: qsTr("Pair a new device")
            Accessible.onPressAction: pair.choose()
            function choose() {
                menu.close()
                menu.pairRequested()
            }
            Keys.onPressed: (event) => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                    pair.choose()
                    event.accepted = true
                }
            }
            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusMd
                color: pairHover.hovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.08) : "transparent"
                border.width: Theme.focusVisible(pair) ? 2 : 0
                border.color: Theme.primary
            }
            Icon {
                id: plusIcon
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                path: Icons.plus
                color: Theme.surfaceContentVariant
            }
            Txt {
                anchors.left: plusIcon.right
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Pair a new device")
                role: "body"
            }
            HoverHandler { id: pairHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: pair.choose() }
        }
    }
}
