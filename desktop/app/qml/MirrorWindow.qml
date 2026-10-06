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
        focus: true

        // Where on the phone's screen a point in the window is (0 to 1).
        function at(x, y) {
            const r = pictureRect()
            return { x: (x - r.x) / Math.max(1, r.width), y: (y - r.y) / Math.max(1, r.height),
                     inside: x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height }
        }

        // The mouse: left is a finger, right is Back, middle is Home.
        MouseArea {
            anchors.fill: parent
            enabled: Mirror.canControl && Mirror.phase === "showing"
            acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            property bool down: false
            property double lastMove: 0
            onPressed: (mouse) => {
                video.forceActiveFocus()
                const p = video.at(mouse.x, mouse.y)
                if (mouse.button === Qt.RightButton) { Mirror.key("back"); return }
                if (mouse.button === Qt.MiddleButton) { Mirror.key("home"); return }
                if (!p.inside) return
                down = true
                Mirror.touch("down", p.x, p.y)
            }
            onPositionChanged: (mouse) => {
                if (!down) return
                // About 60 a second is plenty for a finger.
                const now = Date.now()
                if (now - lastMove < 16) return
                lastMove = now
                const p = video.at(mouse.x, mouse.y)
                Mirror.touch("move", p.x, p.y)
            }
            onReleased: (mouse) => {
                if (!down) return
                down = false
                const p = video.at(mouse.x, mouse.y)
                Mirror.touch("up", p.x, p.y)
            }
            onWheel: (wheel) => {
                const p = video.at(wheel.x, wheel.y)
                if (p.inside)
                    Mirror.scroll(p.x, p.y, -wheel.angleDelta.x / 120, -wheel.angleDelta.y / 120)
            }
        }

        // The keyboard: text goes into the phone's text field; a few keys
        // have phone meanings.
        Keys.onPressed: (event) => {
            if (!Mirror.canControl || Mirror.phase !== "showing")
                return
            const keys = {}
            keys[Qt.Key_Escape] = "back"
            keys[Qt.Key_Back] = "back"
            keys[Qt.Key_Home] = "home"
            keys[Qt.Key_Return] = "enter"
            keys[Qt.Key_Enter] = "enter"
            keys[Qt.Key_Backspace] = "backspace"
            keys[Qt.Key_Delete] = "delete"
            keys[Qt.Key_Left] = "left"
            keys[Qt.Key_Right] = "right"
            keys[Qt.Key_Up] = "up"
            keys[Qt.Key_Down] = "down"
            keys[Qt.Key_Tab] = "tab"
            if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V) {
                Mirror.paste()
                event.accepted = true
            } else if (keys[event.key] !== undefined) {
                Mirror.key(keys[event.key])
                event.accepted = true
            } else if (event.text.length > 0 && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier))
                       && event.text.charCodeAt(0) >= 32) {
                Mirror.text(event.text)
                event.accepted = true
            }
        }
    }

    // The phone's sound on this PC: on or off, shown while the mouse is
    // over the window (and while it's off).
    HoverHandler { id: hovering }
    Rectangle {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 12
        width: 40; height: 40
        radius: 20
        color: Qt.rgba(0, 0, 0, 0.6)
        visible: Mirror.sound && Mirror.phase === "showing"
        opacity: hovering.hovered || Mirror.muted ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        IconButton {
            anchors.centerIn: parent
            iconPath: Mirror.muted ? Icons.soundOff : Icons.speaker
            iconColor: "white"
            label: Mirror.muted ? qsTr("Play the phone's sound here") : qsTr("Mute the phone's sound here")
            onClicked: Mirror.toggleSound()
        }
    }

    // How to control it, until the phone allows it.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 12
        height: hint.implicitHeight + 20
        radius: Theme.radiusMd
        visible: Mirror.phase === "showing" && video.frameSize.width > 0 && !Mirror.canControl && !hintClose.closed
        color: Qt.rgba(0, 0, 0, 0.72)
        Txt {
            id: hint
            anchors.left: parent.left
            anchors.right: hintClose.left
            anchors.margins: 10
            anchors.verticalCenter: parent.verticalCenter
            wrapMode: Text.WordWrap
            role: "bodySmall"
            color: "white"
            text: qsTr("To use your mouse and keyboard on the phone, open Nectarlink on the phone and turn on Control from your PC in Settings.")
        }
        IconButton {
            id: hintClose
            property bool closed: false
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            iconPath: Icons.close
            iconColor: "white"
            label: qsTr("Hide")
            onClicked: closed = true
        }
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
