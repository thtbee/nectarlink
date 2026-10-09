// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A round icon button (or compact icon+text pill) with a hover tooltip and accessible name.
FocusScope {
    id: button
    property string iconPath
    property string text: ""
    property string label
    property bool tonal: false
    property color iconColor: tonal ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
    signal clicked

    readonly property bool hasText: text.length > 0
    readonly property string effectiveTip: label.length > 0 ? label : text

    implicitWidth: hasText ? contentRow.implicitWidth + 24 : 36
    implicitHeight: hasText ? 32 : 36
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38
    Component.onDestruction: tooltipPill.shown = false

    Accessible.role: Accessible.Button
    Accessible.name: hasText ? text : label
    Accessible.description: hasText && label.length > 0 && label !== text ? label : ""
    Accessible.onPressAction: if (enabled) button.clicked()
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            button.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        id: body
        anchors.fill: parent
        radius: Theme.pill(height)
        color: button.tonal ? Theme.secondaryContainer : "transparent"
        border.width: Theme.graphite && button.hasText ? 1 : 0
        border.color: button.tonal ? Theme.surfaceContent : Theme.outlineVariant
        scale: tap.pressed ? Theme.pressScale : 1
        Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.surfaceContent
            opacity: tap.pressed ? 0.12 : (hover.hovered ? 0.07 : 0)
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        }
        Icon {
            visible: !button.hasText
            anchors.centerIn: parent
            width: 18; height: 18
            path: button.iconPath
            color: button.iconColor
        }
        Row {
            id: contentRow
            visible: button.hasText
            anchors.centerIn: parent
            spacing: 6
            Icon {
                visible: button.iconPath.length > 0
                anchors.verticalCenter: parent.verticalCenter
                width: 15; height: 15
                path: button.iconPath
                color: button.iconColor
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: button.text
                role: "label"
                size: Theme.graphite ? 11 : 13
                color: button.tonal ? Theme.secondaryContainerContent : Theme.surfaceContent
            }
        }
    }
    Rectangle {
        anchors.fill: body
        anchors.margins: -3
        radius: body.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(button)
    }

    Timer {
        id: tipTimer
        interval: 450
        running: hover.hovered && !tap.pressed && button.enabled && button.visible && button.effectiveTip.length > 0
        onRunningChanged: if (!running) tooltipPill.shown = false
        onTriggered: {
            tooltipPill.reposition()
            tooltipPill.shown = true
        }
    }

    Rectangle {
        id: tooltipPill
        property bool shown: false
        parent: button.Window.contentItem ? button.Window.contentItem : button
        z: 1000
        visible: opacity > 0 && button.effectiveTip.length > 0
        opacity: shown && hover.hovered && !tap.pressed && button.visible ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        width: Math.min(tipText.implicitWidth + 18, 300)
        height: tipText.implicitHeight + 10
        radius: Theme.graphite ? Theme.radiusXs : Theme.radiusSm
        color: Theme.graphite ? Theme.surface : Theme.surfaceContainerHighest
        border.width: 1
        border.color: Theme.outlineVariant
        onWidthChanged: if (shown) reposition()

        function reposition() {
            const host = parent
            if (!host || host.width <= 0 || host.height <= 0)
                return
            const w = width
            const h = height
            const below = button.mapToItem(host, button.width / 2, button.height + 6)
            const above = button.mapToItem(host, button.width / 2, -h - 6)
            const maxX = Math.max(8, host.width - w - 8)
            x = Math.round(Math.max(8, Math.min(maxX, below.x - w / 2)))
            y = Math.round((below.y + h + 8 <= host.height) ? below.y : Math.max(8, above.y))
        }

        Txt {
            id: tipText
            anchors.centerIn: parent
            width: Math.min(implicitWidth, 282)
            elide: Text.ElideRight
            role: "caption"
            color: Theme.surfaceContent
            text: button.effectiveTip
        }
    }

    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { id: tap; onTapped: button.clicked() }
}
