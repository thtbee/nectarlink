// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import app.nectarlink

// The part of Nectar Island that grows under the pill while it is hovered or
// pinned. It lives in the Island's own window, so hovering stretches the
// Island itself instead of opening a second, detached window.
Item {
    id: card
    required property var island

    readonly property bool light: island ? island.light : false
    readonly property color ink: island ? island.ink : "#FFFFFF"
    function wash(alpha) { return island ? island.wash(alpha) : Qt.rgba(1, 1, 1, alpha) }
    // The Island settings page uses the main Settings catalogue for its labels,
    // so every Island row reads in all languages without a second set.
    function st(text) { return qsTranslate("SettingsPage", text) }
    readonly property bool settingsOpen: island ? island.islandSettingsOpen : false

    function triggerOn(key) {
        if (key === "calls") return Preferences.nectarIslandCalls
        if (key === "transfers") return Preferences.nectarIslandTransfers
        if (key === "media") return Preferences.nectarIslandMedia
        if (key === "pc") return Preferences.nectarIslandPcMedia
        return Preferences.nectarIslandNotifications
    }
    function setTrigger(key, on) {
        if (key === "calls") Preferences.nectarIslandCalls = on
        else if (key === "transfers") Preferences.nectarIslandTransfers = on
        else if (key === "media") Preferences.nectarIslandMedia = on
        else if (key === "pc") Preferences.nectarIslandPcMedia = on
        else Preferences.nectarIslandNotifications = on
    }

    property int clockTick: 0
    // Ages the notifications' relative times while the card is open.
    Timer {
        interval: 60000
        repeat: true
        running: visible
        onTriggered: card.clockTick++
    }

    width: 340
    // The card height is fixed by the open Island (eight compact heights).
    // Its body scrolls instead of resizing the window.
    readonly property real cardHeight: island ? island.expandedCardHeight : 286
    implicitHeight: cardHeight
    height: implicitHeight

    Rectangle {
        anchors.fill: parent
        radius: 20
        // The pill's stretched surface is behind both; this only catches
        // rounding for standalone previews.
        color: "transparent"
        border.width: 0
    }

    HoverHandler {
        onHoveredChanged: if (card.island) card.island.panelHovered = hovered
    }

    Flickable {
        id: bodyFlick
        x: 12
        y: 8
        width: parent.width - 24
        height: parent.height - 16
        contentHeight: body.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        visible: !card.settingsOpen

        Column {
            id: body
            width: bodyFlick.width
            spacing: 8

        // Grab handle and the two Island actions, all icon-sized.
        Item {
            width: parent.width
            height: 26

            Rectangle {
                width: 28
                height: 3
                radius: 1.5
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                color: card.wash(0.22)
            }

            // Follow this phone or this PC for the current song, icons only.
            Row {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                visible: card.island && card.island.mode === "media"

                SourceIcon {
                    iconPath: Icons.phone
                    label: qsTr("Phone")
                    active: card.island.mediaDeviceId !== "local"
                    available: card.island.phoneSourcePlaying
                    onClicked: card.island.pickMediaSource("phone")
                }
                SourceIcon {
                    iconPath: Icons.desktop
                    label: qsTr("PC")
                    active: card.island.mediaDeviceId === "local"
                    available: card.island.pcSourcePlaying
                    onClicked: card.island.pickMediaSource("pc")
                }
            }

            Row {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                CardIconButton {
                    visible: card.island && !card.island.onWayland
                    iconPath: Icons.target
                    label: qsTr("Reset position")
                    onClicked: card.island.resetIslandPosition()
                }
                CardIconButton {
                    iconPath: Icons.home
                    label: qsTr("Open NectarLink")
                    onClicked: card.island.openMain()
                }
                CardIconButton {
                    iconPath: Icons.settings
                    label: card.st("Settings")
                    onClicked: card.island.openIslandSettings()
                }
                CardIconButton {
                    iconPath: Icons.close
                    label: qsTr("Close")
                    onClicked: card.island.closePanel()
                }
            }
        }

        // Side docks have no pill above the card, so the card carries the
        // now-playing row, its controls and the progress bar.
        Column {
            visible: card.island && card.island.sideDock
            width: parent.width
            spacing: 7

            Row {
                width: parent.width
                spacing: 10

                Item {
                    width: 40
                    height: 40
                    anchors.verticalCenter: parent.verticalCenter

                    Rectangle {
                        anchors.fill: parent
                        radius: 10
                        visible: !(card.island.mode === "media"
                            && card.island.mediaArt.length > 0)
                        color: card.wash(0.10)
                        Icon {
                            anchors.centerIn: parent
                            width: 18
                            height: 18
                            path: card.island.mode === "call" ? Icons.call
                                : card.island.mode === "transfer" ? Icons.send
                                : Icons.music
                            color: card.wash(0.70)
                        }
                    }
                    RoundedImage {
                        anchors.fill: parent
                        radius: 10
                        visible: card.island.mode === "media" && card.island.mediaArt.length > 0
                        source: card.island.mediaArt
                        sourceSize: Qt.size(96, 96)
                    }
                }

                Column {
                    width: parent.width - 40 - 10 - sideControls.width - 10
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2

                    Txt {
                        width: parent.width
                        text: card.island.primaryLabel
                        role: "title"
                        size: 13
                        weight: Font.DemiBold
                        color: card.ink
                        elide: Text.ElideRight
                    }
                    Txt {
                        width: parent.width
                        text: card.island.secondaryLabel
                        role: "caption"
                        size: 11
                        color: card.wash(0.68)
                        elide: Text.ElideRight
                    }
                }

                Row {
                    id: sideControls
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    CardIconButton {
                        visible: card.island.mode === "media" && card.island.mediaTitle.length > 0
                        iconPath: card.island.mediaPlaying ? Icons.pause : Icons.play
                        label: card.island.mediaPlaying ? qsTr("Pause") : qsTr("Play")
                        onClicked: MediaList.command(card.island.mediaDeviceId,
                            card.island.mediaPlayerId,
                            card.island.mediaPlaying ? "pause" : "play")
                    }
                    CardIconButton {
                        visible: card.island.mode === "media" && card.island.mediaCanNext
                        iconPath: Icons.skipNext
                        label: qsTr("Next track")
                        onClicked: MediaList.command(card.island.mediaDeviceId,
                            card.island.mediaPlayerId, "next")
                    }
                }
            }

            Rectangle {
                visible: card.island.mode === "media" && card.island.mediaDuration > 0
                width: parent.width
                height: 3
                radius: 1.5
                color: card.wash(0.14)

                Rectangle {
                    height: parent.height
                    radius: parent.radius
                    color: Theme.primary
                    width: parent.width * Math.max(0, Math.min(1,
                        card.island.mediaPositionNow / card.island.mediaDuration))
                    Behavior on width {
                        enabled: !Theme.reduceMotion
                        NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                    }
                }
            }
        }

        // ---- Scrolling lyrics ----
        // The pill keeps one always-visible line; opening the Island gives the
        // whole lyric a viewport that follows the song.
        LyricsScroll {
            visible: card.island && card.island.mode === "media"
                && card.island.lyricLines.length > 0
            width: parent.width
            lines: card.island.lyricLines
            current: card.island.lyricIndex
            progress: card.island.lyricProgress
            baseColor: card.wash(0.55)
            fillColor: Theme.primary
        }

        Txt {
            visible: card.island && card.island.mode === "media"
                && card.island.lyricLines.length === 0
                && card.island.lyricUnavailable && !card.island.lyricLoading
            width: parent.width
            text: qsTr("No text available for this media.")
            role: "caption"
            size: 11
            color: card.wash(0.50)
            horizontalAlignment: Text.AlignHCenter
        }

        // ---- Phone notifications ----
        RowLayout {
            width: parent.width
            spacing: 6

            Txt {
                text: qsTr("Phone notifications")
                role: "label"
                size: 12
                weight: Font.DemiBold
                color: card.wash(0.62)
            }

            Rectangle {
                visible: NotificationList.count > 0
                Layout.preferredWidth: countLabel.implicitWidth + 12
                Layout.preferredHeight: 18
                radius: 9
                color: card.wash(0.10)
                Txt {
                    id: countLabel
                    anchors.centerIn: parent
                    text: String(NotificationList.count)
                    role: "mono"
                    size: 10
                    weight: Font.DemiBold
                    color: card.ink
                }
            }

            Item { Layout.fillWidth: true }

            Txt {
                visible: Preferences.nectarIslandNotifications && NotificationList.count > 0
                text: qsTr("Clear")
                role: "caption"
                size: 11
                color: clearMouse.containsMouse ? card.ink : card.wash(0.55)
                MouseArea {
                    id: clearMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: NotificationList.clearIsland()
                }
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Clear all notifications from the Island")
            }
        }

        RowLayout {
            visible: !Preferences.nectarIslandNotifications
            width: parent.width
            spacing: 8

            Icon {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                path: Icons.bell
                color: card.wash(0.55)
            }
            Txt {
                Layout.fillWidth: true
                text: qsTr("Phone alerts are off for the Island.")
                role: "caption"
                size: 11
                color: card.wash(0.55)
                wrapMode: Text.WordWrap
            }
            CardButton {
                iconPath: Icons.bell
                label: qsTr("Turn on")
                onClicked: Preferences.nectarIslandNotifications = true
            }
        }

        RowLayout {
            visible: Preferences.nectarIslandNotifications && AppController.notificationsPaused
            width: parent.width
            spacing: 8

            Icon {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                path: Icons.moon
                color: card.wash(0.55)
            }
            Txt {
                Layout.fillWidth: true
                text: qsTr("Notifications are paused.")
                role: "caption"
                size: 11
                color: card.wash(0.55)
            }
            CardButton {
                iconPath: Icons.bell
                label: qsTr("Resume")
                onClicked: AppController.toggleNotificationsPaused()
            }
        }

        Item {
            id: listBox
            visible: Preferences.nectarIslandNotifications && NotificationList.count > 0
            width: parent.width
            height: Math.min(notifColumn.implicitHeight, 280)

            Flickable {
                id: notifFlick
                anchors.fill: parent
                contentWidth: width
                contentHeight: notifColumn.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds

                Column {
                    id: notifColumn
                    width: notifFlick.width
                    spacing: 4

                    Repeater {
                        model: NotificationList
                        delegate: CardNotificationRow {}
                    }
                }
            }

            ScrollRail {
                flick: notifFlick
                inset: -2
                thumbColor: card.wash(0.35)
            }
            WheelScroll { flick: notifFlick; notch: 96 }
        }

        RowLayout {
            visible: Preferences.nectarIslandNotifications && NotificationList.count === 0
            width: parent.width
            spacing: 8

            Icon {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                path: Icons.bell
                color: card.wash(0.42)
            }
            Txt {
                Layout.fillWidth: true
                text: NotificationList.allCount > 0
                    ? qsTr("Cleared from the Island. The main list still has them.")
                    : qsTr("No phone notifications right now.")
                role: "caption"
                size: 11
                color: card.wash(0.55)
                wrapMode: Text.WordWrap
            }
        }
        }
        WheelScroll { flick: bodyFlick; notch: 72 }
    }

    // The Island's own settings page: inside the same card and window, with
    // no second window. Leaving the pointer collapses it with the card.
    Item {
        id: settingsView
        visible: card.settingsOpen
        x: 12
        y: 8
        width: parent.width - 24
        height: settingsHeader.height + 8 + settingsFlick.height
        implicitHeight: height

        Item {
            id: settingsHeader
            width: parent.width
            height: 26

            Txt {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                text: card.st("Settings")
                role: "title"
                size: 13
                weight: Font.DemiBold
                color: card.ink
            }
            CardIconButton {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.close
                label: qsTr("Close")
                onClicked: card.island.closeIslandSettings()
            }
        }

        Flickable {
            id: settingsFlick
            y: settingsHeader.height + 8
            width: parent.width
            height: Math.max(80, settingsPanelHeight - settingsHeader.height - 8)
            contentHeight: settingsColumn.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            readonly property real settingsPanelHeight: card.island
                && card.island.islandSettingsPanelHeight > 0
                ? card.island.islandSettingsPanelHeight
                : Math.max(180, (card.island && card.island.areaH > 200
                    ? card.island.areaH : 800) - 190)

            Column {
                id: settingsColumn
                width: settingsFlick.width - 4
                spacing: 8

                Txt {
                    text: card.st("Appearance")
                    role: "caption"
                    size: 10
                    weight: Font.DemiBold
                    color: card.wash(0.45)
                }

                Txt {
                    text: card.st("Island skin")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Grid {
                    id: skinGrid
                    width: parent.width
                    columns: 3
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: [
                            { value: "classic", label: card.st("Classic") },
                            { value: "cream", label: card.st("Cream") },
                            { value: "glassLight", label: card.st("Glass light") },
                            { value: "glassDark", label: card.st("Glass dark") },
                            { value: "cloud", label: card.st("Cloud") }
                        ]
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (skinGrid.width - (skinGrid.columns - 1) * skinGrid.columnSpacing)
                                / skinGrid.columns
                            text: modelData.label
                            selected: Preferences.nectarIslandSkin === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandSkin = modelData.value
                        }
                    }
                }

                Txt {
                    text: card.st("Island shape")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Grid {
                    id: shapeGrid
                    width: parent.width
                    columns: 2
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: [
                            { value: "capsule", label: card.st("Capsule") },
                            { value: "notch", label: card.st("Notch") }
                        ]
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (shapeGrid.width - (shapeGrid.columns - 1) * shapeGrid.columnSpacing)
                                / shapeGrid.columns
                            text: modelData.label
                            selected: Preferences.nectarIslandShape === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandShape = modelData.value
                        }
                    }
                }

                Txt {
                    text: card.st("Island size")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Row {
                    width: parent.width
                    spacing: 6

                    Txt {
                        width: parent.width - 110
                        text: card.st("%1%").arg(Preferences.nectarIslandSize)
                        role: "caption"
                        size: 11
                        color: card.wash(0.72)
                    }
                    Rectangle {
                        id: sizeMinus
                        width: 26; height: 26; radius: 13
                        color: sizeMinusMouse.containsMouse ? card.wash(0.16) : card.wash(0.08)
                        Txt {
                            anchors.centerIn: parent
                            text: "−"
                            role: "label"
                            size: 14
                            color: card.ink
                        }
                        MouseArea {
                            id: sizeMinusMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: Preferences.nectarIslandSize =
                                Math.max(80, Preferences.nectarIslandSize - 5)
                        }
                        IslandTipHover { island: card.island; tip: qsTr("Smaller"); owner: sizeMinus }
                    }
                    Txt {
                        width: 40
                        height: 26
                        text: card.st("%1%").arg(Preferences.nectarIslandSize)
                        role: "caption"
                        size: 11
                        weight: Font.DemiBold
                        color: card.ink
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    Rectangle {
                        id: sizePlus
                        width: 26; height: 26; radius: 13
                        color: sizePlusMouse.containsMouse ? card.wash(0.16) : card.wash(0.08)
                        Txt {
                            anchors.centerIn: parent
                            text: "+"
                            role: "label"
                            size: 14
                            color: card.ink
                        }
                        MouseArea {
                            id: sizePlusMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: Preferences.nectarIslandSize =
                                Math.min(140, Preferences.nectarIslandSize + 5)
                        }
                        IslandTipHover { island: card.island; tip: qsTr("Larger"); owner: sizePlus }
                    }
                }

                Txt {
                    text: card.st("Dock edge")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Grid {
                    id: edgeGrid
                    width: parent.width
                    columns: 3
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: [
                            { value: "top", label: card.st("Top") },
                            { value: "left", label: card.st("Left") },
                            { value: "right", label: card.st("Right") }
                        ]
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (edgeGrid.width - (edgeGrid.columns - 1) * edgeGrid.columnSpacing)
                                / edgeGrid.columns
                            text: modelData.label
                            selected: Preferences.nectarIslandEdge === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandEdge = modelData.value
                        }
                    }
                }

                Txt {
                    text: card.st("Island display")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Grid {
                    id: displayGrid
                    width: parent.width
                    columns: 2
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: {
                            const list = [{ value: "", label: card.st("Primary") }]
                            const screens = Qt.application.screens || []
                            for (let i = 0; i < screens.length; i++) {
                                if (screens[i].name.length > 0)
                                    list.push({ value: screens[i].name, label: screens[i].name })
                            }
                            return list
                        }
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (displayGrid.width - (displayGrid.columns - 1) * displayGrid.columnSpacing)
                                / displayGrid.columns
                            text: modelData.label
                            selected: Preferences.nectarIslandDisplay === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandDisplay = modelData.value
                        }
                    }
                }

                Txt {
                    text: card.st("Island opacity")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Row {
                    width: parent.width
                    spacing: 6

                    Txt {
                        width: parent.width - 110
                        text: card.st("%1%").arg(Preferences.nectarIslandOpacity)
                        role: "caption"
                        size: 11
                        color: card.wash(0.72)
                    }
                    Rectangle {
                        id: opacityMinus
                        width: 26; height: 26; radius: 13
                        color: opacityMinusMouse.containsMouse ? card.wash(0.16) : card.wash(0.08)
                        Txt {
                            anchors.centerIn: parent
                            text: "−"
                            role: "label"
                            size: 14
                            color: card.ink
                        }
                        MouseArea {
                            id: opacityMinusMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: Preferences.nectarIslandOpacity =
                                Math.max(30, Preferences.nectarIslandOpacity - 5)
                        }
                        IslandTipHover { island: card.island; tip: qsTr("More transparent"); owner: opacityMinus }
                    }
                    Txt {
                        width: 40
                        height: 26
                        text: card.st("%1%").arg(Preferences.nectarIslandOpacity)
                        role: "caption"
                        size: 11
                        weight: Font.DemiBold
                        color: card.ink
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    Rectangle {
                        id: opacityPlus
                        width: 26; height: 26; radius: 13
                        color: opacityPlusMouse.containsMouse ? card.wash(0.16) : card.wash(0.08)
                        Txt {
                            anchors.centerIn: parent
                            text: "+"
                            role: "label"
                            size: 14
                            color: card.ink
                        }
                        MouseArea {
                            id: opacityPlusMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: Preferences.nectarIslandOpacity =
                                Math.min(100, Preferences.nectarIslandOpacity + 5)
                        }
                        IslandTipHover { island: card.island; tip: qsTr("Less transparent"); owner: opacityPlus }
                    }
                }

                Row {
                    width: parent.width
                    height: 26
                    spacing: 8
                    Txt {
                        width: parent.width - 46
                        text: card.st("Clock")
                        role: "label"
                        size: 11
                        color: card.ink
                    }
                    IslandSwitch {
                        y: (parent.height - height) / 2
                        checked: Preferences.nectarIslandClock
                        label: card.st("Clock")
                        inkColor: card.ink
                        offColor: card.wash(0.18)
                        onToggled: (on) => Preferences.nectarIslandClock = on
                    }
                }
                Grid {
                    id: clockGrid
                    visible: Preferences.nectarIslandClock
                    width: parent.width
                    columns: 2
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: [
                            { value: "24", label: card.st("24-hour") },
                            { value: "12", label: card.st("12-hour") }
                        ]
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (clockGrid.width - (clockGrid.columns - 1) * clockGrid.columnSpacing)
                                / clockGrid.columns
                            text: modelData.label
                            selected: (Preferences.nectarIslandClock12h ? "12" : "24") === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandClock12h = modelData.value === "12"
                        }
                    }
                }

                Column {
                    visible: Qt.platform.os === "windows"
                    width: parent.width
                    spacing: 2
                    Row {
                        width: parent.width
                        height: 26
                        spacing: 8
                        Txt {
                            width: parent.width - 46
                            text: qsTr("Click-through")
                            role: "label"
                            size: 11
                            color: card.ink
                        }
                        IslandSwitch {
                            y: (parent.height - height) / 2
                            checked: Preferences.nectarIslandClickThrough
                            label: qsTr("Click-through")
                            inkColor: card.ink
                            offColor: card.wash(0.18)
                            onToggled: (on) => Preferences.nectarIslandClickThrough = on
                        }
                    }
                    Txt {
                        width: parent.width
                        text: qsTr("Hover still opens the Island; clicks go to the window underneath. Turn it off in NectarLink's Settings if you need to click the Island.")
                        role: "caption"
                        size: 10
                        color: card.wash(0.50)
                        wrapMode: Text.WordWrap
                    }
                }

                Rectangle {
                    width: parent.width
                    height: 1
                    color: card.wash(0.08)
                }

                Txt {
                    text: card.st("Island triggers")
                    role: "caption"
                    size: 10
                    weight: Font.DemiBold
                    color: card.wash(0.45)
                }

                Repeater {
                    model: [
                        { key: "calls", label: card.st("Calls") },
                        { key: "transfers", label: card.st("Transfers") },
                        { key: "media", label: card.st("Media") },
                        { key: "notifications", label: card.st("Alerts") },
                        { key: "pc", label: card.st("PC") }
                    ]
                    delegate: Item {
                        required property var modelData
                        width: settingsColumn.width
                        height: 26
                        Row {
                            width: parent.width
                            height: parent.height
                            spacing: 8
                            Txt {
                                width: parent.width - 46
                                text: modelData.label
                                role: "label"
                                size: 11
                                color: card.ink
                            }
                            IslandSwitch {
                                y: (parent.height - height) / 2
                                checked: card.triggerOn(modelData.key)
                                label: modelData.label
                                inkColor: card.ink
                                offColor: card.wash(0.18)
                                onToggled: (on) => card.setTrigger(modelData.key, on)
                            }
                        }
                    }
                }

                Row {
                    width: parent.width
                    height: 26
                    spacing: 8
                    Txt {
                        width: parent.width - 46
                        text: card.st("Show when idle")
                        role: "label"
                        size: 11
                        color: card.ink
                    }
                    IslandSwitch {
                        y: (parent.height - height) / 2
                        checked: Preferences.nectarIslandAlwaysVisible
                        label: card.st("Show when idle")
                        inkColor: card.ink
                        offColor: card.wash(0.18)
                        onToggled: (on) => Preferences.nectarIslandAlwaysVisible = on
                    }
                }

                Txt {
                    text: card.st("When both play")
                    role: "label"
                    size: 11
                    color: card.ink
                }
                Grid {
                    id: bothPlayGrid
                    width: parent.width
                    columns: 2
                    columnSpacing: 4
                    rowSpacing: 4
                    height: childrenRect.height
                    Repeater {
                        model: [
                            { value: "phone", label: card.st("Phone") },
                            { value: "pc", label: card.st("PC") }
                        ]
                        delegate: IslandChoiceChip {
                            required property var modelData
                            width: (bothPlayGrid.width - (bothPlayGrid.columns - 1) * bothPlayGrid.columnSpacing)
                                / bothPlayGrid.columns
                            text: modelData.label
                            selected: Preferences.nectarIslandMediaSource === modelData.value
                            baseColor: card.wash(0.08)
                            textColor: card.ink
                            onPicked: Preferences.nectarIslandMediaSource = modelData.value
                        }
                    }
                }
            }

            WheelScroll { flick: settingsFlick; notch: 72 }
        }
        // A slim indicator so it is clear the fixed panel scrolls.
        Rectangle {
            visible: settingsFlick.contentHeight > settingsFlick.height + 1
            x: settingsFlick.x + settingsFlick.width - 3
            y: settingsFlick.y + Math.max(2,
                settingsFlick.visibleArea.yPosition * settingsFlick.height)
            width: 3
            height: Math.max(18,
                settingsFlick.visibleArea.heightRatio * settingsFlick.height - 4)
            radius: 1.5
            color: card.wash(0.22)
        }
    }

    component IslandChoiceChip: Rectangle {
        id: chip
        property string text
        property bool selected: false
        property color baseColor: "transparent"
        property color textColor: "#FFFFFF"
        signal picked

        implicitWidth: chipLabel.implicitWidth + 18
        implicitHeight: 26
        width: implicitWidth
        height: implicitHeight
        radius: 13
        color: selected
            ? Theme.primary
            : (chipMouse.containsMouse
                ? Qt.rgba(textColor.r, textColor.g, textColor.b, 0.16) : baseColor)
        border.width: selected ? 0 : 1
        border.color: Qt.rgba(textColor.r, textColor.g, textColor.b, 0.18)

        Txt {
            id: chipLabel
            anchors.centerIn: parent
            width: Math.min(implicitWidth, chip.width - 12)
            text: chip.text
            role: "caption"
            size: 11
            weight: chip.selected ? Font.DemiBold : Font.Normal
            color: chip.selected ? Theme.primaryContent : chip.textColor
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
        }
        MouseArea {
            id: chipMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: chip.picked()
        }
        Accessible.role: Accessible.Button
        Accessible.name: chip.text
    }

    component IslandSwitch: Item {
        id: islandSwitch
        property bool checked: false
        property string label
        property color inkColor: "#FFFFFF"
        property color offColor: Qt.rgba(1, 1, 1, 0.18)
        signal toggled(bool on)

        implicitWidth: 38
        implicitHeight: 22
        width: implicitWidth
        height: implicitHeight

        Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: islandSwitch.checked ? Theme.primary : islandSwitch.offColor
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }

            Rectangle {
                width: 16; height: 16; radius: 8
                y: 3
                x: islandSwitch.checked ? parent.width - width - 3 : 3
                color: islandSwitch.checked ? Theme.primaryContent : islandSwitch.inkColor
                Behavior on x {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
                }
            }
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: islandSwitch.toggled(!islandSwitch.checked)
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: islandSwitch.label
        Accessible.checkable: true
        Accessible.checked: islandSwitch.checked
    }

    component SourceIcon: Rectangle {
        id: sourceIcon
        property string iconPath
        property string label
        property bool active: false
        property bool available: false
        signal clicked

        width: 26
        height: 26
        radius: 13
        opacity: available ? 1 : 0.35
        color: active
            ? Theme.primary
            : (sourceMouse.containsMouse ? card.wash(0.14) : "transparent")
        border.width: active ? 1 : 0
        border.color: active ? Theme.primary : card.wash(0.10)
        scale: sourceMouse.pressed ? 0.92 : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

        Icon {
            anchors.centerIn: parent
            width: 14
            height: 14
            path: sourceIcon.iconPath
            color: sourceIcon.active ? Theme.primaryContent : card.wash(0.62)
        }

        MouseArea {
            id: sourceMouse
            anchors.fill: parent
            enabled: sourceIcon.available
            hoverEnabled: true
            cursorShape: sourceIcon.available ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: sourceIcon.clicked()
        }
        Accessible.role: Accessible.Button
        Accessible.name: sourceIcon.label
        IslandTipHover { island: card.island; tip: sourceIcon.label; owner: sourceIcon }
    }

    component CardIconButton: Rectangle {
        id: iconButton
        property string iconPath
        property string label
        signal clicked

        width: 26
        height: 26
        radius: 13
        color: iconMouse.containsMouse ? card.wash(0.16) : "transparent"
        scale: iconMouse.pressed ? 0.92 : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

        Icon {
            anchors.centerIn: parent
            width: 13
            height: 13
            path: iconButton.iconPath
            color: card.wash(0.62)
        }
        MouseArea {
            id: iconMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: iconButton.clicked()
        }
        Accessible.role: Accessible.Button
        Accessible.name: iconButton.label
        IslandTipHover { island: card.island; tip: iconButton.label; owner: iconButton }
    }

    component CardButton: Rectangle {
        id: cardButton
        property string iconPath
        property string label
        signal clicked

        implicitWidth: buttonRow.implicitWidth + 18
        implicitHeight: 28
        radius: 14
        color: buttonMouse.containsMouse ? card.wash(0.16) : card.wash(0.08)
        border.width: 1
        border.color: card.wash(0.10)

        Row {
            id: buttonRow
            anchors.centerIn: parent
            spacing: 5
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                width: 12
                height: 12
                path: cardButton.iconPath
                color: card.ink
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: cardButton.label
                role: "label"
                size: 11
                weight: Font.DemiBold
                color: card.ink
            }
        }
        MouseArea {
            id: buttonMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: cardButton.clicked()
        }
        IslandTipHover { island: card.island; tip: cardButton.label; owner: cardButton }
    }

    component CardNotificationRow: Item {
        id: row
        required property string deviceId
        required property string key
        required property string app
        required property string appName
        required property string title
        required property string text
        required property string sub
        required property real when
        required property string iconUrl
        required property string actions
        required property string replyAction
        required property string replyLabel
        required property string otpCode
        required property string live
        required property bool ongoing
        required property int groupCount
        required property int islandGroupCount
        required property bool islandCleared
        required property int groupIndex
        required property bool groupExpanded
        visible: !islandCleared

        readonly property var buttons: {
            try {
                const parsed = JSON.parse(actions)
                return Array.isArray(parsed) ? parsed : []
            } catch (e) {
                return []
            }
        }
        readonly property var liveData: {
            if (!live || live.length === 0)
                return null
            try { return JSON.parse(live) } catch (e) { return null }
        }
        readonly property bool hasLiveProgress: liveData !== null
            && (Boolean(liveData.indeterminate) || Number(liveData.max) > 0)
        readonly property real liveRatio: {
            if (!hasLiveProgress)
                return 0
            if (liveData.indeterminate)
                return 0.35
            const max = Math.max(1, Number(liveData.max || 1))
            return Math.max(0, Math.min(1, Number(liveData.progress || 0) / max))
        }
        readonly property bool isChild: groupIndex > 0
        readonly property string appLabel: appName.length > 0 ? appName : app

        function timeText() {
            card.clockTick
            const minutes = Math.round((Date.now() - when) / 60000)
            if (minutes < 1) return qsTr("now")
            if (minutes < 60) return qsTr("%n min", "", minutes)
            const date = new Date(when)
            if (Date.now() - when < 24 * 3600 * 1000)
                return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
            return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
        }

        width: parent ? parent.width : 0
        implicitHeight: rowContent.implicitHeight + 12
        height: implicitHeight

        Rectangle {
            anchors.fill: parent
            radius: 12
            color: rowHover.hovered ? card.wash(0.055) : "transparent"
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        }
        HoverHandler { id: rowHover }

        Rectangle {
            visible: row.isChild
            x: 10
            y: 0
            width: 2
            height: parent.height
            radius: 1
            color: card.wash(0.10)
        }

        Row {
            id: rowContent
            x: row.isChild ? 19 : 8
            y: 6
            width: parent.width - x - 8
            spacing: 8

            Item {
                width: 24
                height: 24

                Rectangle {
                    anchors.fill: parent
                    radius: 7
                    color: card.wash(0.10)
                }
                RoundedImage {
                    id: rowIcon
                    anchors.fill: parent
                    radius: 7
                    source: row.iconUrl
                    sourceSize: Qt.size(48, 48)
                }
                Txt {
                    anchors.centerIn: parent
                    visible: rowIcon.status !== Image.Ready
                    text: row.appLabel.substring(0, 1).toUpperCase()
                    role: "label"
                    size: 11
                    weight: Font.DemiBold
                    color: card.ink
                }
            }

            Column {
                width: parent.width - 24 - closeButton.width - parent.spacing * 2
                spacing: 2

                Row {
                    width: parent.width
                    spacing: 5

                    Txt {
                        width: Math.max(30, parent.width - timeLabel.implicitWidth
                            - (groupBadge.visible ? groupBadge.width + 5 : 0) - 10)
                        text: row.sub.length > 0 ? row.appLabel + " · " + row.sub : row.appLabel
                        role: "caption"
                        size: 10
                        weight: Font.DemiBold
                        color: card.wash(0.66)
                        elide: Text.ElideRight
                    }
                    Txt {
                        id: timeLabel
                        text: row.timeText()
                        role: "caption"
                        size: 10
                        color: card.wash(0.40)
                    }
                    Rectangle {
                        id: groupBadge
                        visible: row.islandGroupCount > 1 && row.groupIndex === 0
                        height: 15
                        width: groupBadgeRow.implicitWidth + 8
                        radius: 7.5
                        color: card.wash(0.10)
                        Row {
                            id: groupBadgeRow
                            anchors.centerIn: parent
                            spacing: 2
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                text: String(row.islandGroupCount)
                                role: "mono"
                                size: 9
                                weight: Font.DemiBold
                                color: card.wash(0.66)
                            }
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 8
                                height: 8
                                path: row.groupExpanded ? Icons.chevronUp : Icons.chevronDown
                                color: card.wash(0.52)
                            }
                        }
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: NotificationList.toggleGroup(row.app)
                        }
                    }
                }

                Txt {
                    width: parent.width
                    visible: row.title.length > 0
                    text: row.title
                    role: "body"
                    size: 11.5
                    weight: Font.DemiBold
                    color: card.ink
                    elide: Text.ElideRight
                }

                Txt {
                    width: parent.width
                    visible: row.text.length > 0
                    text: row.text
                    role: "bodySmall"
                    size: 10.5
                    color: card.wash(0.60)
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }

                Rectangle {
                    visible: row.hasLiveProgress
                    width: parent.width
                    height: 3
                    radius: 1.5
                    color: card.wash(0.12)
                    Rectangle {
                        width: parent.width * row.liveRatio
                        height: parent.height
                        radius: parent.radius
                        color: Theme.primary
                    }
                }

                Flow {
                    width: parent.width
                    spacing: 5
                    visible: row.buttons.length > 0 || row.replyAction.length > 0 || row.otpCode.length > 0

                    Repeater {
                        model: row.buttons
                        delegate: ActionChip {
                            required property var modelData
                            label: modelData.title
                            onActivated: NotificationList.runAction(row.deviceId, row.key, modelData.id)
                        }
                    }

                    ActionChip {
                        visible: row.replyAction.length > 0
                        label: row.replyLabel.length > 0 ? row.replyLabel : qsTr("Reply")
                        onActivated: card.island.openMain()
                    }

                    ActionChip {
                        visible: row.otpCode.length > 0
                        label: row.otpCode
                        mono: true
                        onActivated: NotificationList.copyCode(row.otpCode)
                    }
                }
            }

            Rectangle {
                id: closeButton
                width: visible ? 18 : 0
                height: 18
                radius: 9
                visible: !row.ongoing
                color: closeMouse.containsMouse ? card.wash(0.16) : "transparent"

                Icon {
                    anchors.centerIn: parent
                    width: 10
                    height: 10
                    path: Icons.close
                    color: card.wash(0.52)
                }
                MouseArea {
                    id: closeMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: NotificationList.dismiss(row.deviceId, row.key)
                }
            }
        }
    }

    component ActionChip: Rectangle {
        id: chip
        property string label
        property bool mono: false
        signal activated

        width: chipText.implicitWidth + 12
        height: 20
        radius: 10
        color: chipMouse.containsMouse ? card.wash(0.16) : card.wash(0.07)
        border.width: 1
        border.color: card.wash(0.10)

        Txt {
            id: chipText
            anchors.centerIn: parent
            text: chip.label
            role: chip.mono ? "code" : "caption"
            size: chip.mono ? 10 : 9.5
            weight: Font.DemiBold
            color: card.ink
        }
        MouseArea {
            id: chipMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: chip.activated()
        }
    }
}
