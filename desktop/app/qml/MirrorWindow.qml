// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// A phone's screen in a window of its own, sized like the phone.
Window {
    id: win
    readonly property string deviceName: Mirror.name

    visible: Mirror.phase.length > 0
    title: deviceName.length > 0 ? qsTr("%1 · Nectarlink").arg(deviceName) : qsTr("Phone screen · Nectarlink")
    color: "black"
    minimumWidth: 240
    minimumHeight: 240

    // A phone-shaped window the first time a picture arrives (portrait or
    // landscape), within the screen.
    readonly property size frame: video.frameSize
    property bool sized: false
    onFrameChanged: {
        if (frame.width <= 0)
            return
        const maxH = Screen.desktopAvailableHeight * 0.85
        const maxW = Screen.desktopAvailableWidth * 0.85
        const scale = Math.min(maxH / frame.height, maxW / frame.width, 1)
        if (!sized || Math.abs(width / height - frame.width / frame.height) > 0.05) {
            width = Math.round(frame.width * scale)
            height = Math.round(frame.height * scale)
            sized = true
        }
    }
    onVisibleChanged: {
        if (visible) {
            sized = false
            width = 420
            height = 860
        }
    }
    onClosing: Mirror.stop()

    VideoView {
        id: video
        anchors.fill: parent
        stream: Mirror.device
    }

    // Before the picture: waiting for the phone, or why it ended.
    Rectangle {
        anchors.fill: parent
        visible: Mirror.phase !== "showing" || video.frameSize.width <= 0
        color: Theme.surface
        Column {
            anchors.centerIn: parent
            width: Math.min(parent.width - 48, 320)
            spacing: 14
            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: Mirror.phase !== "ended"
                width: 28; height: 28
            }
            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: Mirror.phase === "ended"
                width: 32; height: 32
                path: Icons.mirror
                color: Theme.surfaceContentVariant
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "title"
                text: Mirror.phase === "asking" ? qsTr("Allow it on your phone")
                    : Mirror.phase === "ended" ? qsTr("Mirroring stopped")
                    : qsTr("Starting…")
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: Mirror.phase === "asking"
                      ? qsTr("Tap the notification on %1, then choose to share the entire screen.").arg(win.deviceName)
                      : Mirror.phase === "ended" ? (Mirror.reason.length > 0 ? Mirror.reason : qsTr("The phone stopped sharing its screen."))
                      : ""
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: Mirror.phase === "ended"
                variant: "tonal"
                text: qsTr("Try again")
                onClicked: Mirror.start(Mirror.device)
            }
        }
    }
}
