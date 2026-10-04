import QtQuick
import QtQuick.Effects
import app.nectarlink.spike

Window {
    id: window
    readonly property bool mica: Qt.application.arguments.indexOf("--mica") >= 0
    width: 1180
    height: 720
    minimumWidth: 980
    minimumHeight: 640
    visible: true
    title: "Nectarlink · S1 proof"
    color: mica ? "transparent" : Theme.surface

    DeviceModel {
        id: device
        onNotification: (app, title, body) => {
            notifications.insert(0, { app: app, title: title, body: body, time: "now" })
            if (notifications.count > 40)
                notifications.remove(notifications.count - 1)
        }
    }

    ListModel {
        id: notifications
        ListElement { app: "Google"; title: "Sign-in code"; body: "Your verification code is 482 913"; time: "now" }
        ListElement { app: "WhatsApp"; title: "Mom"; body: "Dinner at 8? Bring the charger you borrowed."; time: "2m" }
        ListElement { app: "Swiggy"; title: "On the way"; body: "Your order arrives in 12 minutes"; time: "18m" }
        ListElement { app: "Calendar"; title: "Design review"; body: "With Aanya at 4:30 PM"; time: "1h" }
        ListElement { app: "Discord"; title: "#general"; body: "6 new messages"; time: "2h" }
    }

    // --autotest: drives the proof by itself and prints the measurements.
    readonly property bool autotest: Qt.application.arguments.indexOf("--autotest") >= 0
    property real minFps: 1e9
    property real fpsSum: 0
    property int fpsSamples: 0
    Timer {
        running: window.autotest; interval: 1000; repeat: false
        onTriggered: { stats.measuring = true; device.startStress(10000); phase2.start() }
    }
    Timer {
        id: phase2; interval: 2000
        onTriggered: { detail.openFrom(list.itemAtIndex(0), "Mom", "WhatsApp", "Dinner at 8? Bring the charger you borrowed."); phase3.start() }
    }
    Timer {
        id: phase3; interval: 2500
        onTriggered: { detail.close(); phase4.start() }
    }
    Timer {
        id: phase4; interval: 2000
        onTriggered: {
            device.refreshMemory()
            console.log("AUTOTEST fps_avg=" + (window.fpsSum / Math.max(1, window.fpsSamples)).toFixed(1)
                        + " fps_min=" + window.minFps.toFixed(1)
                        + " events_per_s=" + device.eventsPerSecond
                        + " events_total=" + Math.round(device.eventsTotal)
                        + " notifications=" + notifications.count
                        + " mem_mb=" + device.workingSetMb.toFixed(1)
                        + " first_frame_ms=" + device.startupMs.toFixed(0))
            device.stopStress()
            Qt.quit()
        }
    }
    Timer {
        running: window.autotest && stats.measuring; interval: 250; repeat: true
        onTriggered: {
            // Skip the first second while the frame clock settles.
            if (stats.fps > 0 && device.eventsTotal > 10000) {
                window.minFps = Math.min(window.minFps, stats.fps)
                window.fpsSum += stats.fps; window.fpsSamples += 1
            }
        }
    }

    // Report the first rendered frame for the startup measurement.
    Connections {
        target: window
        function onFrameSwapped() { device.firstFrame(); target = null }
    }

    readonly property var icons: ({
        home: "M3 11l9-7 9 7M5 10v10h14V10",
        msg: "M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z",
        bell: "M6 8a6 6 0 1 1 12 0c0 7 3 8 3 8H3s3-1 3-8M10 20a2 2 0 0 0 4 0",
        photo: "M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3zM21 15l-5-5L5 21",
        apps: "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z",
        deck: "M6 5h12a3 3 0 0 1 3 3v8a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V8a3 3 0 0 1 3-3zM8 14h8",
        gear: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1L7 17M17 7l2.1-2.1",
        clip: "M9 3h6v4H9zM16 5h2a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2",
        mirror: "M4 4h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM18.5 8h2a1.5 1.5 0 0 1 1.5 1.5v7a1.5 1.5 0 0 1-1.5 1.5h-2A1.5 1.5 0 0 1 17 16.5v-7A1.5 1.5 0 0 1 18.5 8zM6 18h6",
        send: "M22 2L11 13M22 2l-7 20-4-9-9-4z",
        ring: "M6 8a6 6 0 1 1 12 0c0 7 3 8 3 8H3s3-1 3-8M10 20a2 2 0 0 0 4 0",
        mic: "M12 2a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V5a3 3 0 0 1 3-3zM5 11a7 7 0 0 0 14 0M12 18v4",
        play: "M8 5l11 7-11 7z",
        lock: "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2zM8 11V8a4 4 0 0 1 8 0v3",
        code: "M8 8l-5 4 5 4M16 8l5 4-5 4",
        moon: "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z",
        hex: "M12 2.5l8.2 4.75v9.5L12 21.5l-8.2-4.75v-9.5zM9 12h6",
        back: "M15 6l-6 6 6 6",
        battery: "M4 7h14a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2zM22 11v2"
    })

    Row {
        anchors.fill: parent

        // ---- Navigation rail ----
        Rectangle {
            id: rail
            width: 80
            height: parent.height
            color: window.mica ? "transparent" : Theme.surfaceLow
            Column {
                anchors.horizontalCenter: parent.horizontalCenter
                topPadding: 16
                spacing: 6
                Rectangle {
                    width: 38; height: 38; radius: Theme.radiusSm; color: Theme.primary
                    anchors.horizontalCenter: parent.horizontalCenter
                    Icon { anchors.centerIn: parent; path: window.icons.hex; color: Theme.primaryContent }
                }
                Item { width: 1; height: 8 }
                Repeater {
                    model: [["home", "Home"], ["msg", "Messages"], ["bell", "Alerts"], ["photo", "Photos"], ["apps", "Apps"], ["deck", "Deck"]]
                    delegate: Column {
                        id: navItem
                        required property var modelData
                        required property int index
                        readonly property bool selected: rail.selected === index
                        spacing: 4
                        anchors.horizontalCenter: parent.horizontalCenter
                        Rectangle {
                            width: 52; height: 30; radius: 15
                            anchors.horizontalCenter: parent.horizontalCenter
                            color: navItem.selected ? Theme.secondaryContainer : (navHover.hovered ? Theme.surfaceContainer : "transparent")
                            Behavior on color { ColorAnimation { duration: 160 } }
                            Icon { anchors.centerIn: parent; path: window.icons[navItem.modelData[0]]; color: navItem.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant }
                            HoverHandler { id: navHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: rail.selected = navItem.index }
                        }
                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: navItem.modelData[1]; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.DemiBold
                            color: navItem.selected ? Theme.surfaceContent : Theme.surfaceContentVariant
                        }
                    }
                }
            }
            property int selected: 0
            Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.outlineVariant }
        }

        // ---- Main ----
        Rectangle {
            width: parent.width - rail.width
            height: parent.height
            color: window.mica ? Qt.rgba(1, 0.97, 0.95, 0.72) : Theme.surface

            Item {
                id: topBar
                width: parent.width; height: 52
                Row {
                    anchors { left: parent.left; leftMargin: 22; verticalCenter: parent.verticalCenter }
                    spacing: 12
                    Text { text: "Home"; font.family: Theme.displayFont; font.pixelSize: 20; font.weight: Font.DemiBold; color: Theme.surfaceContent }
                    Row {
                        spacing: 8; anchors.verticalCenter: parent.verticalCenter
                        Rectangle { width: 7; height: 7; radius: 4; color: Theme.primary; anchors.verticalCenter: parent.verticalCenter }
                        Text { text: device.deviceName + " · Wi-Fi · " + device.rttMs + " ms"; font.family: Theme.font; font.pixelSize: 13; color: Theme.surfaceContentVariant }
                    }
                }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.outlineVariant }
            }

            Row {
                anchors { top: topBar.bottom; left: parent.left; right: parent.right; bottom: parent.bottom; margins: 20 }
                spacing: 16

                // Left column
                Column {
                    id: leftCol
                    width: (parent.width - 16) * 0.58
                    spacing: 16

                    // Hero: the connected phone, with a real-time blurred glow behind it.
                    Item {
                        width: parent.width; height: 168
                        Rectangle {
                            id: heroBg
                            anchors.fill: parent
                            radius: Theme.radiusXl
                            color: Theme.hero
                        }
                        Rectangle {
                            id: glow
                            x: 30; y: 26; width: 120; height: 120; radius: 60
                            color: Theme.primary; opacity: 0.35
                            visible: false
                        }
                        MultiEffect {
                            source: glow; anchors.fill: glow
                            blurEnabled: true; blur: 1.0; blurMax: 64
                            opacity: 0.55
                        }
                        Row {
                            anchors { fill: parent; margins: 18 }
                            spacing: 20
                            Rectangle {
                                width: 76; height: 132; radius: 18
                                border.width: 3; border.color: Theme.surfaceContent
                                gradient: Gradient {
                                    GradientStop { position: 0; color: Theme.primary }
                                    GradientStop { position: 1; color: Qt.darker(Theme.primary, 2.2) }
                                }
                                Rectangle { width: 20; height: 5; radius: 3; color: Theme.surfaceContent; anchors.horizontalCenter: parent.horizontalCenter; y: 7 }
                            }
                            Column {
                                spacing: 6
                                anchors.verticalCenter: parent.verticalCenter
                                Text { text: device.deviceName; font.family: Theme.displayFont; font.pixelSize: 30; font.weight: Font.DemiBold; color: Theme.primaryContainerContent }
                                Text { text: "Android 16 · paired since Oct 5"; font.family: Theme.font; font.pixelSize: 13; color: Theme.primaryContainerContent; opacity: 0.78 }
                                Row {
                                    spacing: 8; topPadding: 8
                                    Repeater {
                                        model: [device.battery + "%" + (device.charging ? " · charging" : ""), "Home Wi-Fi", "3 unread"]
                                        delegate: Rectangle {
                                            required property string modelData
                                            height: 30; width: pill.implicitWidth + 24; radius: 15
                                            color: Qt.rgba(0.17, 0.09, 0, 0.09)
                                            Text { id: pill; anchors.centerIn: parent; text: modelData; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; color: Theme.primaryContainerContent }
                                        }
                                    }
                                }
                            }
                        }
                        Rectangle {
                            anchors { top: parent.top; right: parent.right; margins: 16 }
                            width: levelText.implicitWidth + 24; height: 28; radius: 14; color: Theme.surface
                            Text { id: levelText; anchors.centerIn: parent; text: "Elevated"; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.DemiBold; color: Theme.surfaceContent }
                        }
                    }

                    Row {
                        width: parent.width; spacing: 10
                        Repeater {
                            model: [["Send clipboard", "Last copied: a link", "clip"], ["Mirror screen", "1080p · 60 fps", "mirror"], ["Send files", "Or drop anywhere", "send"], ["Find phone", "Rings on silent", "ring"]]
                            delegate: QuickAction {
                                required property var modelData
                                width: (leftCol.width - 30) / 4
                                title: modelData[0]; subtitle: modelData[1]; iconPath: window.icons[modelData[2]]
                            }
                        }
                    }

                    Rectangle {
                        width: parent.width; height: deckGrid.implicitHeight + 46
                        radius: Theme.radiusLg; color: Theme.surfaceContainer
                        Text { x: 16; y: 14; text: "Deck"; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; color: Theme.surfaceContentVariant }
                        Grid {
                            id: deckGrid
                            x: 16; y: 38; columns: 6; spacing: 10
                            Repeater {
                                model: [["Mic", "Muted", "mic", true], ["Play", "Spotify", "play", false], ["Focus", "Off", "moon", false], ["VS Code", "Launch", "code", false], ["Lock PC", "Win + L", "lock", false], ["Clipboard", "History", "clip", false]]
                                delegate: DeckKey {
                                    required property var modelData
                                    width: (leftCol.width - 32 - 50) / 6; height: width
                                    label: modelData[0]; detail: modelData[1]; iconPath: window.icons[modelData[2]]; active: modelData[3]
                                }
                            }
                        }
                    }
                }

                // Right column: notifications
                Rectangle {
                    id: feed
                    width: parent.width - leftCol.width - 16
                    height: parent.height
                    radius: Theme.radiusLg
                    color: Theme.surfaceContainer
                    clip: true
                    Text { x: 16; y: 14; text: "Notifications"; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; color: Theme.surfaceContentVariant }
                    ListView {
                        id: list
                        anchors { fill: parent; topMargin: 40; leftMargin: 6; rightMargin: 6; bottomMargin: 6 }
                        model: notifications
                        spacing: 2
                        clip: true
                        add: Transition {
                            ParallelAnimation {
                                NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 220 }
                                SpringAnimation { property: "y"; spring: Theme.springStandard; damping: Theme.dampingStandard }
                            }
                        }
                        addDisplaced: Transition { SpringAnimation { property: "y"; spring: Theme.springStandard; damping: Theme.dampingStandard } }
                        delegate: NotificationCard {
                            width: ListView.view.width
                            onOpened: (source) => detail.openFrom(source, title, app, body)
                        }
                    }
                }
            }

            StatsPanel {
                id: stats
                model: device
                anchors { right: parent.right; bottom: parent.bottom; margins: 24 }
                z: 5
            }

            // Shared-element transition: the conversation grows out of the tapped card.
            Rectangle {
                id: detail
                property rect from
                property string who
                property string via
                property string message
                property bool open: false
                visible: open || springX.running
                z: 10
                radius: open ? Theme.radiusXl : Theme.radiusMd
                color: Theme.surface
                border.color: Theme.outlineVariant
                layer.enabled: true
                layer.effect: MultiEffect { shadowEnabled: true; shadowBlur: 0.7; shadowOpacity: 0.18; shadowVerticalOffset: 10 }

                function openFrom(source, title, app, body) {
                    const p = source.mapToItem(detail.parent, 0, 0)
                    from = Qt.rect(p.x, p.y, source.width, source.height)
                    x = from.x; y = from.y; width = from.width; height = from.height
                    who = title; via = app; message = body
                    open = true
                }
                function close() { open = false }

                states: State {
                    when: detail.open
                    PropertyChanges { detail.x: feed.x + 20 - 40; detail.y: topBar.height + 20; detail.width: feed.width + 40; detail.height: feed.height }
                }
                transitions: Transition {
                    SpringAnimation { id: springX; properties: "x,y,width,height"; spring: Theme.springGentle; damping: 0.32; epsilon: 0.25 }
                }
                Behavior on radius { NumberAnimation { duration: 220 } }

                Column {
                    anchors { fill: parent; margins: 20 }
                    spacing: 14
                    opacity: detail.open ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: 180 } }
                    Row {
                        spacing: 12
                        Rectangle {
                            width: 36; height: 36; radius: 18; color: Theme.secondaryContainer
                            Icon { anchors.centerIn: parent; path: window.icons.back; color: Theme.secondaryContainerContent }
                            TapHandler { onTapped: detail.close() }
                        }
                        Column {
                            Text { text: detail.who; font.family: Theme.displayFont; font.pixelSize: 18; font.weight: Font.DemiBold; color: Theme.surfaceContent }
                            Text { text: detail.via + " · via " + device.deviceName; font.family: Theme.font; font.pixelSize: 12; color: Theme.surfaceContentVariant }
                        }
                    }
                    Rectangle {
                        width: Math.min(bubble.implicitWidth + 28, parent.width * 0.8); height: bubble.implicitHeight + 20
                        radius: 18; color: Theme.surfaceContainerHigh
                        Text { id: bubble; anchors.centerIn: parent; width: parent.width - 28; wrapMode: Text.Wrap; text: detail.message; font.family: Theme.font; font.pixelSize: 14; color: Theme.surfaceContent }
                    }
                }
                Rectangle {
                    anchors { left: parent.left; right: parent.right; bottom: parent.bottom; margins: 20 }
                    height: 46; radius: 23; color: Theme.surfaceContainerHigh
                    opacity: detail.open ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: 180 } }
                    Text { x: 18; anchors.verticalCenter: parent.verticalCenter; text: "Reply to " + detail.who + "…"; font.family: Theme.font; font.pixelSize: 13; color: Theme.surfaceContentVariant }
                    Rectangle {
                        width: 38; height: 38; radius: 19; color: Theme.primary
                        anchors { right: parent.right; rightMargin: 4; verticalCenter: parent.verticalCenter }
                        Icon { anchors.centerIn: parent; path: window.icons.send; color: Theme.primaryContent; width: 18; height: 18 }
                    }
                }
            }
        }
    }
}
