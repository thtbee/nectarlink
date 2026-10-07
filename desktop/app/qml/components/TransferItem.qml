// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// One file transfer: direction, name, progress and speed, and what can be
// done with it (cancel while it runs; open or show what arrived).
Item {
    id: item
    required property string transferId
    required property string deviceName
    required property bool incoming
    required property bool recording
    required property string title
    required property real total
    required property real done
    required property real progress
    required property real rate
    required property string status
    required property string reason
    required property string savedPath

    readonly property bool active: status === "waiting" || status === "running"

    implicitHeight: column.height + 20

    function size(bytes) {
        if (bytes < 1024) return qsTr("%1 B").arg(Math.round(bytes))
        const units = ["KB", "MB", "GB", "TB"]
        let value = bytes / 1024
        let unit = 0
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++ }
        return (value < 10 ? value.toFixed(1) : Math.round(value)) + " " + units[unit]
    }

    function detail() {
        switch (status) {
        case "waiting":
            return item.incoming ? qsTr("Paused; continues when %1 is back").arg(item.deviceName)
                                 : qsTr("Waiting for %1").arg(item.deviceName)
        case "running":
            return qsTr("%1 of %2").arg(size(done)).arg(size(total))
                   + (rate > 0 ? " · " + qsTr("%1/s").arg(size(rate)) : "")
        case "done":
            return item.incoming ? qsTr("From %1 · %2").arg(item.deviceName).arg(size(total))
                                 : qsTr("Sent to %1 · %2").arg(item.deviceName).arg(size(total))
        case "cancelled":
            return qsTr("Cancelled")
        default:
            switch (reason) {
            case "denied": return qsTr("%1 doesn't accept files from this PC").arg(item.deviceName)
            case "unreachable": return qsTr("Couldn't reach %1").arg(item.deviceName)
            case "noSpace": return qsTr("Not enough space on %1").arg(item.deviceName)
            default: return qsTr("Didn't go through")
            }
        }
    }

    Row {
        id: row
        x: 4; y: 10
        width: parent.width - 8
        spacing: 12

        Rectangle {
            width: 32; height: 32
            radius: Theme.graphite ? Theme.radiusSm : 16
            color: Theme.graphite ? "transparent" : Theme.secondaryContainer
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.outlineVariant
            Icon {
                anchors.centerIn: parent
                width: 18; height: 18
                path: item.incoming ? (item.recording ? Icons.mic : Icons.folder) : Icons.send
                color: Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
            }
        }

        Column {
            id: column
            width: row.width - 32 - actions.width - row.spacing * 2
            spacing: 6
            Txt { width: parent.width; role: "title"; size: 14; text: item.title }
            // Progress, while there's progress to show.
            Rectangle {
                width: parent.width
                height: 4
                radius: 2
                visible: item.active
                color: Theme.surfaceContainerHighest
                Rectangle {
                    height: parent.height
                    radius: parent.radius
                    width: parent.width * Math.max(0, Math.min(1, item.progress))
                    color: Theme.primary
                    opacity: item.status === "waiting" ? 0.45 : 1
                    Behavior on width { NumberAnimation { duration: Theme.reduceMotion ? 0 : 180 } }
                }
            }
            Txt {
                width: parent.width
                role: "bodySmall"
                muted: true
                color: item.status === "failed" ? Theme.error : Theme.surfaceContentVariant
                text: item.detail()
            }
        }

        Row {
            id: actions
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4
            Button {
                visible: item.status === "done" && item.incoming && item.savedPath.length > 0
                variant: "text"
                size: "sm"
                text: qsTr("Open")
                onClicked: TransferList.open(item.transferId)
            }
            IconButton {
                visible: item.status === "done" && item.incoming && item.savedPath.length > 0
                iconPath: Icons.folder
                label: qsTr("Show in folder")
                onClicked: TransferList.showInFolder(item.transferId)
            }
            IconButton {
                visible: item.active
                iconPath: Icons.close
                label: qsTr("Cancel")
                onClicked: TransferList.cancel(item.transferId)
            }
        }
    }
}
