// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// One phone notification in the feed: app, title and text, its actions,
// an inline reply when the app offers one, and dismiss and options buttons
// (what this app's notifications do on this PC).
Item {
    id: item
    required property string deviceId
    required property string key
    required property string app
    required property string appName
    required property string title
    required property string text
    required property string sub
    required property real when
    required property string iconUrl
    required property string imageUrl
    required property string actions
    required property string replyAction
    required property string replyLabel
    required property string replies
    required property string otpCode
    required property string live
    // Bumped by the feed every minute, so relative times stay right.
    property int clock: 0
    property int chronoTick: 0

    readonly property var buttons: { try { return JSON.parse(actions) } catch (e) { return [] } }
    readonly property var sent: { try { return JSON.parse(replies) } catch (e) { return [] } }
    readonly property var liveData: {
        if (!live || live.length === 0)
            return null
        try {
            return JSON.parse(live)
        } catch (e) {
            return null
        }
    }
    readonly property bool isLive: liveData !== null
    readonly property bool hasProgress: isLive && (Boolean(liveData.indeterminate) || Number(liveData.max) > 0)
    readonly property var liveSegments: isLive && liveData.segments ? liveData.segments : []
    readonly property var livePoints: isLive && liveData.points ? liveData.points : []
    readonly property string liveBadgeText: {
        if (!isLive)
            return ""
        const c = (liveData.chip || "").trim()
        return c.length > 0 ? c : qsTr("LIVE")
    }
    readonly property string liveRightLabel: {
        if (!isLive)
            return ""
        if (liveData.chronometer)
            return chronoText()
        if (!liveData.indeterminate && Number(liveData.max) > 0) {
            const pct = Math.min(100, Math.max(0, Math.round(Number(liveData.progress) * 100 / Number(liveData.max))))
            return pct + "%"
        }
        return ""
    }
    property bool replying: false
    property bool canOpenApp: false
    signal optionsRequested
    signal openAppRequested

    implicitHeight: content.height + 24
    activeFocusOnTab: item.canOpenApp && !item.replying
    Accessible.role: item.canOpenApp ? Accessible.Button : Accessible.Grouping
    Accessible.name: [item.appName, item.title, item.text].filter(s => s.length > 0).join(", ")
    Accessible.description: item.canOpenApp ? qsTr("Open %1 in an app window").arg(item.appName) : ""
    Accessible.onPressAction: if (item.canOpenApp && !item.replying) item.openAppRequested()
    Keys.onPressed: (event) => {
        if (item.canOpenApp && !item.replying
                && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            item.openAppRequested()
            event.accepted = true
        }
    }

    // Ticks once per second only for visible chronometer notifications while the window is shown.
    Timer {
        id: chronoTimer
        interval: 1000
        repeat: true
        running: item.isLive && Boolean(item.liveData.chronometer)
                 && item.visible && Window.window
                 && Window.window.visibility !== Window.Hidden
                 && Window.window.visibility !== Window.Minimized
        onTriggered: item.chronoTick++
    }

    function chronoText() {
        if (!isLive || !liveData.chronometer)
            return ""
        chronoTick
        const target = Number(liveData.when || when)
        const diffMs = liveData.countdown
            ? Math.max(0, target - Date.now())
            : Math.max(0, Date.now() - target)
        const totalSecs = Math.floor(diffMs / 1000)
        const hrs = Math.floor(totalSecs / 3600)
        const mins = Math.floor((totalSecs % 3600) / 60)
        const secs = totalSecs % 60
        const pad = (n) => (n < 10 ? "0" + n : String(n))
        if (hrs > 0)
            return hrs + ":" + pad(mins) + ":" + pad(secs)
        return mins + ":" + pad(secs)
    }

    function segmentFillFraction(index) {
        if (!isLive || liveSegments.length === 0)
            return 0
        const prog = Number(liveData.progress || 0)
        let start = 0
        for (let i = 0; i < index; i++)
            start += Number(liveSegments[i].length || 0)
        const len = Math.max(1, Number(liveSegments[index].length || 1))
        return Math.min(1, Math.max(0, (prog - start) / len))
    }

    function timeText() {
        clock // re-evaluated each minute
        const minutes = Math.round((Date.now() - when) / 60000)
        if (minutes < 1) return qsTr("now")
        if (minutes < 60) return qsTr("%n min", "", minutes)
        const date = new Date(when)
        if (Date.now() - when < 24 * 3600 * 1000) return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    function send() {
        if (input.text.trim().length === 0)
            return
        NotificationList.reply(item.deviceId, item.key, item.replyAction, input.text)
        input.text = ""
        item.replying = false
    }

    // Hover tracking; clicking the card body opens the app window when available.
    MouseArea {
        id: hover
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: item.canOpenApp && !item.replying ? Qt.LeftButton : Qt.NoButton
        cursorShape: item.canOpenApp && !item.replying ? Qt.PointingHandCursor : Qt.ArrowCursor
        readonly property bool hovered: containsMouse
        onClicked: if (item.canOpenApp && !item.replying) item.openAppRequested()
    }

    // A light wash of the text color, or a calm highlighted container for Live Updates.
    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: item.isLive
            ? (Theme.graphite
               ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, hover.hovered ? 0.065 : 0.03)
               : Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b,
                         hover.hovered ? (Theme.dark ? 0.36 : 0.52) : (Theme.dark ? 0.24 : 0.36)))
            : Theme.surfaceContent
        opacity: item.isLive ? 1 : (hover.hovered ? 0.045 : 0)
        border.width: item.isLive ? 1 : 0
        border.color: Theme.graphite
            ? Theme.outlineVariant
            : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.34 : 0.24)
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusMd
        color: "transparent"
        border.width: Theme.focusVisible(item) ? 2 : 0
        border.color: Theme.primary
        visible: Theme.focusVisible(item)
    }

    // App icon, or its initial when the phone didn't send one.
    Item {
        id: icon
        x: 12; y: 12
        width: 32; height: 32
        Image {
            id: image
            anchors.fill: parent
            source: item.iconUrl
            sourceSize: Qt.size(64, 64)
            smooth: true
            mipmap: true
            visible: status === Image.Ready
        }
        Avatar {
            anchors.fill: parent
            visible: image.status !== Image.Ready
            name: item.appName
        }
    }

    Column {
        id: content
        anchors.left: icon.right
        anchors.leftMargin: 12
        anchors.right: parent.right
        anchors.rightMargin: 84
        y: 12
        spacing: 4

        Item {
            width: parent.width
            height: Math.max(metaRow.height, liveReadout.height)

            Row {
                id: metaRow
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6

                // Live chip pill (shortCriticalText or "LIVE").
                Rectangle {
                    id: livePill
                    visible: item.isLive
                    anchors.verticalCenter: parent.verticalCenter
                    height: 18
                    width: livePillRow.width + 12
                    radius: Theme.graphite ? Theme.radiusSm : 9
                    color: Theme.graphite ? "transparent" : Theme.primaryContainer
                    border.width: Theme.graphite ? 1 : 0
                    border.color: Theme.primary

                    Row {
                        id: livePillRow
                        anchors.centerIn: parent
                        spacing: 4
                        Rectangle {
                            width: 5
                            height: 5
                            radius: 2.5
                            anchors.verticalCenter: parent.verticalCenter
                            color: Theme.graphite ? Theme.primary : Theme.primaryContainerContent
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "mono"
                            size: 10
                            weight: Font.Bold
                            color: Theme.graphite ? Theme.primary : Theme.primaryContainerContent
                            text: item.liveBadgeText
                        }
                    }
                }

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(40, content.width - (livePill.visible ? livePill.width + 6 : 0)
                                        - (liveReadout.visible ? liveReadout.width + 8 : 0))
                    role: "caption"
                    muted: true
                    elide: Text.ElideRight
                    text: (item.isLive
                           ? [item.appName, item.sub]
                           : [item.appName, item.sub, item.timeText()]).filter(s => s.length > 0).join(" · ")
                }
            }

            Txt {
                id: liveReadout
                visible: item.liveRightLabel.length > 0
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                role: "mono"
                size: 11
                weight: Font.DemiBold
                color: Theme.primary
                text: item.liveRightLabel
            }
        }
        Txt {
            width: parent.width
            visible: item.title.length > 0
            role: "title"
            size: 14
            text: item.title
        }
        Txt {
            width: parent.width
            visible: item.text.length > 0
            role: "body"
            text: item.text
            wrapMode: Text.Wrap
            maximumLineCount: 4
        }

        // Live progress bar (smooth, indeterminate, or multi-segment with milestone points).
        Item {
            id: liveBarBox
            width: parent.width
            height: 12
            visible: item.hasProgress

            // Smooth single-track bar when no segments are defined.
            Rectangle {
                id: smoothTrack
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 6
                radius: 3
                visible: item.liveSegments.length === 0
                color: Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b,
                               Theme.dark ? 0.16 : 0.12)

                Rectangle {
                    height: parent.height
                    radius: 3
                    x: item.isLive && item.liveData.indeterminate ? Math.round(parent.width * 0.15) : 0
                    width: {
                        if (!item.isLive)
                            return 0
                        if (item.liveData.indeterminate)
                            return Math.max(24, Math.round(parent.width * 0.35))
                        const maxVal = Math.max(1, Number(item.liveData.max || 1))
                        const ratio = Math.min(1, Math.max(0, Number(item.liveData.progress || 0) / maxVal))
                        return ratio > 0 ? Math.max(6, Math.round(parent.width * ratio)) : 0
                    }
                    color: Theme.primary
                    Behavior on width {
                        enabled: !Theme.reduceMotion
                        NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                    }
                }
            }

            // Multi-segment progress bar (Android 16 ProgressStyle.Segment).
            Row {
                id: segRow
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 6
                spacing: 3
                visible: item.liveSegments.length > 0
                readonly property real totalLen: {
                    let s = 0
                    for (let i = 0; i < item.liveSegments.length; i++)
                        s += Math.max(1, Number(item.liveSegments[i].length || 1))
                    return Math.max(1, s)
                }
                readonly property real availW: Math.max(10, width - spacing * Math.max(0, item.liveSegments.length - 1))

                Repeater {
                    model: item.liveSegments
                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        height: 6
                        radius: 3
                        width: Math.max(6, Math.floor(segRow.availW * Math.max(1, Number(modelData.length || 1)) / segRow.totalLen))
                        color: Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b,
                                       Theme.dark ? 0.16 : 0.12)

                        Rectangle {
                            height: parent.height
                            radius: 3
                            width: Math.round(parent.width * item.segmentFillFraction(index))
                            color: modelData.color && modelData.color.length > 0 ? modelData.color : Theme.primary
                            Behavior on width {
                                enabled: !Theme.reduceMotion
                                NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                            }
                        }
                    }
                }
            }

            // Milestone points (Android 16 ProgressStyle.Point).
            Repeater {
                model: item.livePoints
                delegate: Rectangle {
                    required property var modelData
                    readonly property real maxVal: Math.max(1, Number(item.isLive ? item.liveData.max : 1))
                    readonly property real posRatio: Math.min(1, Math.max(0, Number(modelData.position || 0) / maxVal))
                    readonly property bool reached: item.isLive && Number(item.liveData.progress || 0) >= Number(modelData.position || 0)
                    width: 8
                    height: 8
                    radius: 4
                    anchors.verticalCenter: parent.verticalCenter
                    x: Math.round((liveBarBox.width - width) * posRatio)
                    color: reached
                        ? (modelData.color && modelData.color.length > 0 ? modelData.color : Theme.primary)
                        : Theme.surfaceContainerHighest
                    border.width: 1.5
                    border.color: modelData.color && modelData.color.length > 0 ? modelData.color : Theme.primary
                }
            }
        }

        // The picture it shows (a photo in a message), up to a modest size.
        RoundedImage {
            id: picture
            width: Math.min(parent.width, 320)
            height: status === Image.Ready
                    ? Math.min(220, width * implicitImageHeight / Math.max(1, implicitImageWidth)) : 0
            visible: item.imageUrl.length > 0 && status === Image.Ready
            source: item.imageUrl
            sourceSize.width: 640
        }

        // Replies sent from this PC.
        Repeater {
            model: item.sent
            delegate: Row {
                required property var modelData
                width: content.width
                spacing: 6
                opacity: modelData.pending ? 0.55 : 1
                Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
                Icon {
                    width: 14; height: 14
                    anchors.verticalCenter: parent.verticalCenter
                    path: modelData.pending ? Icons.send : Icons.check
                    color: Theme.primary
                }
                Txt {
                    width: parent.width - 20
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.Wrap
                    maximumLineCount: 3
                    text: modelData.pending ? qsTr("Sending: %1").arg(modelData.text) : qsTr("You: %1").arg(modelData.text)
                }
            }
        }

        Flow {
            width: parent.width
            spacing: 6
            topPadding: 4
            visible: !item.replying && (item.otpCode.length > 0 || item.replyAction.length > 0 || item.buttons.length > 0)
            Button {
                visible: item.otpCode.length > 0
                variant: "tonal"
                size: "sm"
                iconPath: Icons.copy
                text: qsTr("Copy code")
                onClicked: NotificationList.copyCode(item.otpCode)
            }
            Button {
                visible: item.replyAction.length > 0
                variant: "tonal"
                size: "sm"
                text: item.replyLabel.length > 0 ? item.replyLabel : qsTr("Reply")
                onClicked: { item.replying = true; input.forceActiveFocus() }
            }
            Repeater {
                model: item.buttons
                delegate: Button {
                    required property var modelData
                    variant: "text"
                    size: "sm"
                    text: modelData.title
                    onClicked: NotificationList.runAction(item.deviceId, item.key, modelData.id)
                }
            }
        }

        // Inline reply.
        Rectangle {
            width: parent.width
            height: 40
            visible: item.replying
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: input.activeFocus ? 2 : 1
            border.color: input.activeFocus ? Theme.primary : Theme.outlineVariant

            TextInput {
                id: input
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.right: sendButton.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.fontUi
                font.pixelSize: 14
                color: Theme.surfaceContent
                selectionColor: Theme.primaryContainer
                selectedTextColor: Theme.primaryContainerContent
                clip: true
                Keys.onReturnPressed: item.send()
                Keys.onEnterPressed: item.send()
                Keys.onEscapePressed: { text = ""; item.replying = false }
                Accessible.name: qsTr("Reply to %1").arg(item.title)

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: input.text.length === 0
                    role: "body"
                    muted: true
                    text: qsTr("Reply to %1").arg(item.title.length > 0 ? item.title : item.appName)
                }
            }
            IconButton {
                id: sendButton
                anchors.right: parent.right
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.send
                label: qsTr("Send")
                enabled: input.text.trim().length > 0
                onClicked: item.send()
            }
        }
    }

    IconButton {
        anchors.right: dismiss.left
        y: 8
        iconPath: Icons.more
        label: qsTr("Options for %1").arg(item.appName)
        // Like Dismiss: always there (for keyboards and screen readers),
        // quiet until hovered.
        opacity: hover.hovered || activeFocus ? 1 : 0.55
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        onClicked: item.optionsRequested()
    }
    IconButton {
        id: dismiss
        anchors.right: parent.right
        anchors.rightMargin: 8
        y: 8
        iconPath: Icons.close
        label: qsTr("Dismiss")
        // Always there (for keyboards and screen readers), quiet until hovered.
        opacity: hover.hovered || activeFocus ? 1 : 0.55
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        onClicked: NotificationList.dismiss(item.deviceId, item.key)
    }
}
