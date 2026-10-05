// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Shown on a locked feature: what unlocks it and how long it takes, e.g.
// "Elevated · ~2 min". `feature` is AppController.featureState(...).
Rectangle {
    id: chip
    property var feature: ({})

    readonly property string label: {
        const s = feature || {}
        let what = ""
        switch (s.action) {
        case "raisePower": what = s.target === "elevated" ? qsTr("Elevated") : qsTr("Assist"); break
        case "grantPermission": what = qsTr("Permission"); break
        case "enableAddon": what = qsTr("Add-on"); break
        case "enablePath": what = qsTr("Away mode"); break
        case "enableToggle": what = qsTr("Turned off"); break
        case "updateApp": what = qsTr("Update app"); break
        default: what = s.state === "unsupported" ? qsTr("Not available") : ""
        }
        return s.minutes > 0 ? qsTr("%1 · ~%2 min").arg(what).arg(s.minutes) : what
    }

    visible: label.length > 0
    implicitHeight: 22
    implicitWidth: row.implicitWidth + 16
    radius: Theme.pill(height)
    color: "transparent"
    border.width: 1
    border.color: Theme.outlineVariant

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
            anchors.verticalCenter: parent.verticalCenter
            text: chip.label
            role: "caption"
            muted: true
        }
    }
}
