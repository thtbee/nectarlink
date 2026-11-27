// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import app.nectarlink

// The panel under Nectar Island: where the pill goes, the way back to the
// main window, and every phone notification waiting on this PC. It is its
// own window so the transparent space around the pill never catches clicks.
Window {
    id: panel
    required property var island

    readonly property bool light: island ? island.light : false
    readonly property color ink: island ? island.ink : "#FFFFFF"
    function wash(alpha) { return island ? island.wash(alpha) : Qt.rgba(1, 1, 1, alpha) }

    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
    color: "transparent"
    title: qsTr("Nectar Island panel")
    visible: !!island && island.panelShown && island.visible
    width: 380
    height: card.height

    // The pill's work area, in the island window's logical coordinates.
    // Start below the whole Island window, so the panel's rounded top is
    // never tucked under the pill's transparent margin.
    readonly property real belowY: island
        ? Math.max(island.y + 2 + island.pillHeight + 8, island.y + island.height) : 0
    readonly property real aboveY: island ? island.y - height - 8 : 0
    x: island ? Math.max(island.areaX + 6,
            Math.min(island.x + Math.round((island.width - width) / 2),
                     island.areaX + island.areaW - width - 6)) : 0
    y: {
        if (!island)
            return 0
        const bottom = island.areaY + island.areaH
        if (belowY + height <= bottom)
            return belowY
        if (aboveY >= island.areaY)
            return aboveY
        return Math.max(island.areaY, Math.min(belowY, bottom - height))
    }

    property real reveal: 1
    // Ages the notifications' relative times while the panel is open.
    property int clockTick: 0
    Timer {
        interval: 60000
        repeat: true
        running: panel.visible
        onTriggered: panel.clockTick++
    }
    NumberAnimation {
        id: revealAnim
        target: panel
        property: "reveal"
        from: 0
        to: 1
        duration: Theme.reduceMotion ? 0 : 170
        easing.type: Easing.OutCubic
    }
    // QSG_RENDER_LOOP=basic advances animations only while a window renders;
    // this keeps the panel's reveal, height and hover transitions moving.
    Timer {
        interval: 16
        repeat: true
        running: panel.visible && !Theme.reduceMotion
        onTriggered: panel.update()
    }
    onVisibleChanged: {
        if (visible) {
            reveal = 0
            revealAnim.restart()
            raise()
        } else if (island) {
            island.panelHovered = false
        }
    }

    Rectangle {
        id: card
        width: panel.width
        height: content.implicitHeight + 20
        radius: 20
        color: panel.light
            ? Theme.surfaceContainerHigh
            : Qt.rgba(0.05, 0.055, 0.075, Theme.glass ? 0.94 : 0.97)
        border.width: 1
        border.color: panel.wash(0.16)
        clip: true
        Behavior on height {
            enabled: !Theme.reduceMotion && panel.visible
            SpringAnimation { spring: 4.0; damping: 0.5; epsilon: 0.2 }
        }

        HoverHandler {
            onHoveredChanged: if (panel.island) panel.island.panelHovered = hovered
        }

        Column {
            id: content
            x: 12
            y: 10
            width: parent.width - 24
            spacing: 10
            opacity: panel.reveal
            transform: Translate { y: (1 - panel.reveal) * -4 }

            // A small grab handle ties the panel to the pill; the two Island
            // actions stay icon-sized so they don't take over the card.
            Item {
                width: parent.width
                height: 26

                Rectangle {
                    width: 28
                    height: 3
                    radius: 1.5
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.verticalCenter: parent.verticalCenter
                    color: panel.wash(0.22)
                }

                Row {
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2

                    PanelIconButton {
                        visible: panel.island && !panel.island.onWayland
                        iconPath: Icons.target
                        label: qsTr("Reset position")
                        onClicked: panel.island.resetIslandPosition()
                    }

                    PanelIconButton {
                        iconPath: Icons.home
                        label: qsTr("Open NectarLink")
                        onClicked: panel.island.openMain()
                    }
                }
            }

            RowLayout {
                width: parent.width
                spacing: 6

                Txt {
                    text: qsTr("Phone notifications")
                    role: "label"
                    size: 12
                    weight: Font.DemiBold
                    color: panel.wash(0.62)
                }

                Rectangle {
                    visible: NotificationList.count > 0
                    Layout.preferredWidth: countLabel.implicitWidth + 12
                    Layout.preferredHeight: 18
                    radius: 9
                    color: panel.wash(0.10)
                    Txt {
                        id: countLabel
                        anchors.centerIn: parent
                        text: String(NotificationList.count)
                        role: "mono"
                        size: 10
                        weight: Font.DemiBold
                        color: panel.ink
                    }
                }

                Item { Layout.fillWidth: true }

                Txt {
                    visible: Preferences.nectarIslandNotifications && NotificationList.count > 0
                    text: qsTr("Clear")
                    role: "caption"
                    size: 11
                    color: clearMouse.containsMouse ? panel.ink : panel.wash(0.55)
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

                PanelIconButton {
                    Layout.alignment: Qt.AlignVCenter
                    iconPath: Icons.close
                    label: qsTr("Close")
                    onClicked: panel.island.closePanel()
                }
            }

            // Alerts off: one click turns the Island's notification trigger on.
            RowLayout {
                visible: !Preferences.nectarIslandNotifications
                width: parent.width
                spacing: 8

                Icon {
                    Layout.alignment: Qt.AlignVCenter
                    Layout.preferredWidth: 16
                    Layout.preferredHeight: 16
                    path: Icons.bell
                    color: panel.wash(0.55)
                }
                Txt {
                    Layout.fillWidth: true
                    text: qsTr("Phone alerts are off for the Island.")
                    role: "caption"
                    size: 11
                    color: panel.wash(0.55)
                    wrapMode: Text.WordWrap
                }
                PanelButton {
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
                    color: panel.wash(0.55)
                }
                Txt {
                    Layout.fillWidth: true
                    text: qsTr("Notifications are paused.")
                    role: "caption"
                    size: 11
                    color: panel.wash(0.55)
                }
                PanelButton {
                    iconPath: Icons.bell
                    label: qsTr("Resume")
                    onClicked: AppController.toggleNotificationsPaused()
                }
            }

            Item {
                id: listBox
                visible: Preferences.nectarIslandNotifications && NotificationList.count > 0
                width: parent.width
                height: Math.min(notifColumn.implicitHeight, 320,
                    Math.max(120, panel.island ? panel.island.areaH - 230 : 320))

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
                            delegate: NotificationRow {}
                        }
                    }
                }

                ScrollRail {
                    flick: notifFlick
                    inset: -2
                    thumbColor: panel.wash(0.35)
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
                    color: panel.wash(0.42)
                }
                Txt {
                    Layout.fillWidth: true
                    text: NotificationList.allCount > 0
                        ? qsTr("Cleared from the Island. The main list still has them.")
                        : qsTr("No phone notifications right now.")
                    role: "caption"
                    size: 11
                    color: panel.wash(0.55)
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    component PanelButton: Rectangle {
        id: pbtn
        property string iconPath
        property string label
        property bool primary: false
        signal clicked

        implicitWidth: pbtnRow.implicitWidth + 20
        implicitHeight: 30
        radius: 15
        color: pbtn.primary
            ? (pbtnMouse.containsMouse ? Qt.lighter(Theme.primary, 1.06) : Theme.primary)
            : (pbtnMouse.containsMouse ? panel.wash(0.16) : panel.wash(0.08))
        border.width: pbtn.primary ? 0 : 1
        border.color: panel.wash(0.10)
        scale: pbtnMouse.pressed ? 0.97 : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

        Row {
            id: pbtnRow
            anchors.centerIn: parent
            spacing: 6
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                width: 13
                height: 13
                path: pbtn.iconPath
                color: pbtn.primary ? Theme.primaryContent : panel.ink
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: pbtn.label
                role: "label"
                size: 11
                weight: Font.DemiBold
                color: pbtn.primary ? Theme.primaryContent : panel.ink
            }
        }

        MouseArea {
            id: pbtnMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: pbtn.clicked()
        }
        Accessible.role: Accessible.Button
        Accessible.name: pbtn.label
    }

    component PanelIconButton: Rectangle {
        id: ibtn
        property string iconPath
        property string label
        signal clicked

        implicitWidth: 26
        implicitHeight: 26
        radius: 13
        color: ibtnMouse.containsMouse ? panel.wash(0.16) : "transparent"
        scale: ibtnMouse.pressed ? 0.92 : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

        Icon {
            anchors.centerIn: parent
            width: 13
            height: 13
            path: ibtn.iconPath
            color: panel.wash(0.62)
        }

        MouseArea {
            id: ibtnMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: ibtn.clicked()
        }
        Accessible.role: Accessible.Button
        Accessible.name: ibtn.label
    }

    component ActionChip: Rectangle {
        id: chip
        property string label
        property bool mono: false
        signal activated

        implicitWidth: chipRow.implicitWidth + 14
        implicitHeight: 22
        radius: 11
        color: chipMouse.containsMouse ? panel.wash(0.16) : panel.wash(0.07)
        border.width: 1
        border.color: panel.wash(0.10)
        scale: chipMouse.pressed ? 0.96 : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }

        Row {
            id: chipRow
            anchors.centerIn: parent
            spacing: 4
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: chip.label
                role: chip.mono ? "code" : "caption"
                size: chip.mono ? 11 : 10
                weight: Font.DemiBold
                color: panel.ink
            }
        }

        MouseArea {
            id: chipMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: chip.activated()
        }
        Accessible.role: Accessible.Button
        Accessible.name: chip.label
    }

    component NotificationRow: Item {
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
            panel.clockTick // re-evaluated each minute
            const minutes = Math.round((Date.now() - when) / 60000)
            if (minutes < 1) return qsTr("now")
            if (minutes < 60) return qsTr("%n min", "", minutes)
            const date = new Date(when)
            if (Date.now() - when < 24 * 3600 * 1000)
                return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
            return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
        }

        width: parent ? parent.width : 0
        implicitHeight: rowContent.implicitHeight + 14
        height: implicitHeight
        Accessible.role: Accessible.ListItem
        Accessible.name: [row.appLabel, row.title, row.text].filter(s => s.length > 0).join(", ")

        Rectangle {
            anchors.fill: parent
            radius: 12
            color: rowHover.hovered ? panel.wash(0.055) : "transparent"
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }
        }
        HoverHandler { id: rowHover }

        Rectangle {
            visible: row.isChild
            x: 12
            y: 0
            width: 2
            height: parent.height
            radius: 1
            color: panel.wash(0.10)
        }

        Row {
            id: rowContent
            x: row.isChild ? 22 : 10
            y: 7
            width: parent.width - x - 10
            spacing: 8

            Item {
                id: appIconBox
                width: 26
                height: 26

                Rectangle {
                    anchors.fill: parent
                    radius: 8
                    color: panel.wash(0.10)
                }
                RoundedImage {
                    id: appIcon
                    anchors.fill: parent
                    radius: 8
                    source: row.iconUrl
                    sourceSize: Qt.size(52, 52)
                }
                Txt {
                    anchors.centerIn: parent
                    visible: appIcon.status !== Image.Ready
                    text: row.appLabel.substring(0, 1).toUpperCase()
                    role: "label"
                    size: 12
                    weight: Font.DemiBold
                    color: panel.ink
                }
            }

            Column {
                id: textColumn
                width: parent.width - appIconBox.width - closeButton.width - parent.spacing * 2
                spacing: 3

                Row {
                    width: parent.width
                    spacing: 6

                    Txt {
                        width: Math.max(30, parent.width - timeLabel.implicitWidth
                            - (groupBadge.visible ? groupBadge.width + 6 : 0) - 12)
                        text: row.sub.length > 0 ? row.appLabel + " · " + row.sub : row.appLabel
                        role: "caption"
                        size: 10
                        weight: Font.DemiBold
                        color: panel.wash(0.68)
                        elide: Text.ElideRight
                    }
                    Txt {
                        id: timeLabel
                        text: row.timeText()
                        role: "caption"
                        size: 10
                        color: panel.wash(0.42)
                    }
                    Rectangle {
                        id: groupBadge
                        visible: row.islandGroupCount > 1 && row.groupIndex === 0
                        height: 16
                        width: groupBadgeRow.implicitWidth + 10
                        radius: 8
                        color: panel.wash(0.10)
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
                                color: panel.wash(0.68)
                            }
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 9
                                height: 9
                                path: row.groupExpanded ? Icons.chevronUp : Icons.chevronDown
                                color: panel.wash(0.55)
                            }
                        }
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: NotificationList.toggleGroup(row.app)
                        }
                        Accessible.role: Accessible.Button
                        Accessible.name: row.groupExpanded
                            ? qsTr("Collapse %1 notifications").arg(row.islandGroupCount)
                            : qsTr("Expand %1 notifications").arg(row.islandGroupCount)
                    }
                }

                Txt {
                    width: parent.width
                    visible: row.title.length > 0
                    text: row.title
                    role: "body"
                    size: 12
                    weight: Font.DemiBold
                    color: panel.ink
                    elide: Text.ElideRight
                }

                Txt {
                    width: parent.width
                    visible: row.text.length > 0
                    text: row.text
                    role: "bodySmall"
                    size: 11
                    color: panel.wash(0.62)
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }

                Rectangle {
                    visible: row.hasLiveProgress
                    width: parent.width
                    height: 3
                    radius: 1.5
                    color: panel.wash(0.12)
                    Rectangle {
                        width: parent.width * row.liveRatio
                        height: parent.height
                        radius: parent.radius
                        color: Theme.primary
                        Behavior on width {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                        }
                    }
                }

                Flow {
                    width: parent.width
                    spacing: 6
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
                        onActivated: panel.island.openMain()
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
                width: visible ? 20 : 0
                height: 20
                radius: 10
                visible: !row.ongoing
                color: closeMouse.containsMouse ? panel.wash(0.16) : "transparent"
                Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 120 } }

                Icon {
                    anchors.centerIn: parent
                    width: 11
                    height: 11
                    path: Icons.close
                    color: panel.wash(0.55)
                }
                MouseArea {
                    id: closeMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: NotificationList.dismiss(row.deviceId, row.key)
                }
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Dismiss notification")
            }
        }
    }
}
