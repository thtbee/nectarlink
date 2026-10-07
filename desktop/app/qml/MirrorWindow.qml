// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// A phone's screen, or one of its apps, in a window of its own, sized like
// the phone. One per mirroring (`mirrorKey`); closing it ends that one.
Window {
    id: win
    required property string mirrorKey
    // This window's mirroring (see Mirror.windows).
    readonly property var info: {
        try {
            return JSON.parse(Mirror.windows).find(w => w.key === mirrorKey) || ({})
        } catch (e) {
            return ({})
        }
    }
    readonly property string deviceName: info.name || ""
    readonly property string phase: info.phase || ""
    readonly property bool canControl: info.canControl === true

    visible: true
    title: info.app ? qsTr("%1 · %2").arg(info.title).arg(deviceName)
         : deviceName.length > 0 ? qsTr("%1 · Nectarlink").arg(deviceName) : qsTr("Phone screen · Nectarlink")
    color: "black"
    minimumWidth: 240
    minimumHeight: 240

    // A phone-shaped window the first time a picture arrives (portrait or
    // landscape), within the screen, or restored to where the user left it.
    readonly property size frame: video.frameSize
    property bool sized: false
    property bool readyForResize: false

    Timer {
        id: resizeTimer
        interval: 300
        repeat: false
        onTriggered: {
            if (!win.info.app || !win.sized)
                return
            Mirror.saveGeometry(win.mirrorKey, win.x, win.y, win.width, win.height)
            if (win.phase === "showing")
                Mirror.resize(win.mirrorKey, win.width, win.height)
        }
    }

    Timer {
        id: moveTimer
        interval: 300
        repeat: false
        onTriggered: {
            if (win.info.app && win.sized)
                Mirror.saveGeometry(win.mirrorKey, win.x, win.y, win.width, win.height)
        }
    }

    onWidthChanged: if (info.app && sized && readyForResize) resizeTimer.restart()
    onHeightChanged: if (info.app && sized && readyForResize) resizeTimer.restart()
    onXChanged: if (info.app && sized && readyForResize) moveTimer.restart()
    onYChanged: if (info.app && sized && readyForResize) moveTimer.restart()
    onPhaseChanged: if (phase === "showing" && info.app && sized) resizeTimer.restart()

    onFrameChanged: {
        if (frame.width <= 0)
            return
        const availW = Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920
        const availH = Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : 1080
        if (!sized && info.app && (info.savedWidth || 0) >= 240 && (info.savedHeight || 0) >= 240) {
            readyForResize = false
            const w = Math.max(240, Math.min(Math.round(availW * 0.95), info.savedWidth))
            const h = Math.max(240, Math.min(Math.round(availH * 0.95), info.savedHeight))
            width = w
            height = h
            if (info.savedX !== null && info.savedX !== undefined && info.savedY !== null && info.savedY !== undefined) {
                x = Math.max(0, Math.min(availW - w, info.savedX))
                y = Math.max(0, Math.min(availH - h, info.savedY))
            }
            sized = true
            Qt.callLater(() => { win.readyForResize = true })
            return
        }
        const maxH = availH * 0.85
        const maxW = availW * 0.85
        const scale = Math.min(maxH / frame.height, maxW / frame.width, 1)
        if (!sized || (!info.app && Math.abs(width / height - frame.width / frame.height) > 0.05)) {
            readyForResize = false
            width = Math.round(frame.width * scale)
            height = Math.round(frame.height * scale)
            sized = true
            if (info.app)
                Mirror.saveGeometry(mirrorKey, x, y, width, height)
            Qt.callLater(() => { win.readyForResize = true })
        }
    }
    width: 420
    height: 860
    onClosing: {
        if (info.app && sized)
            Mirror.saveGeometry(mirrorKey, x, y, width, height)
        Mirror.stop(mirrorKey)
    }

    VideoView {
        id: video
        anchors.fill: parent
        stream: win.mirrorKey
        windowIcon: win.info.icon || ""
        windowAppId: win.info.app && (win.info.pkg || "").length > 0
            ? "Nectarlink.App." + (win.info.device || "").slice(0, 8) + "." + win.info.pkg
            : ""
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
            enabled: win.canControl && win.phase === "showing"
            acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
            cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            property bool down: false
            property double lastMove: 0
            onPressed: (mouse) => {
                video.forceActiveFocus()
                const p = video.at(mouse.x, mouse.y)
                if (mouse.button === Qt.RightButton) { Mirror.press(win.mirrorKey, "back"); return }
                if (mouse.button === Qt.MiddleButton) { Mirror.press(win.mirrorKey, "home"); return }
                if (!p.inside) return
                down = true
                Mirror.touch(win.mirrorKey, "down", p.x, p.y)
            }
            onPositionChanged: (mouse) => {
                if (!down) return
                // About 60 a second is plenty for a finger.
                const now = Date.now()
                if (now - lastMove < 16) return
                lastMove = now
                const p = video.at(mouse.x, mouse.y)
                Mirror.touch(win.mirrorKey, "move", p.x, p.y)
            }
            onReleased: (mouse) => {
                if (!down) return
                down = false
                const p = video.at(mouse.x, mouse.y)
                Mirror.touch(win.mirrorKey, "up", p.x, p.y)
            }
            onWheel: (wheel) => {
                const p = video.at(wheel.x, wheel.y)
                if (p.inside)
                    Mirror.scroll(win.mirrorKey, p.x, p.y, -wheel.angleDelta.x / 120, -wheel.angleDelta.y / 120)
            }
        }

        // The keyboard: text goes into the phone's text field; a few keys
        // have phone meanings.
        Keys.onPressed: (event) => {
            if (!win.canControl || win.phase !== "showing")
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
                Mirror.paste(win.mirrorKey)
                event.accepted = true
            } else if (keys[event.key] !== undefined) {
                Mirror.press(win.mirrorKey, keys[event.key])
                event.accepted = true
            } else if (event.text.length > 0 && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier))
                       && event.text.charCodeAt(0) >= 32) {
                Mirror.text(win.mirrorKey, event.text)
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
        visible: win.info.sound === true && win.phase === "showing"
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
        visible: win.phase === "showing" && video.frameSize.width > 0 && !win.canControl && !win.info.app && !hintClose.closed
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
        visible: win.phase !== "showing" || video.frameSize.width <= 0
        color: Theme.surface
        Column {
            anchors.centerIn: parent
            width: Math.min(parent.width - 48, 320)
            spacing: 14
            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: win.phase !== "ended"
                width: 28; height: 28
            }
            RoundedImage {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: win.phase === "ended" && win.info.app === true && (win.info.icon || "").length > 0
                width: 40; height: 40
                radius: 10
                source: win.info.icon || ""
            }
            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: win.phase === "ended" && !(win.info.app === true && (win.info.icon || "").length > 0)
                width: 32; height: 32
                path: Icons.mirror
                color: Theme.surfaceContentVariant
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "title"
                text: win.phase === "asking" ? (win.info.app ? qsTr("Opening %1").arg(win.info.title) : qsTr("Allow it on your phone"))
                    : win.phase === "ended" ? (win.info.app ? qsTr("%1 closed").arg(win.info.title) : qsTr("Mirroring stopped"))
                    : qsTr("Starting…")
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: win.phase === "asking" && !win.info.app
                      ? qsTr("Tap the notification on %1, then choose to share the entire screen.").arg(win.deviceName)
                      : win.phase === "ended" ? ((win.info.reason || "").length > 0 ? win.info.reason
                                                : win.info.app ? qsTr("The app closed on %1.").arg(win.deviceName)
                                                : qsTr("The phone stopped sharing its screen."))
                      : ""
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: win.phase === "ended"
                variant: "tonal"
                text: win.info.app ? qsTr("Open again") : qsTr("Try again")
                onClicked: Mirror.reopen(win.mirrorKey)
            }
        }
    }
}
