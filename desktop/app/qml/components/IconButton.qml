// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A round icon button (or compact icon+text pill) with a hover tooltip and accessible name.
FocusScope {
    id: button
    property string iconPath
    property int iconSize: hasText ? 16 : 18
    property string text: ""
    property string label
    property bool tonal: false
    // A small dot at the top right: something new behind this button.
    property bool dot: false
    property color iconColor: tonal ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
    signal clicked

    readonly property bool hasText: text.length > 0
    readonly property string effectiveTip: label.length > 0 ? label : text

    implicitWidth: hasText ? contentRow.implicitWidth + 24 : 34
    implicitHeight: 34
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38
    Component.onDestruction: tooltipPill.shown = false
    onVisibleChanged: if (!visible) { tooltipPill.shown = false; tipAutoHide.stop() }
    onEnabledChanged: if (!enabled) { tooltipPill.shown = false; tipAutoHide.stop() }

    Accessible.role: Accessible.Button
    Accessible.name: hasText ? text : label
    Accessible.description: dot ? qsTr("New changes")
        : (hasText && label.length > 0 && label !== text ? label : "")
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
        color: button.tonal
            ? (Theme.glass
                ? Qt.tint(Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.54), Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.18 : 0.14))
                : Theme.secondaryContainer)
            : "transparent"
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        border.width: (Theme.graphite && button.hasText) || (Theme.glass && button.tonal) ? 1 : 0
        border.color: Theme.glass
            ? Qt.rgba(1, 1, 1, Theme.dark ? 0.12 : 0.38)
            : (button.tonal ? Theme.surfaceContent : Theme.outlineVariant)
        scale: mouse.pressed ? Theme.pressScale : 1
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: Theme.surfaceContent
            opacity: mouse.pressed ? 0.14 : (mouse.containsMouse ? 0.08 : 0)
            Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
        }
        Icon {
            visible: !button.hasText
            anchors.centerIn: parent
            width: button.iconSize; height: button.iconSize
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
                width: button.iconSize; height: button.iconSize
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
        width: 6
        height: 6
        radius: 3
        x: body.width - 11
        y: 7
        color: Theme.primary
        visible: opacity > 0
        opacity: button.dot ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
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
        repeat: false
        running: mouse.containsMouse && !mouse.pressed && button.enabled && button.visible && button.effectiveTip.length > 0 && !tooltipPill.shown
        onTriggered: {
            if (mouse.containsMouse && button.visible) {
                tooltipPill.reposition()
                tooltipPill.shown = true
                tipAutoHide.restart()
            }
        }
    }

    Timer {
        id: tipAutoHide
        interval: 2400
        repeat: false
        onTriggered: tooltipPill.shown = false
    }

    Rectangle {
        id: tooltipPill
        property bool shown: false
        // Always in the window's top layer: its position is worked out there
        // before it shows (attached to the button until visible, it came out
        // at the window's top-left corner).
        parent: button.Window.contentItem ? button.Window.contentItem : button
        onParentChanged: if (shown) reposition()
        z: 1000
        visible: opacity > 0 && button.effectiveTip.length > 0
        opacity: shown && mouse.containsMouse && !mouse.pressed && button.visible && button.enabled ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
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

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onContainsMouseChanged: if (!containsMouse) tooltipPill.shown = false
        onClicked: {
            tooltipPill.shown = false
            button.clicked()
        }
    }
}
