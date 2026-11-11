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
         : deviceName.length > 0 ? qsTr("%1 · NectarLink").arg(deviceName) : qsTr("Phone screen · NectarLink")
    color: "black"
    minimumWidth: 240
    minimumHeight: 240
    flags: Qt.Window

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
    onPhaseChanged: {
        if (phase === "showing" && info.app && sized)
            resizeTimer.restart()
        // Typing goes to the picture again, not to the hidden Try again button.
        if (phase === "showing")
            video.forceActiveFocus()
    }
    WindowPlacement {
        id: windowPlacement
    }
    function placeOnScreen(px, py, w, h) {
        const spot = windowPlacement.spot(px, py, w, h, Qt.application.screens, Screen)
        x = spot.x
        y = spot.y
    }

    onActiveChanged: if (win.active) Mirror.syncClipboardOnFocus(win.mirrorKey)

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
                placeOnScreen(info.savedX, info.savedY, w, h)
            }
            sized = true
            Qt.callLater(() => { win.readyForResize = true })
            return
        }
        // An app that only runs one way up turned the phone's display (#126):
        // the window takes the app's shape, and its usual resize tells the
        // phone the matching size.
        if (sized && info.app && (frame.width > frame.height) !== (width > height)) {
            const turnedW = Math.max(240, Math.min(Math.round(availW * 0.95), height))
            const turnedH = Math.max(240, Math.min(Math.round(availH * 0.95), width))
            width = turnedW
            height = turnedH
            placeOnScreen(x, y, turnedW, turnedH)
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
    readonly property bool staysOnTop: win.flags & Qt.WindowStaysOnTopHint

    function toggleStayOnTop() {
        win.flags = win.staysOnTop
            ? Qt.Window
            : Qt.WindowStaysOnTopHint | Qt.Window
    }

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
        acceptsText: win.phase === "showing" && win.canControl
        // The emoji panel (Win+.) and input methods give finished text here
        // instead of as key presses.
        onTextCommitted: (text) => Mirror.text(win.mirrorKey, text)

        // Where on the phone's screen a point in the window is (0 to 1).
        function at(x, y) {
            const r = pictureRect()
            return { x: (x - r.x) / Math.max(1, r.width), y: (y - r.y) / Math.max(1, r.height),
                     inside: x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height }
        }

        DropArea {
            id: mirrorDropArea
            anchors.fill: parent
            enabled: win.phase === "showing"
            onEntered: (drag) => {
                Mirror.syncClipboardOnFocus(win.mirrorKey)
            }
            onDropped: (drop) => {
                const urls = []
                if (drop.urls) {
                    for (let i = 0; i < drop.urls.length; i++) {
                        urls.push(drop.urls[i].toString())
                    }
                }
                if (urls.length > 0) {
                    Mirror.syncClipboardOnFocus(win.mirrorKey)
                    Mirror.dropFiles(urls)
                }
                drop.acceptProposedAction()
            }
        }

        HoverHandler {
            id: videoHover
            onHoveredChanged: {
                if (hovered) {
                    toolbarWakeTimer.restart()
                    if (win.phase === "showing")
                        Mirror.enterWindow(win.mirrorKey)
                } else {
                    Mirror.leaveWindow()
                }
            }
        }

        // The mouse: left is a finger, right is Back, middle is Home.
        MouseArea {
            anchors.fill: parent
            enabled: win.phase === "showing"
            acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
            cursorShape: win.canControl ? Qt.PointingHandCursor : Qt.ArrowCursor
            property bool down: false
            property double lastMove: 0
            onPressed: (mouse) => {
                video.forceActiveFocus()
                if (!win.canControl) {
                    hintClose.closed = false
                    hintPulse.restart()
                    Mirror.requestControl(win.mirrorKey)
                    return
                }
                const p = video.at(mouse.x, mouse.y)
                if (mouse.button === Qt.RightButton) { Mirror.press(win.mirrorKey, "back"); return }
                if (mouse.button === Qt.MiddleButton) { Mirror.press(win.mirrorKey, "home"); return }
                if (!p.inside) return
                down = true
                Mirror.touch(win.mirrorKey, "down", p.x, p.y)
            }
            onPositionChanged: (mouse) => {
                if (!down || !win.canControl) return
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
                if (!win.canControl) return
                const p = video.at(mouse.x, mouse.y)
                Mirror.touch(win.mirrorKey, "up", p.x, p.y)
            }
            onWheel: (wheel) => {
                if (!win.canControl) {
                    hintClose.closed = false
                    Mirror.requestControl(win.mirrorKey)
                    return
                }
                const p = video.at(wheel.x, wheel.y)
                if (p.inside)
                    Mirror.scroll(win.mirrorKey, p.x, p.y, -wheel.angleDelta.x / 120, -wheel.angleDelta.y / 120)
            }
        }

        // The keyboard: shortcuts for screenshots and recording work even
        // without remote control; text and navigation keys go to the phone.
        Keys.onPressed: (event) => {
            if (win.phase !== "showing")
                return
            if ((event.modifiers & Qt.ControlModifier) && (event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_C) {
                Mirror.screenshotClipboard(win.mirrorKey)
                event.accepted = true
                return
            }
            if ((event.modifiers & Qt.ControlModifier) && !(event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_S) {
                Mirror.screenshotFile(win.mirrorKey)
                event.accepted = true
                return
            }
            if ((event.modifiers & Qt.ControlModifier) && !(event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_R) {
                Mirror.toggleRecording(win.mirrorKey)
                event.accepted = true
                return
            }
            if (!win.canControl)
                return
            if (Preferences.enterSends && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) {
                // A plain Enter presses the app's Send; with a modifier, a new line.
                if (event.modifiers & (Qt.ControlModifier | Qt.ShiftModifier))
                    Mirror.text(win.mirrorKey, "\n")
                else
                    Mirror.press(win.mirrorKey, "send")
                event.accepted = true
                return
            }
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
            // Copy, cut and select all on the phone; what's copied there
            // comes back with the clipboard.
            const edits = {}
            edits[Qt.Key_C] = "copy"
            edits[Qt.Key_X] = "cut"
            edits[Qt.Key_A] = "select_all"
            const ctrlOnly = (event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.ShiftModifier | Qt.MetaModifier))
                === Qt.ControlModifier
            if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V) {
                Mirror.paste(win.mirrorKey)
                event.accepted = true
            } else if (ctrlOnly && edits[event.key] !== undefined) {
                Mirror.press(win.mirrorKey, edits[event.key])
                event.accepted = true
            } else if (keys[event.key] !== undefined) {
                Mirror.press(win.mirrorKey, keys[event.key])
                event.accepted = true
            } else if (event.text.length > 0 && event.text.charCodeAt(0) >= 32
                       && (!(event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))
                           // AltGr (@ on a French keyboard, say) arrives as Ctrl+Alt on Windows.
                           // A plain letter, digit or space with Ctrl+Alt is a shortcut:
                           // Qt gives Ctrl+Alt+2 the text "2" even without AltGr.
                           || ((event.modifiers & Qt.ControlModifier) && (event.modifiers & Qt.AltModifier)
                               && !/^[A-Za-z0-9 ]$/.test(event.text)))) {
                Mirror.text(win.mirrorKey, event.text)
                event.accepted = true
            }
        }
    }

    property int recordingElapsedSecs: 0
    function updateRecordingElapsed() {
        if (win.info.recording === true && (win.info.recordingStartedMs || 0) > 0) {
            win.recordingElapsedSecs = Math.max(0, Math.floor((Date.now() - win.info.recordingStartedMs) / 1000))
        } else {
            win.recordingElapsedSecs = 0
        }
    }
    onInfoChanged: updateRecordingElapsed()

    Timer {
        interval: 1000
        repeat: true
        running: win.info.recording === true
        onTriggered: win.updateRecordingElapsed()
    }

    function formatDuration(secs) {
        const m = Math.floor(secs / 60)
        const s = secs % 60
        return (m < 10 ? "0" + m : "" + m) + ":" + (s < 10 ? "0" + s : "" + s)
    }

    // Floating mirror toolbar: screenshots, MP4 recording, stay awake, screen off, and audio mute.
    Timer {
        id: toolbarWakeTimer
        interval: 1800
        repeat: false
    }
    Item {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: navPill.visible && !navPill.besideToolbar ? 104 : 56
        HoverHandler { id: topZoneHover }
    }
    Rectangle {
        id: toolbar
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 12
        width: toolbarRow.implicitWidth + 8
        height: 40
        radius: 20
        color: Qt.rgba(0, 0, 0, 0.68)
        visible: win.phase === "showing" && video.frameSize.width > 0
        readonly property bool toolbarHasFocus: lockBtn.activeFocus || copyShotBtn.activeFocus || saveShotBtn.activeFocus
            || recordBtn.activeFocus || stayAwakeBtn.activeFocus || screenOffBtn.activeFocus || soundBtn.activeFocus
            || backBtn.activeFocus || homeBtn.activeFocus || recentsBtn.activeFocus
        opacity: topZoneHover.hovered || toolbarHover.hovered || navHover.hovered || toolbarWakeTimer.running
            || win.info.recording === true || win.info.screenOff === true || toolbarHasFocus ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }

        HoverHandler { id: toolbarHover }

        Row {
            id: toolbarRow
            anchors.centerIn: parent
            spacing: 2

            IconButton {
                id: copyShotBtn
                iconPath: Icons.copy
                iconColor: "white"
                label: qsTr("Copy screenshot (Ctrl+Shift+C)")
                onClicked: Mirror.screenshotClipboard(win.mirrorKey)
            }
            IconButton {
                id: saveShotBtn
                iconPath: Icons.camera
                iconColor: "white"
                label: qsTr("Save screenshot (Ctrl+S)")
                onClicked: Mirror.screenshotFile(win.mirrorKey)
            }
            IconButton {
                id: recordBtn
                iconPath: win.info.recording === true ? Icons.stopSquare : Icons.record
                iconColor: win.info.recording === true ? Theme.error : "white"
                label: win.info.recording === true ? qsTr("Stop recording (Ctrl+R)") : qsTr("Record video (Ctrl+R)")
                onClicked: Mirror.toggleRecording(win.mirrorKey)
            }
            Txt {
                visible: win.info.recording === true
                anchors.verticalCenter: parent.verticalCenter
                leftPadding: 2
                rightPadding: 6
                role: "code"
                size: 12
                color: "white"
                text: win.formatDuration(win.recordingElapsedSecs)
            }
            IconButton {
                id: stayAwakeBtn
                iconPath: Icons.coffee
                iconColor: win.info.stayAwake === true ? Theme.primary : "white"
                label: win.info.stayAwake === true
                    ? qsTr("Phone stays awake while mirroring (turn off)")
                    : qsTr("Keep phone awake while mirroring")
                onClicked: Mirror.setStayAwake(win.mirrorKey, !(win.info.stayAwake === true))
            }
            IconButton {
                id: screenOffBtn
                visible: win.info.canScreenOff === true
                iconPath: Icons.screenOff
                iconColor: win.info.screenOff === true ? Theme.primary : "white"
                label: win.info.screenOff === true
                    ? qsTr("Turn phone screen back on")
                    : qsTr("Turn phone screen off while mirroring")
                onClicked: Mirror.setScreenOff(win.mirrorKey, !(win.info.screenOff === true))
            }
            IconButton {
                id: lockBtn
                visible: win.canControl
                iconPath: Icons.lock
                iconColor: "white"
                label: qsTr("Lock phone")
                onClicked: Mirror.press(win.mirrorKey, "lock")
            }
            IconButton {
                id: pinBtn
                iconPath: Icons.pin
                iconColor: win.staysOnTop ? Theme.primary : "white"
                label: win.staysOnTop ? qsTr("Don't keep on top") : qsTr("Keep on top")
                onClicked: toggleStayOnTop()
            }
            IconButton {
                id: soundBtn
                // Muted, no sound comes in: this brings it back.
                visible: win.info.sound === true || (Mirror.muted && !win.info.app)
                iconPath: Mirror.muted ? Icons.soundOff : Icons.speaker
                iconColor: "white"
                label: Mirror.muted ? qsTr("Play the phone's sound here") : qsTr("Play the sound on the phone")
                onClicked: Mirror.toggleSound()
            }
        }
    }

    // The phone's three navigation keys, whole screen only. Beside the toolbar
    // when the window is wide enough, under it when it's phone-narrow.
    Rectangle {
        id: navPill
        readonly property bool besideToolbar: win.width >= toolbar.width + width + 30
        anchors.top: besideToolbar ? parent.top : toolbar.bottom
        anchors.topMargin: besideToolbar ? 12 : 6
        anchors.right: besideToolbar ? toolbar.left : parent.right
        anchors.rightMargin: besideToolbar ? 6 : 12
        width: navRow.implicitWidth + 8
        height: 40
        radius: 20
        color: Qt.rgba(0, 0, 0, 0.68)
        visible: toolbar.visible && win.canControl && !win.info.app
        opacity: toolbar.opacity

        HoverHandler { id: navHover }

        Row {
            id: navRow
            anchors.centerIn: parent
            spacing: 2
            // Faded out, a click here belongs to the phone's screen.
            enabled: navPill.opacity > 0

            IconButton {
                id: backBtn
                iconPath: Icons.back
                iconColor: "white"
                label: qsTr("Back (right-click)")
                onClicked: Mirror.press(win.mirrorKey, "back")
            }
            IconButton {
                id: homeBtn
                iconPath: Icons.home
                iconColor: "white"
                label: qsTr("Home (middle-click)")
                onClicked: Mirror.press(win.mirrorKey, "home")
            }
            IconButton {
                id: recentsBtn
                iconPath: Icons.apps
                iconColor: "white"
                label: qsTr("Recent apps")
                onClicked: Mirror.press(win.mirrorKey, "recents")
            }
        }
    }

    // The phone's sound is muted (in every window): a small badge in the
    // toolbar's corner says so, instead of the whole toolbar staying over
    // the picture. A click plays the sound again.
    Rectangle {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 12
        width: 32
        height: 32
        radius: 16
        color: Qt.rgba(0, 0, 0, 0.68)
        visible: opacity > 0
        opacity: Mirror.muted && !win.info.app && toolbar.visible && toolbar.opacity === 0 ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
        IconButton {
            anchors.centerIn: parent
            width: 28
            height: 28
            iconSize: 14
            iconPath: Icons.soundOff
            iconColor: "white"
            label: qsTr("Play the phone's sound here")
            onClicked: Mirror.toggleSound()
        }
    }

    // Brief confirmation pill for screenshots, recordings, file drops, and power toggles.
    Rectangle {
        id: noticePill
        readonly property string pillText: mirrorDropArea.containsDrag
            ? qsTr("Release to send files to %1").arg(win.deviceName.length > 0 ? win.deviceName : qsTr("phone"))
            : (win.info.notice || "")
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: hintBanner.visible ? hintBanner.top : parent.bottom
        anchors.bottomMargin: hintBanner.visible ? 8 : 16
        width: Math.min(parent.width - 24, noticeTxt.implicitWidth + 24)
        height: noticeTxt.implicitHeight + 12
        radius: height / 2
        color: Qt.rgba(0, 0, 0, 0.78)
        visible: opacity > 0
        opacity: pillText.length > 0 ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
        Txt {
            id: noticeTxt
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            role: "caption"
            color: "white"
            text: noticePill.pillText
        }
    }

    // How to control it, until the phone allows it.
    Rectangle {
        id: hintBanner
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 12
        height: Math.max(hint.implicitHeight + 20, 48)
        radius: Theme.radiusMd
        visible: win.phase === "showing" && video.frameSize.width > 0 && !win.canControl && !win.info.app && !hintClose.closed
        color: Qt.rgba(0, 0, 0, 0.72)

        SequentialAnimation {
            id: hintPulse
            NumberAnimation {
                target: hintBanner
                property: "scale"
                from: 1.0
                to: Theme.reduceMotion ? 1.0 : 1.03
                duration: 110
                easing.type: Easing.OutQuad
            }
            NumberAnimation {
                target: hintBanner
                property: "scale"
                from: Theme.reduceMotion ? 1.0 : 1.03
                to: 1.0
                duration: 140
                easing.type: Easing.InOutQuad
            }
        }

        Txt {
            id: hint
            anchors.left: parent.left
            anchors.right: notifyPhoneBtn.left
            anchors.margins: 10
            anchors.verticalCenter: parent.verticalCenter
            wrapMode: Text.WordWrap
            role: "bodySmall"
            color: "white"
            text: qsTr("To control the phone, turn on Control from your PC in NectarLink's settings on the phone.")
        }
        Button {
            id: notifyPhoneBtn
            anchors.right: hintClose.left
            anchors.rightMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            variant: "tonal"
            size: "sm"
            text: qsTr("Notify phone")
            onClicked: Mirror.requestControl(win.mirrorKey)
        }
        IconButton {
            id: hintClose
            property bool closed: false
            anchors.right: parent.right
            anchors.rightMargin: 4
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
