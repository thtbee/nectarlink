// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Effects
import app.nectarlink

// What plays on a phone or this PC: compact by default and adjustable (expand/collapse),
// with a native GPU-blurred album art backdrop, animated equalizer bars,
// smooth track transitions, a 60fps animated progress bar with traveling shimmer,
// -10s / +10s seek controls, inline volume slider, and one-tap Glass/Artwork theme sync.
Card {
    id: card
    required property string deviceId
    required property string playerId
    required property string app
    required property string title
    required property string artist
    required property bool playing
    required property real duration
    required property real position
    required property real positionAt
    required property string art
    required property bool canPlay
    required property bool canPause
    required property bool canNext
    required property bool canPrevious
    required property bool canSeek

    property bool embeddedInHero: false
    property bool hasMultipleSources: false
    // This PC's players, `[{ id, app, playing }]`: two or more get a picker (#665).
    property var pcPlayers: []
    property string activeSource: deviceId === "local" ? "local" : "phone"
    property int phoneVolume: 65
    signal sourceSelected(string source)
    signal playerSelected(string playerId)

    // Compact by default; user can expand/collapse at any time.
    property bool expanded: false
    padding: expanded ? 14 : 12
    radius: embeddedInHero ? 22 : Theme.radiusLg

    readonly property bool activelyRendered: card.visible
        && Qt.application.state === Qt.ApplicationActive
    readonly property bool windowResizing: Window.window && Window.window.resizing ? true : false

    color: embeddedInHero
        ? Qt.rgba(Theme.surfaceContainerHigh.r, Theme.surfaceContainerHigh.g, Theme.surfaceContainerHigh.b, Theme.glass ? (Theme.dark ? 0.46 : 0.56) : (Theme.dark ? 0.52 : 0.68))
        : Theme.cardColor
    border.width: embeddedInHero ? (Theme.glass ? 1 : 0) : (Theme.graphite || Theme.ambientBlurActive ? 1 : 0)
    border.color: embeddedInHero
        ? Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.34)
        : Theme.cardBorder

    implicitHeight: contentCol.implicitHeight + padding * 2
    height: implicitHeight
    Behavior on implicitHeight {
        enabled: !Theme.reduceMotion && card.activelyRendered && !card.windowResizing
        NumberAnimation { duration: 200; easing.type: Easing.OutCubic }
    }

    // Rounded-masked internal blurred album-art glow inside the media player card (zero rectangular corner bleed!)
    Item {
        id: cardArtBackdrop
        x: -card.padding
        y: -card.padding
        width: card.width
        height: card.height
        z: 0
        visible: card.art.length > 0 && cardArtBg.status === Image.Ready && !Theme.graphite
        opacity: Theme.dark ? 0.38 : 0.26
        Behavior on opacity { NumberAnimation { duration: 320 } }

        Rectangle {
            id: cardArtMask
            anchors.fill: parent
            radius: card.radius
            visible: false
            layer.enabled: card.activelyRendered && cardArtBackdrop.visible
            layer.textureSize: Qt.size(192, 96)
            layer.smooth: true
        }

        Image {
            id: cardArtBg
            anchors.fill: parent
            source: card.art
            sourceSize: Qt.size(96, 96)
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            visible: false
            layer.enabled: card.activelyRendered && cardArtBackdrop.visible
            layer.textureSize: Qt.size(96, 96)
            layer.smooth: true
        }

        MultiEffect {
            visible: card.activelyRendered && cardArtBackdrop.visible
            anchors.fill: parent
            source: cardArtBg
            autoPaddingEnabled: false
            blurEnabled: true
            blurMax: 32
            blur: 1.0
            saturation: 0.45
            maskEnabled: true
            maskSource: cardArtMask
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
        }
    }

    // Low-overhead 2Hz timer while playing so seek bar and timestamps advance without continuous 60-120Hz GPU scene-graph redraws.
    property real now: Date.now()
    Timer {
        interval: 500
        repeat: true
        running: card.activelyRendered && card.playing && card.duration > 0
        onTriggered: card.now = Date.now()
        onRunningChanged: card.now = Date.now()
    }
    onPositionAtChanged: now = Date.now()

    // Smoothly animate title/artist when the track or active media source changes.
    onTitleChanged: {
        if (!Theme.reduceMotion && card.activelyRendered)
            trackSwitchAnim.restart()
    }
    onDeviceIdChanged: {
        if (!Theme.reduceMotion && card.activelyRendered)
            sourceSlideAnim.restart()
    }

    function switchSource(targetSource) {
        const currentSrc = card.isLocalPc ? "local" : "phone"
        if (!card.hasMultipleSources || targetSource === currentSrc)
            return
        sourceSlideOffset.x = targetSource === "local" ? 18 : -18
        sourceSlideAnim.restart()
        card.sourceSelected(targetSource)
    }

    readonly property real elapsed: position < 0 ? 0
        : Math.min(duration > 0 ? duration : Infinity, position + (playing ? Math.max(0, now - positionAt) : 0))
    // While dragging, the bar shows where the user is pointing.
    property real dragTo: -1
    readonly property real shown: dragTo >= 0 ? dragTo : elapsed
    // Quantized to 1Hz seconds so timestamp text never re-layouts 60-120 times per second.
    readonly property int shownSeconds: Math.floor(Math.max(0, card.shown) / 1000)
    readonly property int remainingSeconds: Math.floor(Math.max(0, card.duration - card.shown) / 1000)

    readonly property bool isLocalPc: card.deviceId === "local"
    property int dragVolume: -1
    readonly property int currentVolume: dragVolume >= 0
        ? dragVolume
        : (isLocalPc ? DeckController.volume : card.phoneVolume)
    readonly property bool currentMuted: isLocalPc ? DeckController.muted : (currentVolume <= 0)

    function send(action, to) { MediaList.command(deviceId, playerId, action, to === undefined ? 0 : to) }
    function skipBy(deltaMs) {
        if (!card.canSeek || card.duration <= 0) return
        const target = Math.round(Math.max(0, Math.min(card.duration, card.shown + deltaMs)))
        card.send("seek", target)
    }
    function clockFromSec(s) {
        const sec = Math.max(0, s)
        const h = Math.floor(sec / 3600)
        const m = Math.floor((sec % 3600) / 60)
        const ss = String(sec % 60).padStart(2, "0")
        return h > 0 ? h + ":" + String(m).padStart(2, "0") + ":" + ss : m + ":" + ss
    }

    // Horizontal swipe & wheel gesture handler when both Phone and PC media are available
    DragHandler {
        id: cardSwipeDrag
        enabled: card.hasMultipleSources && !seek.pressed && !volMouse.pressed
        target: null
        yAxis.enabled: false
        property real startX: 0
        onActiveChanged: {
            if (active) {
                startX = centroid.position.x
            } else {
                const dx = centroid.position.x - startX
                if (dx <= -28)
                    card.switchSource(card.isLocalPc ? "phone" : "local")
                else if (dx >= 28)
                    card.switchSource(card.isLocalPc ? "phone" : "local")
            }
        }
    }

    WheelHandler {
        enabled: card.hasMultipleSources
        acceptedDevices: PointerDevice.TouchPad | PointerDevice.Mouse
        onWheel: (event) => {
            const dx = event.angleDelta.x !== 0 ? event.angleDelta.x : ((event.modifiers & Qt.ShiftModifier) ? event.angleDelta.y : 0)
            if (dx <= -30) {
                card.switchSource("local")
                event.accepted = true
            } else if (dx >= 30) {
                card.switchSource("phone")
                event.accepted = true
            }
        }
    }

    Column {
        id: contentCol
        width: parent.width
        spacing: card.expanded ? 10 : 7
        z: 2
        transform: Translate { id: sourceSlideOffset; x: 0 }

        ParallelAnimation {
            id: sourceSlideAnim
            NumberAnimation {
                target: contentCol
                property: "opacity"
                from: 0.35
                to: 1.0
                duration: 190
                easing.type: Easing.OutCubic
            }
            NumberAnimation {
                target: sourceSlideOffset
                property: "x"
                to: 0
                duration: 210
                easing.type: Easing.OutCubic
            }
        }

        // ---- Compact header row: equalizer indicator + segmented Phone/PC switcher + app label + actions ----
        Item {
            width: parent.width
            height: 24

            Row {
                anchors.left: parent.left
                anchors.right: rightHeaderActions.left
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6

                // 3-bar equalizer indicator (settles cleanly without continuous 60fps loop)
                Row {
                    id: eqBars
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2
                    height: 12
                    Repeater {
                        model: 3
                        delegate: Rectangle {
                            id: eqBar
                            required property int index
                            width: 2.5
                            radius: 1.25
                            anchors.bottom: parent.bottom
                            color: card.playing ? Theme.primary : Theme.surfaceContentVariant
                            Behavior on color { ColorAnimation { duration: 200 } }
                            height: card.playing ? (eqBar.index === 1 ? 11.5 : (eqBar.index === 0 ? 7.5 : 9.5)) : 3.5
                            Behavior on height {
                                enabled: !Theme.reduceMotion
                                NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                            }
                        }
                    }
                }

                // Single source badge OR interactive dual-source segmented pill ([Phone | This PC])
                Rectangle {
                    id: sourcePill
                    anchors.verticalCenter: parent.verticalCenter
                    height: 21
                    width: card.hasMultipleSources ? dualSourceRow.implicitWidth + 6 : singleSourceRow.implicitWidth + 10
                    radius: 10.5
                    color: Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, Theme.dark ? 0.12 : 0.08)
                    border.width: card.hasMultipleSources ? 1 : 0
                    border.color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.28)

                    // Single source badge when only 1 device has media
                    Row {
                        id: singleSourceRow
                        visible: !card.hasMultipleSources
                        anchors.centerIn: parent
                        spacing: 4
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 11; height: 11
                            path: card.isLocalPc ? Icons.desktop : Icons.phone
                            color: Theme.primary
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "caption"
                            size: 10
                            weight: 600
                            color: Theme.surfaceContent
                            text: card.isLocalPc ? qsTr("This PC") : qsTr("Phone")
                        }
                    }

                    // Dual source segmented switcher when both Phone and PC have media
                    Row {
                        id: dualSourceRow
                        visible: card.hasMultipleSources
                        anchors.centerIn: parent
                        spacing: 2

                        Rectangle {
                            height: 17
                            width: phoneSegRow.implicitWidth + 10
                            radius: 8.5
                            color: !card.isLocalPc ? Theme.primary : "transparent"
                            Behavior on color { ColorAnimation { duration: 150 } }
                            Row {
                                id: phoneSegRow
                                anchors.centerIn: parent
                                spacing: 3.5
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 10; height: 10
                                    path: Icons.phone
                                    color: !card.isLocalPc ? Theme.primaryContent : Theme.surfaceContentVariant
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "caption"
                                    size: 10
                                    weight: 600
                                    color: !card.isLocalPc ? Theme.primaryContent : Theme.surfaceContentVariant
                                    text: qsTr("Phone")
                                }
                            }
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: card.switchSource("phone") }
                        }

                        Rectangle {
                            height: 17
                            width: pcSegRow.implicitWidth + 10
                            radius: 8.5
                            color: card.isLocalPc ? Theme.primary : "transparent"
                            Behavior on color { ColorAnimation { duration: 150 } }
                            Row {
                                id: pcSegRow
                                anchors.centerIn: parent
                                spacing: 3.5
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 10; height: 10
                                    path: Icons.desktop
                                    color: card.isLocalPc ? Theme.primaryContent : Theme.surfaceContentVariant
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "caption"
                                    size: 10
                                    weight: 600
                                    color: card.isLocalPc ? Theme.primaryContent : Theme.surfaceContentVariant
                                    text: qsTr("PC")
                                }
                            }
                            HoverHandler { cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: card.switchSource("local") }
                        }
                    }
                }

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(36, parent.width - eqBars.width - sourcePill.width - 16)
                    role: "caption"
                    weight: 600
                    color: card.playing ? Theme.primary : Theme.surfaceContentVariant
                    text: card.app.length > 0 ? card.app : qsTr("Now playing")
                    elide: Text.ElideRight
                }
            }

            Row {
                id: rightHeaderActions
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                // Play on phone: this PC's player, on the phone (streamed, or in the service's app).
                IconButton {
                    visible: card.isLocalPc && MediaList.playOnPhoneName !== ""
                    width: 24; height: 24
                    iconSize: 13
                    iconPath: Icons.phone
                    iconColor: Theme.surfaceContentVariant
                    label: qsTr("Play on %1").arg(MediaList.playOnPhoneName)
                    onClicked: card.send("play_on_phone")
                }

                IconButton {
                    visible: !Theme.graphite
                    width: 24; height: 24
                    iconSize: 13
                    tonal: Theme.isArtworkSeed
                    iconPath: Icons.colorSwatch
                    iconColor: Theme.isArtworkSeed ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                    label: Theme.isArtworkSeed
                        ? qsTr("Artwork color sync is on (click to turn off)")
                        : qsTr("Sync app colors & blur to playing artwork")
                    onClicked: {
                        Preferences.seed = Theme.isArtworkSeed ? "wallpaper" : "artwork"
                    }
                }

                IconButton {
                    width: 24; height: 24
                    iconSize: 13
                    iconPath: card.expanded ? Icons.chevronUp : Icons.chevronDown
                    iconColor: Theme.surfaceContentVariant
                    label: card.expanded ? qsTr("Compact player") : qsTr("More controls")
                    onClicked: card.expanded = !card.expanded
                }
            }
        }

        // ---- This PC's players, when it has several: pick the one to control (#665) ----
        Flow {
            width: parent.width
            spacing: 6
            visible: card.isLocalPc && card.pcPlayers.length > 1
            Repeater {
                model: card.isLocalPc && card.pcPlayers.length > 1 ? card.pcPlayers : []
                delegate: Rectangle {
                    id: playerChip
                    required property var modelData
                    readonly property bool current: modelData.id === card.playerId
                    readonly property string name: modelData.app && modelData.app.length > 0 ? modelData.app : qsTr("Now playing")
                    height: 24
                    width: Math.min(card.width, chipRow.implicitWidth + 20)
                    radius: 12
                    color: current ? Theme.primary
                        : Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, chipHover.hovered ? 0.14 : 0.08)
                    border.width: Theme.focusVisible(playerChip) ? 2 : 0
                    border.color: Theme.primary
                    activeFocusOnTab: true
                    Accessible.role: Accessible.Button
                    Accessible.name: name
                    Accessible.checkable: true
                    Accessible.checked: current
                    Accessible.onPressAction: card.playerSelected(modelData.id)
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            card.playerSelected(modelData.id)
                            event.accepted = true
                        }
                    }
                    Row {
                        id: chipRow
                        anchors.centerIn: parent
                        spacing: 5
                        Rectangle {
                            visible: playerChip.modelData.playing
                            anchors.verticalCenter: parent.verticalCenter
                            width: 6; height: 6; radius: 3
                            color: playerChip.current ? Theme.primaryContent : Theme.primary
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            width: Math.min(implicitWidth, card.width - 40)
                            role: "caption"
                            size: 11
                            weight: 600
                            elide: Text.ElideRight
                            color: playerChip.current ? Theme.primaryContent : Theme.surfaceContent
                            text: playerChip.name
                        }
                    }
                    HoverHandler { id: chipHover; cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: card.playerSelected(playerChip.modelData.id) }
                }
            }
        }

        // ---- Main track row (cover + metadata + inline controls when compact) ----
        Item {
            width: parent.width
            height: Math.max(cover.height, metaCol.height)

            Rectangle {
                id: cover
                width: card.expanded ? 64 : 46
                height: width
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                radius: card.expanded ? Theme.radiusMd : Theme.radiusSm
                color: (card.art.length > 0 && artwork.status === Image.Ready) ? "transparent" : Theme.secondaryContainer
                scale: card.playing ? 1.0 : 0.95
                Behavior on width {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 220; easing.type: Easing.OutCubic }
                }
                Behavior on scale {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 200; easing.type: Easing.OutBack }
                }

                Icon {
                    anchors.centerIn: parent
                    width: card.expanded ? 24 : 18
                    height: width
                    visible: !(card.art.length > 0 && artwork.status === Image.Ready)
                    path: Icons.music
                    color: Theme.secondaryContainerContent
                }
                RoundedImage {
                    id: artwork
                    anchors.fill: parent
                    radius: cover.radius
                    source: card.art
                    sourceSize: Qt.size(160, 160)
                }
                TapHandler {
                    gesturePolicy: TapHandler.ReleaseWithinBounds
                    onTapped: card.expanded = !card.expanded
                }
            }

            Column {
                id: metaCol
                anchors.left: cover.right
                anchors.leftMargin: 11
                anchors.right: compactControls.visible ? compactControls.left : parent.right
                anchors.rightMargin: compactControls.visible ? 6 : 0
                anchors.verticalCenter: parent.verticalCenter
                spacing: card.expanded ? 3 : 1
                transform: Translate { id: metaSlide; x: 0 }

                ParallelAnimation {
                    id: trackSwitchAnim
                    NumberAnimation {
                        target: metaCol
                        property: "opacity"
                        from: 0.25
                        to: 1.0
                        duration: 210
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        target: metaSlide
                        property: "x"
                        from: 8
                        to: 0
                        duration: 220
                        easing.type: Easing.OutCubic
                    }
                }

                Txt {
                    width: parent.width
                    role: "title"
                    size: card.expanded ? 14 : 13
                    text: card.title.length > 0 ? card.title : card.app
                    elide: Text.ElideRight
                    maximumLineCount: card.expanded ? 2 : 1
                    wrapMode: card.expanded ? Text.WordWrap : Text.NoWrap
                }
                Txt {
                    width: parent.width
                    visible: card.artist.length > 0
                    role: "bodySmall"
                    size: card.expanded ? 12 : 11
                    muted: true
                    text: card.artist
                    elide: Text.ElideRight
                }
            }

            // Inline compact transport controls (shown when compact; -10s/+10s appear in expanded mode)
            Row {
                id: compactControls
                visible: !card.expanded
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28; height: 28
                    iconSize: 14
                    iconPath: Icons.skipPrevious
                    label: qsTr("Previous")
                    enabled: card.canPrevious
                    onClicked: card.send("previous")
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 34; height: 34
                    iconSize: 16
                    tonal: true
                    iconPath: card.playing ? Icons.pause : Icons.play
                    label: card.playing ? qsTr("Pause") : qsTr("Play")
                    enabled: card.playing ? card.canPause : card.canPlay
                    onClicked: card.send(card.playing ? "pause" : "play")
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28; height: 28
                    iconSize: 14
                    iconPath: Icons.skipNext
                    label: qsTr("Next")
                    enabled: card.canNext
                    onClicked: card.send("next")
                }
            }
        }

        // ---- Animated Seek bar with traveling shimmer & glowing playhead ----
        Column {
            width: parent.width
            spacing: 2
            visible: card.duration > 0

            Item {
                id: bar
                width: parent.width
                height: card.expanded ? 16 : 13
                activeFocusOnTab: card.canSeek
                Accessible.role: Accessible.Slider
                Accessible.name: qsTr("Playback position")
                Keys.onPressed: (event) => {
                    if (!card.canSeek || card.duration <= 0) return
                    if (event.key === Qt.Key_Left) {
                        card.skipBy(-5000)
                        event.accepted = true
                    } else if (event.key === Qt.Key_Right) {
                        card.skipBy(5000)
                        event.accepted = true
                    }
                }
                readonly property real fraction: card.duration > 0 ? Math.max(0, Math.min(1, card.shown / card.duration)) : 0
                readonly property bool activeHover: card.canSeek && (seek.containsMouse || seek.pressed || Theme.focusVisible(bar))

                Rectangle {
                    id: track
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width
                    height: bar.activeHover ? 7 : 4.5
                    radius: height / 2
                    color: Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, Theme.dark ? 0.16 : 0.12)
                    Behavior on height {
                        enabled: !Theme.reduceMotion
                        NumberAnimation { duration: 150; easing.type: Easing.OutBack }
                    }

                    Rectangle {
                        id: progressFill
                        width: Math.max(parent.height, parent.width * bar.fraction)
                        visible: bar.fraction > 0
                        height: parent.height
                        radius: parent.radius
                        color: Theme.primary
                    }
                }

                Rectangle {
                    id: seekThumb
                    visible: card.canSeek
                    scale: bar.activeHover ? (seek.pressed ? 1.22 : 1.0) : (card.playing ? 0.65 : 0.0)
                    opacity: bar.activeHover ? 1.0 : (card.playing ? 0.85 : 0.0)
                    x: Math.max(0, Math.min(bar.width - width, (bar.width * bar.fraction) - width / 2))
                    anchors.verticalCenter: parent.verticalCenter
                    width: 12; height: 12; radius: 6
                    color: Theme.primary
                    border.width: 2
                    border.color: Theme.surface
                    Behavior on scale {
                        enabled: !Theme.reduceMotion
                        NumberAnimation { duration: 150; easing.type: Easing.OutBack }
                    }
                    Behavior on opacity {
                        enabled: !Theme.reduceMotion
                        NumberAnimation { duration: 120 }
                    }
                }

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -3
                    radius: Theme.radiusXs + 3
                    color: "transparent"
                    border.width: 2
                    border.color: Theme.primary
                    visible: Theme.focusVisible(bar)
                }

                MouseArea {
                    id: seek
                    anchors.fill: parent
                    anchors.margins: -4
                    enabled: card.canSeek
                    hoverEnabled: true
                    cursorShape: card.canSeek ? Qt.PointingHandCursor : Qt.ArrowCursor
                    function at(x) { return Math.max(0, Math.min(1, (x - 4) / bar.width)) * card.duration }
                    onPressed: (mouse) => card.dragTo = at(mouse.x)
                    onPositionChanged: (mouse) => { if (pressed) card.dragTo = at(mouse.x) }
                    onReleased: (mouse) => {
                        card.send("seek", Math.round(at(mouse.x)))
                        card.dragTo = -1
                    }
                    onCanceled: card.dragTo = -1
                }
            }

            Item {
                width: parent.width
                height: elapsedText.height
                Txt { id: elapsedText; role: "caption"; size: 10; muted: true; text: card.clockFromSec(card.shownSeconds) }
                // Remaining, the whole length, or both (#658); a click goes to the next.
                Txt {
                    id: timeText
                    readonly property string mode: ["remaining", "total", "both"].indexOf(Preferences.mediaTime) >= 0
                        ? Preferences.mediaTime : "remaining"
                    readonly property string remaining: "-" + card.clockFromSec(card.remainingSeconds)
                    readonly property string total: card.clockFromSec(Math.floor(card.duration / 1000))
                    anchors.right: parent.right
                    role: "caption"
                    size: 10
                    muted: !timeMouse.containsMouse
                    text: mode === "total" ? total : mode === "both" ? remaining + " / " + total : remaining
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("Time shown: %1. Click to switch").arg(text)
                    Accessible.onPressAction: timeText.next()
                    function next() {
                        Preferences.mediaTime = mode === "remaining" ? "total" : mode === "total" ? "both" : "remaining"
                    }
                    MouseArea {
                        id: timeMouse
                        anchors.fill: parent
                        anchors.margins: -4
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: timeText.next()
                    }
                }
            }
        }

        // ---- Expanded transport + volume + audio output controls ----
        Column {
            width: parent.width
            spacing: 8
            visible: card.expanded
            opacity: card.expanded ? 1.0 : 0.0
            Behavior on opacity {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 10

                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 32; height: 32
                    iconSize: 15
                    iconPath: Icons.rewind10
                    label: qsTr("Rewind 10 seconds")
                    enabled: card.canSeek && card.duration > 0
                    onClicked: card.skipBy(-10000)
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 34; height: 34
                    iconPath: Icons.skipPrevious
                    label: qsTr("Previous")
                    enabled: card.canPrevious
                    onClicked: card.send("previous")
                }
                IconButton {
                    id: playPause
                    anchors.verticalCenter: parent.verticalCenter
                    width: 42; height: 42
                    tonal: true
                    iconPath: card.playing ? Icons.pause : Icons.play
                    label: card.playing ? qsTr("Pause") : qsTr("Play")
                    enabled: card.playing ? card.canPause : card.canPlay
                    onClicked: card.send(card.playing ? "pause" : "play")
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 34; height: 34
                    iconPath: Icons.skipNext
                    label: qsTr("Next")
                    enabled: card.canNext
                    onClicked: card.send("next")
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 32; height: 32
                    iconSize: 15
                    iconPath: Icons.forward10
                    label: qsTr("Forward 10 seconds")
                    enabled: card.canSeek && card.duration > 0
                    onClicked: card.skipBy(10000)
                }
            }

            // Inline Volume slider + Phone Speaker audio routing pill
            Item {
                width: parent.width
                height: 26

                IconButton {
                    id: muteBtn
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: 26; height: 26
                    iconSize: 14
                    iconPath: card.currentMuted ? Icons.soundOff : Icons.speaker
                    label: card.currentMuted ? qsTr("Unmute") : qsTr("Mute")
                    onClicked: card.send("mute", card.currentVolume)
                }

                Item {
                    id: volBar
                    anchors.left: muteBtn.right
                    anchors.leftMargin: 6
                    anchors.right: speakerRoutePill.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    height: 16
                    readonly property real volFraction: card.currentMuted ? 0 : Math.max(0, Math.min(1, card.currentVolume / 100))

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width
                        height: volMouse.containsMouse || volMouse.pressed ? 6 : 4
                        radius: height / 2
                        color: Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, Theme.dark ? 0.16 : 0.12)
                        Behavior on height { NumberAnimation { duration: 120 } }

                        Rectangle {
                            width: parent.width * volBar.volFraction
                            height: parent.height
                            radius: parent.radius
                            color: Theme.primary
                        }
                    }

                    Timer {
                        id: volSettleTimer
                        interval: 400
                        onTriggered: card.dragVolume = -1
                    }

                    MouseArea {
                        id: volMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        function levelAt(x) { return Math.round(Math.max(0, Math.min(1, x / volBar.width)) * 100) }
                        onPressed: (mouse) => {
                            volSettleTimer.stop()
                            card.dragVolume = levelAt(mouse.x)
                        }
                        onPositionChanged: (mouse) => { if (pressed) card.dragVolume = levelAt(mouse.x) }
                        onReleased: (mouse) => {
                            const lvl = levelAt(mouse.x)
                            card.dragVolume = lvl
                            card.send("volume", lvl)
                            volSettleTimer.restart()
                        }
                        onCanceled: card.dragVolume = -1
                    }
                }

                Rectangle {
                    id: speakerRoutePill
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    height: 22
                    width: speakerRouteRow.implicitWidth + 12
                    radius: 11
                    color: Preferences.streamPcAudioToPhone
                        ? Theme.secondaryContainer
                        : Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, Theme.dark ? 0.10 : 0.07)
                    border.width: Preferences.streamPcAudioToPhone ? 1 : 0
                    border.color: Theme.primary

                    Row {
                        id: speakerRouteRow
                        anchors.centerIn: parent
                        spacing: 4
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 11; height: 11
                            path: Preferences.streamPcAudioToPhone ? Icons.phone : Icons.speaker
                            color: Preferences.streamPcAudioToPhone ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "caption"
                            size: 10
                            weight: 600
                            color: Preferences.streamPcAudioToPhone ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                            text: Preferences.streamPcAudioToPhone ? qsTr("Phone speaker") : qsTr("PC speaker")
                        }
                    }
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: Preferences.streamPcAudioToPhone = !Preferences.streamPcAudioToPhone }
                }
            }
        }

        // ---- Interactive Carousel Pager Dots when both Phone and PC have active media ----
        Row {
            visible: card.hasMultipleSources
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 5
            height: 7

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: !card.isLocalPc ? 14 : 5.5
                height: 5.5
                radius: 2.75
                color: !card.isLocalPc
                    ? Theme.primary
                    : Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.28)
                Behavior on width { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
                Behavior on color { ColorAnimation { duration: 180 } }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: card.switchSource("phone") }
            }

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: card.isLocalPc ? 14 : 5.5
                height: 5.5
                radius: 2.75
                color: card.isLocalPc
                    ? Theme.primary
                    : Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.28)
                Behavior on width { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
                Behavior on color { ColorAnimation { duration: 180 } }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
                TapHandler { onTapped: card.switchSource("local") }
            }
        }
    }
}


