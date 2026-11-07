// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Shown on a locked feature: what unlocks it and how long it takes, e.g.
// "Elevated · ~2 min". `feature` is AppController.featureState(...).
Rectangle {
    id: chip
    property var feature: ({})
    property real maxWidth: parent ? parent.width : 0

    readonly property string shortLabel: {
        const s = feature || {}
        switch (s.action) {
        case "raisePower": return s.target === "elevated" ? qsTr("Elevated") : qsTr("Assist")
        case "grantPermission":
            return s.target === "notification_access" ? qsTr("Notification access")
                 : s.target === "dnd_access" ? qsTr("DND access")
                 : s.target === "write_settings" ? qsTr("System settings")
                                                       : qsTr("Permission needed")
        case "enableAddon": return qsTr("Add-on")
        case "enablePath": return qsTr("Connect over mobile data")
        case "enableToggle": return qsTr("Off")
        case "updateApp": return qsTr("Update app")
        default: return s.state === "unsupported" ? qsTr("Unavailable") : ""
        }
    }
    readonly property string label: shortLabel
    readonly property string shownLabel: shortLabel

    readonly property string lockKind: {
        const s = feature || {}
        if (s.action === "raisePower")
            return s.target === "elevated" ? "elevated" : "assist"
        if (s.action === "grantRole" || s.target === "role" || s.target === "sms_role" || s.target === "call_screening")
            return "role"
        if (s.action === "grantPermission")
            return "permission"
        return s.action || ""
    }

    property string helpText: {
        const s = feature || {}
        switch (lockKind) {
        case "assist":
            return qsTr("On your phone, open NectarLink › Settings › Permissions › Control from your PC and turn on the Accessibility service.")
        case "elevated":
            return qsTr("Needs Elevated: set it up in NectarLink › Settings › Permissions › Control from your PC, or connect the phone over USB with USB debugging allowed.")
        case "role":
            return qsTr("On your phone, set NectarLink as the default SMS/Caller ID app when prompted.")
        case "permission":
            if (s.target === "notification_access")
                return qsTr("On your phone, open NectarLink and allow Notification access.")
            if (s.target === "dnd_access")
                return qsTr("On your phone, open NectarLink and allow Do Not Disturb access.")
            if (s.target === "write_settings")
                return qsTr("On your phone, open NectarLink and allow Modify system settings.")
            return qsTr("Open NectarLink on your phone and grant this permission in Settings.")
        case "enableAddon":
            return qsTr("Open Settings › Camera & Audio to set up the NectarLink virtual camera add-on.")
        case "enablePath":
            return qsTr("Open Settings › Connection & About and turn on Connect over mobile data.")
        case "enableToggle":
            return qsTr("This feature is turned off for this phone in Settings › Devices.")
        default:
            return label
        }
    }

    visible: label.length > 0
    implicitHeight: 22
    implicitWidth: textItem.implicitWidth + 34
    width: maxWidth > 0 ? Math.min(implicitWidth, maxWidth) : implicitWidth
    radius: Theme.pill(height)
    color: "transparent"
    border.width: 1
    border.color: Theme.outlineVariant
    Component.onDestruction: helpTooltip.shown = false

    Txt {
        id: fullMeasure
        visible: false
        text: chip.label
        role: "caption"
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 5
        Icon {
            anchors.verticalCenter: parent.verticalCenter
            width: 13; height: 13
            path: Icons.lock
            color: Theme.surfaceContentVariant
        }
        Txt {
            id: textItem
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, Math.max(0, chip.width - 34))
            text: chip.shownLabel
            role: "caption"
            muted: true
            elide: Text.ElideRight
        }
    }

    property bool tappedOpen: false

    Timer {
        id: helpTipTimer
        interval: 220
        repeat: false
        running: lockMouse.containsMouse && chip.visible && chip.helpText.length > 0 && !helpTooltip.shown
        onTriggered: {
            if (lockMouse.containsMouse && chip.visible) {
                helpTooltip.reposition()
                helpTooltip.shown = true
                autoHideTimer.interval = 2600
                autoHideTimer.restart()
            }
        }
    }

    Timer {
        id: autoHideTimer
        interval: 2600
        repeat: false
        onTriggered: {
            chip.tappedOpen = false
            helpTooltip.shown = false
        }
    }

    onVisibleChanged: {
        if (!visible) {
            tappedOpen = false
            helpTooltip.shown = false
            autoHideTimer.stop()
        }
    }
    onEnabledChanged: {
        if (!enabled) {
            tappedOpen = false
            helpTooltip.shown = false
            autoHideTimer.stop()
        }
    }

    Rectangle {
        id: helpTooltip
        property bool shown: false
        // Always in the window's top layer: its position is worked out there
        // before it shows (attached to the chip until visible, it came out
        // at the window's top-left corner).
        parent: chip.Window.contentItem ? chip.Window.contentItem : chip
        onParentChanged: if (shown) reposition()
        z: 1000
        visible: opacity > 0 && chip.helpText.length > 0
        opacity: shown && chip.visible && chip.enabled ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
        width: Math.min(helpTipTxt.implicitWidth + 20, 320)
        height: helpTipTxt.implicitHeight + 12
        radius: Theme.graphite ? Theme.radiusXs : Theme.radiusSm
        color: Theme.graphite ? Theme.surface : Theme.surfaceContainerHighest
        border.width: 1
        border.color: Theme.outlineVariant
        onWidthChanged: if (shown) reposition()
        onHeightChanged: if (shown) reposition()

        function reposition() {
            const host = parent
            if (!host || host.width <= 0 || host.height <= 0)
                return
            const w = width
            const h = height
            const below = chip.mapToItem(host, chip.width / 2, chip.height + 6)
            const above = chip.mapToItem(host, chip.width / 2, -h - 6)
            const maxX = Math.max(8, host.width - w - 8)
            x = Math.round(Math.max(8, Math.min(maxX, below.x - w / 2)))
            y = Math.round((below.y + h + 8 <= host.height) ? below.y : Math.max(8, above.y))
        }

        Txt {
            id: helpTipTxt
            anchors.centerIn: parent
            width: Math.min(implicitWidth, 300)
            wrapMode: Text.WordWrap
            role: "caption"
            color: Theme.surfaceContent
            text: chip.helpText
        }

        TapHandler {
            onTapped: {
                chip.tappedOpen = false
                helpTooltip.shown = false
                autoHideTimer.stop()
            }
        }
    }

    MouseArea {
        id: lockMouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onContainsMouseChanged: {
            if (!containsMouse && !chip.tappedOpen) {
                helpTooltip.shown = false
                autoHideTimer.stop()
            }
        }
        onClicked: {
            if (chip.lockKind === "enableAddon") {
                helpTooltip.shown = false
                AppController.triggerHighlight("settings:camera_audio")
                return
            }
            if (chip.lockKind === "enableToggle") {
                helpTooltip.shown = false
                AppController.triggerHighlight("settings:devices")
                return
            }
            if (chip.lockKind === "enablePath") {
                helpTooltip.shown = false
                AppController.triggerHighlight("settings:connection")
                return
            }
            if (chip.helpText.length > 0) {
                if (helpTooltip.shown && chip.tappedOpen) {
                    chip.tappedOpen = false
                    helpTooltip.shown = false
                    autoHideTimer.stop()
                } else {
                    chip.tappedOpen = true
                    helpTooltip.reposition()
                    helpTooltip.shown = true
                    autoHideTimer.interval = 2600
                    autoHideTimer.restart()
                }
            }
        }
    }
}
