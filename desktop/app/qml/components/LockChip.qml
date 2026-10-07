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
            return s.target === "notification_access" ? qsTr("Allow notification access on the phone")
                 : s.target === "dnd_access" ? qsTr("Allow Do Not Disturb on the phone")
                 : s.target === "write_settings" ? qsTr("Allow system settings on the phone")
                                                      : qsTr("Allow it on the phone")
        case "enableAddon": return qsTr("Add-on")
        case "enablePath": return qsTr("Away mode")
        case "enableToggle": return qsTr("Turned off")
        case "updateApp": return qsTr("Update app")
        default: return s.state === "unsupported" ? qsTr("Not available") : ""
        }
    }
    readonly property string label: {
        const s = feature || {}
        return shortLabel.length > 0 && s.minutes > 0
            ? qsTr("%1 · ~%2 min").arg(shortLabel).arg(s.minutes)
            : shortLabel
    }
    readonly property string shownLabel: maxWidth > 0 && fullMeasure.implicitWidth + 34 > maxWidth
        ? shortLabel
        : label

    visible: label.length > 0
    implicitHeight: 22
    implicitWidth: textItem.implicitWidth + 34
    width: maxWidth > 0 ? Math.min(implicitWidth, maxWidth) : implicitWidth
    radius: Theme.pill(height)
    color: "transparent"
    border.width: 1
    border.color: Theme.outlineVariant

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
}
