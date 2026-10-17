// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Shapes
import app.nectarlink

// Dynamic Island-style floating pill anchored at the top-center of the primary
// monitor (340×40px compact, expanding smoothly to 380×88px on hover or incoming call).
// Stays hidden when idle and surfaces live activity:
//   1. Incoming / active phone call (caller name + Answer / Mute / Speaker / Hang up)
//   2. File transfer in progress (circular progress ring + transfer title & speed)
//   3. Active media playback on Phone or PC (album art + scrolling song title + Play/Pause & Next)
Window {
    id: islandWin

    title: qsTr("Nectar Island")
    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
    color: "transparent"

    readonly property int availW: Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920

    // The work area (the screen minus the taskbar, wherever the taskbar is). Qt only knows
    // its size, so Windows reports the position; refreshed when the work area changes.
    property var workArea: AppController.workArea()
    readonly property int workAreaKey: Screen.desktopAvailableWidth * 100000 + Screen.desktopAvailableHeight
    onWorkAreaKeyChanged: workArea = AppController.workArea()
    readonly property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real areaX: workArea && workArea.width ? workArea.x / dpr : Screen.virtualX
    readonly property real areaY: workArea && workArea.height ? workArea.y / dpr : Screen.virtualY
    readonly property real areaW: workArea && workArea.width ? workArea.width / dpr : availW

    // ---- Track active media across all rows of MediaList (Phone + This PC) ----
    property string mediaDeviceId: ""
    property string mediaPlayerId: ""
    property string mediaApp: ""
    property string mediaTitle: ""
    property string mediaArtist: ""
    property string mediaArt: ""
    property bool mediaPlaying: false
    property bool mediaCanPlay: true
    property bool mediaCanPause: true
    property bool mediaCanNext: true
    property bool mediaCanPrev: true
    property real mediaDuration: 0
    property real mediaPosition: 0

    // ---- Track active file transfer ----
    property string activeTransferId: ""
    property string activeTransferTitle: ""
    property real activeTransferProgress: 0
    property real activeTransferRate: 0
    property bool activeTransferIncoming: false
    property bool transferJustFinished: false

    // ---- Transient notification toast (only when a new phone notification arrives) ----
    property bool notificationToastActive: false
    property int lastNotificationCount: NotificationList.count
    Timer {
        id: notificationToastTimer
        interval: 4200
        repeat: false
        onTriggered: islandWin.notificationToastActive = false
    }
    Connections {
        target: NotificationList
        function onCountChanged() {
            const cur = NotificationList.count
            if (cur > islandWin.lastNotificationCount && Preferences.nectarIslandNotifications) {
                islandWin.notificationToastActive = true
                notificationToastTimer.restart()
            } else if (cur === 0) {
                islandWin.notificationToastActive = false
            }
            islandWin.lastNotificationCount = cur
        }
    }

    Timer {
        id: transferDoneTimer
        interval: 3200
        repeat: false
        onTriggered: islandWin.transferJustFinished = false
    }

    property int lastActiveTransfers: TransferList.active
    onLastActiveTransfersChanged: {
        if (lastActiveTransfers === 0 && activeTransferTitle.length > 0) {
            transferJustFinished = true
            transferDoneTimer.restart()
        }
    }

    function recomputeMedia() {
        let best = null
        const count = mediaRepeater.count
        for (let i = 0; i < count; i++) {
            const item = mediaRepeater.itemAt(i)
            if (!item) continue
            // Only surface phone media in Nectar Island (never PC-local media, which is already on the PC)
            if (item.deviceId === "local") continue
            if (item.playing) {
                if (!best || !best.playing) {
                    best = item
                }
            }
        }
        if (best) {
            mediaDeviceId = best.deviceId
            mediaPlayerId = best.playerId
            mediaApp = best.app
            mediaTitle = best.title
            mediaArtist = best.artist
            mediaArt = best.art
            mediaPlaying = best.playing
            mediaCanPlay = best.canPlay
            mediaCanPause = best.canPause
            mediaCanNext = best.canNext
            mediaCanPrev = best.canPrevious
            mediaDuration = best.duration
            mediaPosition = best.position
        } else {
            mediaDeviceId = ""
            mediaPlayerId = ""
            mediaApp = ""
            mediaTitle = ""
            mediaArtist = ""
            mediaArt = ""
            mediaPlaying = false
        }
    }

    function recomputeTransfer() {
        let found = null
        const count = transferRepeater.count
        for (let i = 0; i < count; i++) {
            const item = transferRepeater.itemAt(i)
            if (!item) continue
            if (item.status === "running" || item.status === "waiting" || item.status === "requested") {
                found = item
                break
            }
        }
        if (found) {
            activeTransferId = found.transferId
            activeTransferTitle = found.title
            activeTransferProgress = Math.max(0, Math.min(1, found.progress || 0))
            activeTransferRate = found.rate || 0
            activeTransferIncoming = found.incoming
        } else if (!transferJustFinished) {
            activeTransferId = ""
            activeTransferTitle = ""
            activeTransferProgress = 0
            activeTransferRate = 0
        }
    }

    function formatRate(bytesPerSec) {
        const b = Number(bytesPerSec) || 0
        if (b <= 0) return ""
        if (b < 1024 * 1024)
            return (b / 1024).toFixed(0) + " KB/s"
        return (b / (1024 * 1024)).toFixed(1) + " MB/s"
    }

    readonly property bool hasMediaCandidate: mediaPlaying && mediaDeviceId !== "local" && mediaTitle.length > 0
    readonly property string mode: {
        if (Preferences.nectarIslandCalls && PhoneCall.active) return "call"
        if (Preferences.nectarIslandTransfers && (TransferList.active > 0 || transferJustFinished)) return "transfer"
        if (Preferences.nectarIslandMedia && hasMediaCandidate) return "media"
        if (Preferences.nectarIslandNotifications && notificationToastActive && NotificationList.count > 0) return "notification"
        if (Preferences.nectarIslandTransfers && TransferList.waitingCount > 0) return "waiting"
        return "idle"
    }

    readonly property bool hasActivity: mode !== "idle"
    visible: Preferences.nectarIsland && hasActivity

    onVisibleChanged: {
        if (visible) {
            Qt.callLater(() => AppController.styleShelfWindow())
            if (!Theme.reduceMotion)
                dropInAnim.restart()
        }
    }

    Component.onCompleted: {
        Qt.callLater(() => {
            recomputeMedia()
            recomputeTransfer()
            AppController.styleShelfWindow()
        })
    }

    Connections {
        target: Preferences
        function onNectarIslandChanged() {
            if (Preferences.nectarIsland) {
                Qt.callLater(() => AppController.styleShelfWindow())
            }
        }
    }

    readonly property bool expanded: pillHover.hovered || (Preferences.nectarIslandCalls && PhoneCall.active && PhoneCall.ringing)
    readonly property real pillWidth: expanded ? 380 : 340
    readonly property real pillHeight: expanded ? 88 : 40
    width: 400
    height: 104
    x: areaX + Math.round((areaW - width) / 2)
    y: areaY + 8

    readonly property string primaryLabel: {
        if (mode === "call")
            return PhoneCall.caller.length > 0 ? PhoneCall.caller : qsTr("Phone call")
        if (mode === "transfer") {
            if (transferJustFinished && TransferList.active === 0)
                return qsTr("Transfer complete")
            return activeTransferTitle.length > 0
                ? activeTransferTitle
                : qsTr("Transferring %1 file(s)…").arg(TransferList.active)
        }
        if (mode === "media")
            return mediaTitle.length > 0 ? mediaTitle : (mediaApp.length > 0 ? mediaApp : qsTr("Now playing"))
        if (mode === "notification")
            return qsTr("%1 phone notification(s)").arg(NotificationList.count)
        if (mode === "waiting")
            return qsTr("%1 waiting").arg(TransferList.waitingCount)
        return DeviceList.count > 0
            ? qsTr("Nectar Island · %1").arg(DeviceList.deviceNameAt(0))
            : qsTr("Nectar Island · Ready")
    }

    readonly property string secondaryLabel: {
        if (mode === "call") {
            if (PhoneCall.ringing) return qsTr("Incoming call")
            if (PhoneCall.held) return qsTr("On hold")
            return PhoneCall.number.length > 0 ? PhoneCall.number : qsTr("Call in progress")
        }
        if (mode === "transfer") {
            if (transferJustFinished && TransferList.active === 0)
                return activeTransferTitle
            const pct = Math.round(activeTransferProgress * 100) + "%"
            const spd = formatRate(activeTransferRate)
            const base = spd.length > 0 ? (pct + " · " + spd) : pct
            return TransferList.waitingCount > 0
                ? (base + " · " + qsTr("%1 waiting").arg(TransferList.waitingCount))
                : base
        }
        if (mode === "media") {
            const src = mediaDeviceId === "local" ? qsTr("This PC") : qsTr("Phone")
            const base = mediaArtist.length > 0
                ? (mediaArtist + " · " + src)
                : (mediaApp.length > 0 ? (mediaApp + " · " + src) : src)
            return TransferList.waitingCount > 0
                ? (base + " · " + qsTr("%1 waiting").arg(TransferList.waitingCount))
                : base
        }
        if (mode === "notification")
            return qsTr("Click to view in NectarLink")
        if (mode === "waiting") {
            return DeviceList.count > 0
                ? qsTr("Will send when %1 is back").arg(DeviceList.deviceNameAt(0))
                : qsTr("Will send when connected")
        }
        return qsTr("Live calls, transfers & media")
    }

    Item {
        id: root
        anchors.fill: parent

        // Hidden repeaters inside Item scope so QtQuick instantiates delegates cleanly
        Repeater {
            id: mediaRepeater
            model: MediaList
            onItemAdded: islandWin.recomputeMedia()
            onItemRemoved: islandWin.recomputeMedia()
            delegate: Item {
                required property int index
                required property string deviceId
                required property string playerId
                required property string app
                required property string title
                required property string artist
                required property string art
                required property bool playing
                required property bool canPlay
                required property bool canPause
                required property bool canNext
                required property bool canPrevious
                required property real duration
                required property real position
                visible: false
                onPlayingChanged: islandWin.recomputeMedia()
                onTitleChanged: islandWin.recomputeMedia()
                onArtistChanged: islandWin.recomputeMedia()
                onArtChanged: islandWin.recomputeMedia()
                onDeviceIdChanged: islandWin.recomputeMedia()
            }
        }

        Repeater {
            id: transferRepeater
            model: TransferList
            onItemAdded: islandWin.recomputeTransfer()
            onItemRemoved: islandWin.recomputeTransfer()
            delegate: Item {
                required property int index
                required property string transferId
                required property string title
                required property real progress
                required property real rate
                required property bool incoming
                required property string status
                visible: false
                onStatusChanged: islandWin.recomputeTransfer()
                onProgressChanged: islandWin.recomputeTransfer()
                onRateChanged: islandWin.recomputeTransfer()
                onTitleChanged: islandWin.recomputeTransfer()
            }
        }

        transform: Translate {
            id: dropSlide
            y: 0
        }

        NumberAnimation {
            id: dropInAnim
            target: dropSlide
            property: "y"
            from: -18
            to: 0
            duration: 240
            easing.type: Easing.OutBack
        }

        // Subtle outer ambient halo around the Dynamic Island capsule
        Rectangle {
            anchors.centerIn: pillBg
            width: pillBg.width + 6
            height: pillBg.height + 6
            radius: height / 2
            color: "transparent"
            border.width: 1.5
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.32)
                : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
        }

        Rectangle {
            id: pillBg
            anchors.top: parent.top
            anchors.topMargin: 2
            anchors.horizontalCenter: parent.horizontalCenter
            width: islandWin.pillWidth
            height: islandWin.pillHeight
            radius: height / 2
            // Authentic deep Dynamic Island obsidian surface with crisp specular glass border
            color: Qt.rgba(0.05, 0.055, 0.075, Theme.glass ? 0.92 : 0.96)
            border.width: 1
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.55)
                : Qt.rgba(1, 1, 1, 0.18)
            clip: true

            Behavior on width {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 220; easing.type: Easing.OutBack }
            }
            Behavior on height {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 220; easing.type: Easing.OutBack }
            }

            HoverHandler {
                id: pillHover
                cursorShape: Qt.PointingHandCursor
            }

            TapHandler {
                onTapped: (eventPoint) => {
                    if (compactMediaControls.visible) {
                        const p1 = compactMediaControls.mapFromItem(pillBg, eventPoint.position)
                        if (p1.x >= 0 && p1.y >= 0 && p1.x <= compactMediaControls.width && p1.y <= compactMediaControls.height)
                            return
                    }
                    if (expandedControlsRow.visible) {
                        const p2 = expandedControlsRow.mapFromItem(pillBg, eventPoint.position)
                        if (p2.x >= 0 && p2.y >= 0 && p2.x <= expandedControlsRow.width && p2.y <= expandedControlsRow.height)
                            return
                    }
                    AppController.activateRequested()
                }
            }

            // ================================================================
            // COMPACT PILL VIEW (340×40px)
            // ================================================================
            Item {
                id: compactView
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 10
                visible: !islandWin.expanded
                opacity: islandWin.expanded ? 0.0 : 1.0
                Behavior on opacity { NumberAnimation { duration: 140 } }

                // Left slot: Album Art / Progress Ring / Call Pulse / Brand Badge
                Item {
                    id: compactLeftSlot
                    width: 26
                    height: 26
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter

                    // 1. Media album art squircle
                    Rectangle {
                        anchors.fill: parent
                        radius: 8
                        visible: islandWin.mode === "media"
                        color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            visible: !(islandWin.mediaArt.length > 0 && compactArtImg.status === Image.Ready)
                            path: Icons.music
                            color: Theme.primary
                        }
                        RoundedImage {
                            id: compactArtImg
                            anchors.fill: parent
                            radius: 8
                            source: islandWin.mediaArt
                            sourceSize: Qt.size(64, 64)
                        }
                    }

                    // 2. Live file transfer circular progress ring
                    Item {
                        anchors.fill: parent
                        visible: islandWin.mode === "transfer"

                        Shape {
                            anchors.fill: parent
                            Component.onCompleted: if ("preferredRendererType" in this) preferredRendererType = 2

                            ShapePath {
                                strokeColor: Qt.rgba(1, 1, 1, 0.16)
                                fillColor: "transparent"
                                strokeWidth: 2.6
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 13; centerY: 13
                                    radiusX: 10.5; radiusY: 10.5
                                    startAngle: 0; sweepAngle: 360
                                }
                            }
                            ShapePath {
                                strokeColor: islandWin.transferJustFinished ? "#4ADE80" : Theme.primary
                                fillColor: "transparent"
                                strokeWidth: 2.6
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 13; centerY: 13
                                    radiusX: 10.5; radiusY: 10.5
                                    startAngle: -90
                                    sweepAngle: 360 * (islandWin.transferJustFinished ? 1.0 : Math.max(0.06, islandWin.activeTransferProgress))
                                }
                            }
                        }

                        Icon {
                            anchors.centerIn: parent
                            width: 11; height: 11
                            path: islandWin.transferJustFinished ? Icons.check : Icons.send
                            color: islandWin.transferJustFinished ? "#4ADE80" : "#FFFFFF"
                        }
                    }

                    // 3. Call / Notification / Waiting / Ready badge
                    Rectangle {
                        anchors.fill: parent
                        radius: 13
                        visible: islandWin.mode === "call" || islandWin.mode === "notification" || islandWin.mode === "waiting" || islandWin.mode === "ready"
                        color: islandWin.mode === "call"
                            ? Qt.rgba(0.29, 0.87, 0.50, 0.22)
                            : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.22)
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            path: islandWin.mode === "call"
                                ? Icons.call
                                : (islandWin.mode === "notification"
                                    ? Icons.bell
                                    : (islandWin.mode === "waiting" ? Icons.send : Icons.phone))
                            color: islandWin.mode === "call" ? "#4ADE80" : Theme.primary
                        }
                    }
                }

                // Right slot: Equalizer + inline quick transport controls (or % badge)
                Row {
                    id: compactMediaControls
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    // Animated 3-bar equalizer in media mode
                    Row {
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 1.8
                        height: 12
                        visible: islandWin.mode === "media"
                        Repeater {
                            model: 3
                            delegate: Rectangle {
                                id: cEqBar
                                required property int index
                                width: 2.2
                                radius: 1.1
                                anchors.bottom: parent.bottom
                                color: Theme.primary
                                height: islandWin.mediaPlaying ? 8 : 3
                                SequentialAnimation on height {
                                    running: islandWin.visible && !islandWin.expanded && islandWin.mediaPlaying && !Theme.reduceMotion
                                    loops: Animation.Infinite
                                    NumberAnimation {
                                        from: 3
                                        to: cEqBar.index === 1 ? 12 : (cEqBar.index === 0 ? 9 : 10)
                                        duration: 240 + cEqBar.index * 80
                                        easing.type: Easing.InOutSine
                                    }
                                    NumberAnimation {
                                        to: 3
                                        duration: 220 + cEqBar.index * 70
                                        easing.type: Easing.InOutSine
                                    }
                                }
                            }
                        }
                    }

                    // Quick Play/Pause button right in the compact pill
                    Rectangle {
                        visible: islandWin.mode === "media"
                        width: 24; height: 24
                        radius: 12
                        anchors.verticalCenter: parent.verticalCenter
                        color: cPlayHover.hovered ? Qt.rgba(1, 1, 1, 0.18) : Qt.rgba(1, 1, 1, 0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 12; height: 12
                            path: islandWin.mediaPlaying ? Icons.pause : Icons.play
                            color: "#FFFFFF"
                        }
                        HoverHandler { id: cPlayHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(
                                islandWin.mediaDeviceId,
                                islandWin.mediaPlayerId,
                                islandWin.mediaPlaying ? "pause" : "play",
                                0
                            )
                        }
                    }

                    // Quick Next Track button right in the compact pill
                    Rectangle {
                        visible: islandWin.mode === "media" && islandWin.mediaCanNext
                        width: 24; height: 24
                        radius: 12
                        anchors.verticalCenter: parent.verticalCenter
                        color: cNextHover.hovered ? Qt.rgba(1, 1, 1, 0.18) : "transparent"
                        Icon {
                            anchors.centerIn: parent
                            width: 12; height: 12
                            path: Icons.skipNext
                            color: Qt.rgba(1, 1, 1, 0.85)
                        }
                        HoverHandler { id: cNextHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "next", 0)
                        }
                    }

                    // Transfer percentage pill
                    Txt {
                        visible: islandWin.mode === "transfer" && !islandWin.transferJustFinished
                        anchors.verticalCenter: parent.verticalCenter
                        role: "caption"
                        size: 11
                        weight: Font.Bold
                        color: Theme.primary
                        text: Math.round(islandWin.activeTransferProgress * 100) + "%"
                    }
                }

                // Center slot: Scrolling / crisp title + subtle artist/subtitle
                Item {
                    id: compactTitleClip
                    anchors.left: compactLeftSlot.right
                    anchors.leftMargin: 9
                    anchors.right: compactMediaControls.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    height: 24
                    clip: true

                    readonly property bool shouldMarquee: compactTitleMetrics.width > compactTitleClip.width
                        && islandWin.visible && !islandWin.expanded && !Theme.reduceMotion

                    TextMetrics {
                        id: compactTitleMetrics
                        font: compactTitleTxt.font
                        text: islandWin.primaryLabel + (islandWin.mediaArtist.length > 0 && islandWin.mode === "media" ? "  ·  " + islandWin.mediaArtist : "")
                    }

                    Txt {
                        id: compactTitleTxt
                        anchors.verticalCenter: parent.verticalCenter
                        width: compactTitleClip.shouldMarquee ? compactTitleMetrics.width : compactTitleClip.width
                        role: "label"
                        size: 12
                        weight: Font.DemiBold
                        color: "#FFFFFF"
                        elide: compactTitleClip.shouldMarquee ? Text.ElideNone : Text.ElideRight
                        text: compactTitleMetrics.text

                        SequentialAnimation on x {
                            running: compactTitleClip.shouldMarquee && islandWin.visible && !islandWin.expanded
                                     && !Theme.reduceMotion
                            onRunningChanged: if (!running) compactTitleTxt.x = 0
                            loops: Animation.Infinite
                            PauseAnimation { duration: 1200 }
                            NumberAnimation {
                                from: 0
                                to: -Math.max(0, compactTitleMetrics.width - compactTitleClip.width + 12)
                                duration: Math.max(1800, (compactTitleMetrics.width - compactTitleClip.width) * 28)
                                easing.type: Easing.InOutSine
                            }
                            PauseAnimation { duration: 900 }
                            NumberAnimation {
                                to: 0
                                duration: 900
                                easing.type: Easing.OutCubic
                            }
                        }
                        onTextChanged: x = 0
                    }
                }
            }

            // ================================================================
            // EXPANDED PILL VIEW (380×88px on hover or incoming call)
            // ================================================================
            Item {
                id: expandedView
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                anchors.topMargin: 12
                anchors.bottomMargin: 12
                visible: islandWin.expanded
                opacity: islandWin.expanded ? 1.0 : 0.0
                Behavior on opacity { NumberAnimation { duration: 160 } }

                // Left cover / artwork / progress dial (48×48px)
                Item {
                    id: expLeftSlot
                    width: 48
                    height: 48
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter

                    Rectangle {
                        anchors.fill: parent
                        radius: 12
                        visible: islandWin.mode === "media"
                        color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
                        Icon {
                            anchors.centerIn: parent
                            width: 22; height: 22
                            visible: !(islandWin.mediaArt.length > 0 && expArtImg.status === Image.Ready)
                            path: Icons.music
                            color: Theme.primary
                        }
                        RoundedImage {
                            id: expArtImg
                            anchors.fill: parent
                            radius: 12
                            source: islandWin.mediaArt
                            sourceSize: Qt.size(112, 112)
                        }
                    }

                    Item {
                        anchors.fill: parent
                        visible: islandWin.mode === "transfer"

                        Shape {
                            anchors.fill: parent
                            Component.onCompleted: if ("preferredRendererType" in this) preferredRendererType = 2
                            ShapePath {
                                strokeColor: Qt.rgba(1, 1, 1, 0.16)
                                fillColor: "transparent"
                                strokeWidth: 3.4
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 24; centerY: 24
                                    radiusX: 19; radiusY: 19
                                    startAngle: 0; sweepAngle: 360
                                }
                            }
                            ShapePath {
                                strokeColor: islandWin.transferJustFinished ? "#4ADE80" : Theme.primary
                                fillColor: "transparent"
                                strokeWidth: 3.4
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 24; centerY: 24
                                    radiusX: 19; radiusY: 19
                                    startAngle: -90
                                    sweepAngle: 360 * (islandWin.transferJustFinished ? 1.0 : Math.max(0.06, islandWin.activeTransferProgress))
                                }
                            }
                        }

                        Txt {
                            anchors.centerIn: parent
                            size: 10
                            weight: Font.Bold
                            color: "#FFFFFF"
                            text: islandWin.transferJustFinished
                                ? "100%"
                                : (Math.round(islandWin.activeTransferProgress * 100) + "%")
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        visible: islandWin.mode === "call" || islandWin.mode === "notification" || islandWin.mode === "waiting" || islandWin.mode === "ready"
                        color: islandWin.mode === "call"
                            ? Qt.rgba(0.29, 0.87, 0.50, 0.22)
                            : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.22)
                        Icon {
                            anchors.centerIn: parent
                            width: 22; height: 22
                            path: islandWin.mode === "call"
                                ? Icons.call
                                : (islandWin.mode === "notification"
                                    ? Icons.bell
                                    : (islandWin.mode === "waiting" ? Icons.send : Icons.phone))
                            color: islandWin.mode === "call" ? "#4ADE80" : Theme.primary
                        }
                    }
                }

                // Center title + subtitle + thin progress bar
                Column {
                    anchors.left: expLeftSlot.right
                    anchors.leftMargin: 12
                    anchors.right: expandedControlsRow.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 3

                    Txt {
                        width: parent.width
                        role: "title"
                        size: 13
                        weight: Font.DemiBold
                        color: "#FFFFFF"
                        elide: Text.ElideRight
                        text: islandWin.primaryLabel
                    }

                    Txt {
                        width: parent.width
                        role: "caption"
                        size: 11
                        color: Qt.rgba(1, 1, 1, 0.68)
                        elide: Text.ElideRight
                        text: islandWin.secondaryLabel
                    }

                    // Sleek progress bar for media or transfer
                    Rectangle {
                        visible: (islandWin.mode === "media" && islandWin.mediaDuration > 0) || islandWin.mode === "transfer"
                        width: parent.width
                        height: 3.5
                        radius: 1.75
                        color: Qt.rgba(1, 1, 1, 0.16)

                        Rectangle {
                            height: parent.height
                            radius: parent.radius
                            color: islandWin.mode === "transfer" && islandWin.transferJustFinished ? "#4ADE80" : Theme.primary
                            width: {
                                if (islandWin.mode === "transfer")
                                    return parent.width * (islandWin.transferJustFinished ? 1.0 : Math.max(0.04, islandWin.activeTransferProgress))
                                if (islandWin.mediaDuration > 0)
                                    return parent.width * Math.max(0, Math.min(1, islandWin.mediaPosition / islandWin.mediaDuration))
                                return 0
                            }
                            Behavior on width { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
                        }
                    }
                }

                // Right action buttons in expanded pill
                Row {
                    id: expandedControlsRow
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6

                    // ---- Call controls: Answer (when ringing), Mute, Speaker, Hang up ----
                    Rectangle {
                        visible: islandWin.mode === "call" && PhoneCall.ringing && PhoneCall.canEnd
                        width: 32; height: 32; radius: 16
                        color: "#22C55E"
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: Icons.call
                            color: "#FFFFFF"
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.answer() }
                    }

                    Rectangle {
                        visible: islandWin.mode === "call" && !PhoneCall.ringing && PhoneCall.controls
                        width: 30; height: 30; radius: 15
                        color: PhoneCall.muted ? Qt.rgba(1, 1, 1, 0.26) : Qt.rgba(1, 1, 1, 0.12)
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            path: PhoneCall.muted ? Icons.micOff : Icons.mic
                            color: "#FFFFFF"
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.setMute(!PhoneCall.muted) }
                    }

                    Rectangle {
                        visible: islandWin.mode === "call" && PhoneCall.canEnd
                        width: 32; height: 32; radius: 16
                        color: "#EF4444"
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: Icons.callEnd
                            color: "#FFFFFF"
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.end() }
                    }

                    // ---- Media controls: Previous, Play/Pause, Next ----
                    Rectangle {
                        visible: islandWin.mode === "media" && islandWin.mediaCanPrev
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expPrevHover.hovered ? Qt.rgba(1, 1, 1, 0.20) : Qt.rgba(1, 1, 1, 0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.skipPrevious
                            color: "#FFFFFF"
                        }
                        HoverHandler { id: expPrevHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "previous", 0)
                        }
                    }

                    Rectangle {
                        visible: islandWin.mode === "media"
                        width: 34; height: 34; radius: 17
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.primary
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: islandWin.mediaPlaying ? Icons.pause : Icons.play
                            color: Theme.primaryContent
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(
                                islandWin.mediaDeviceId,
                                islandWin.mediaPlayerId,
                                islandWin.mediaPlaying ? "pause" : "play",
                                0
                            )
                        }
                    }

                    Rectangle {
                        visible: islandWin.mode === "media" && islandWin.mediaCanNext
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expNextHover.hovered ? Qt.rgba(1, 1, 1, 0.20) : Qt.rgba(1, 1, 1, 0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.skipNext
                            color: "#FFFFFF"
                        }
                        HoverHandler { id: expNextHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "next", 0)
                        }
                    }

                    // ---- Transfer cancel button ----
                    Rectangle {
                        visible: islandWin.mode === "transfer" && !islandWin.transferJustFinished && islandWin.activeTransferId.length > 0
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expCancelHover.hovered ? Qt.rgba(1, 0.3, 0.3, 0.28) : Qt.rgba(1, 1, 1, 0.10)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.close
                            color: "#FFFFFF"
                        }
                        HoverHandler { id: expCancelHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: TransferList.cancel(islandWin.activeTransferId)
                        }
                    }
                }
            }
        }
    }
}
