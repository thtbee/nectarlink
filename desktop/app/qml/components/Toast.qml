// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A short message at the bottom of the window that hides itself.
Rectangle {
    id: toast
    property int timeout: 4000

    function show(message) {
        label.text = message
        shown = true
        timer.restart()
    }

    property bool shown: false
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: shown ? 24 : -height
    Behavior on anchors.bottomMargin { SpringAnimation { spring: Theme.springStandard; damping: Theme.dampingStandard } }
    opacity: shown ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    z: 200

    width: Math.min(label.implicitWidth + 40, parent.width - 48)
    height: 44
    radius: Theme.pill(height)
    color: Theme.surfaceContent

    Accessible.role: Accessible.AlertMessage
    Accessible.name: label.text

    Txt {
        id: label
        anchors.centerIn: parent
        width: parent.width - 40
        horizontalAlignment: Text.AlignHCenter
        role: "body"
        color: Theme.surface
    }
    Timer { id: timer; interval: toast.timeout; onTriggered: toast.shown = false }
}
