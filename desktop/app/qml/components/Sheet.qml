// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A modal dialog over the window: a scrim and a card that springs in.
// Escape or a click on the scrim closes it (unless `dismissable` is false).
Item {
    id: sheet
    property bool opened: false
    property bool dismissable: true
    property real cardWidth: 460
    default property alias content: body.data
    signal dismissed

    function open() {
        opened = true
        if (!sheet.activeFocus)
            sheet.forceActiveFocus(Qt.OtherFocusReason)
    }
    function close() { opened = false }

    anchors.fill: parent
    visible: opened || fade.running
    z: 100

    Shortcut {
        sequence: "Esc"
        enabled: sheet.opened && sheet.dismissable
        onActivated: {
            sheet.close()
            sheet.dismissed()
        }
    }

    MouseArea {
        anchors.fill: parent
        enabled: sheet.opened
        acceptedButtons: Qt.NoButton
        hoverEnabled: true
        onWheel: (wheel) => wheel.accepted = true
    }

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Theme.scrim
        opacity: sheet.opened ? 1 : 0
        Behavior on opacity { NumberAnimation { id: fade; duration: Theme.fadeNormal } }
        // Only taps outside the card: a tap on the card (or a button in it)
        // reaches this handler too.
        TapHandler {
            onTapped: (eventPoint) => {
                const p = card.mapFromItem(scrim, eventPoint.position)
                const inside = p.x >= 0 && p.y >= 0 && p.x <= card.width && p.y <= card.height
                if (!inside && sheet.dismissable) { sheet.close(); sheet.dismissed() }
            }
        }
    }

    GlassShadow {
        anchors.fill: card
        radius: card.radius
        elevation: 1.50
        opacity: card.opacity
        scale: card.scale
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(sheet.cardWidth, parent.width - 48)
        height: Math.min(body.childrenRect.height + 48, parent.height - 48)
        radius: Theme.radiusXl
        color: Theme.glass
            ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.88 : 0.92)
            : Theme.surface
        border.width: Theme.glass ? 0 : 1
        border.color: Theme.outlineVariant
        opacity: sheet.opened ? 1 : 0
        scale: sheet.opened || Theme.reduceMotion ? 1 : 0.95
        Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic } }
        Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springStandard; damping: Theme.dampingStandard } }
        clip: true

        GlassRim { radius: card.radius; z: 10 }

        ScrollRail { flick: sheetFlick }
        Flickable {
            id: sheetFlick
            anchors.fill: parent
            anchors.margins: 24
            contentWidth: width
            contentHeight: body.childrenRect.height
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            Item {
                id: body
                width: parent.width
                height: childrenRect.height
            }
        }
    }

    focus: opened
    Keys.onEscapePressed: if (sheet.dismissable) { sheet.close(); sheet.dismissed() }
}
