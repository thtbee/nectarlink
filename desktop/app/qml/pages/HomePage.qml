// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import QtQuick.Shapes
import QtQml.Models
import QtCore
import app.nectarlink

// Home: the paired phone at a glance (status, battery, power level) and
// quick actions. Actions reflect the capability matrix: what isn't
// available shows what unlocks it.
Item {
    id: page
    // Home is drawn as the phone itself (status bar, clock, media,
    // notifications and dock inside a phone frame, with a Live switch for the
    // real screen). False brings back the original hero, tiles and feed.
    readonly property bool phoneLayout: true
    // Whether Qt Multimedia (which the radio needs) is installed. Asked
    // quietly: a failed import throws here instead of logging a warning.
    readonly property bool radioAvailable: {
        try {
            Qt.createQmlObject("import QtQuick; import QtMultimedia; QtObject {}", page, "radioProbe").destroy()
            return true
        } catch (e) {
            return false
        }
    }
    property bool active: true
    property int current: AppController.currentDevice
    Connections {
        target: AppController
        function onCurrentDeviceChanged() {
            if (page.current !== AppController.currentDevice)
                page.current = AppController.currentDevice
        }
    }
    // The device shown, for dropped files.
    property string currentDeviceId
    property string currentDeviceName
    property bool currentCanReceive: false
    signal pairRequested

    readonly property var localsendPeers: {
        if (!AppController.localsendEnabled || !AppController.localsendPeersJson || AppController.localsendPeersJson.length === 0)
            return []
        try {
            return JSON.parse(AppController.localsendPeersJson)
        } catch (e) {
            return []
        }
    }

    // The send queue as a model that's updated in place, so its rows keep
    // their delegates (and their animations) as progress comes in. Items
    // whose transfer already has a row under Transfers are left out: one
    // row per file.
    ListModel { id: sendQueueModel }
    property int sendQueueWaiting: 0

    function syncSendQueue() {
        let items = []
        try {
            items = TransferList.sendQueue && TransferList.sendQueue.length > 0
                ? JSON.parse(TransferList.sendQueue) : []
        } catch (e) {
            items = []
        }
        const rows = items
            .filter(item => !(item.status === "sending" && item.live))
            .map(item => ({
                queueId: String(item.id || ""),
                kind: String(item.kind || ""),
                icon: String(item.icon || ""),
                title: String(item.title || ""),
                status: String(item.status || "waiting"),
                reason: String(item.reason || ""),
                deviceName: String(item.deviceName || ""),
                totalBytes: Number(item.totalBytes || 0),
                progress: Number(item.progress || 0)
            }))
        const ids = rows.map(row => row.queueId)
        for (let r = sendQueueModel.count - 1; r >= 0; r--) {
            if (ids.indexOf(sendQueueModel.get(r).queueId) < 0)
                sendQueueModel.remove(r)
        }
        for (let i = 0; i < rows.length; i++) {
            let at = -1
            for (let r = i; r < sendQueueModel.count; r++) {
                if (sendQueueModel.get(r).queueId === rows[i].queueId) { at = r; break }
            }
            if (at < 0) {
                sendQueueModel.insert(i, rows[i])
            } else {
                if (at !== i) sendQueueModel.move(at, i, 1)
                sendQueueModel.set(i, rows[i])
            }
        }
        sendQueueWaiting = rows.filter(row => row.status === "waiting" || row.status === "sending").length
    }

    Connections {
        target: TransferList
        function onSendQueueChanged() { page.syncSendQueue() }
    }
    Component.onCompleted: syncSendQueue()

    function formatQueueSize(bytes) {
        if (!bytes || bytes <= 0) return ""
        if (bytes < 1024) return qsTr("%1 B").arg(Math.round(bytes))
        const units = [qsTr("%1 KB"), qsTr("%1 MB"), qsTr("%1 GB"), qsTr("%1 TB")]
        let value = bytes / 1024
        let unit = 0
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++ }
        return units[unit].arg(value < 10 ? Qt.locale().toString(value, "f", 1) : Math.round(value))
    }

    FileDialog {
        id: localsendFilePicker
        property string targetDeviceId: ""
        title: qsTr("Send files via LocalSend")
        fileMode: FileDialog.OpenFiles
        onAccepted: {
            if (targetDeviceId.length > 0 && selectedFiles.length > 0)
                TransferList.send(targetDeviceId, selectedFiles.map(url => url.toString()))
        }
    }

    visible: active
    readonly property bool resizing: Window.window && Window.window.resizing ? true : false

    onCurrentChanged: {
        if (current >= DeviceList.count) current = 0
        if (AppController.currentDevice !== current)
            AppController.currentDevice = current
    }
    onCurrentDeviceIdChanged: AppController.watchHomeSummary(visible ? currentDeviceId : "")
    onVisibleChanged: AppController.watchHomeSummary(visible ? currentDeviceId : "")
    Component.onDestruction: AppController.watchHomeSummary("")
    Connections {
        target: DeviceList
        function onCountChanged() {
            if (page.current >= DeviceList.count)
                page.current = Math.max(0, DeviceList.count - 1)
        }
    }

    // GETs JSON (the lock screen's weather).
    function getJson(url, ok, fail) {
        const xhr = new XMLHttpRequest()
        xhr.onreadystatechange = function() {
            if (xhr.readyState !== XMLHttpRequest.DONE)
                return
            let data = null
            if (xhr.status >= 200 && xhr.status < 300) {
                try { data = JSON.parse(xhr.responseText) } catch (e) {}
            }
            if (data !== null)
                ok(data)
            else if (fail)
                fail()
        }
        xhr.open("GET", url)
        xhr.send()
    }

    function relativeTime(unixSeconds) {
        if (!unixSeconds)
            return qsTr("never")
        const minutes = Math.round((Date.now() / 1000 - unixSeconds) / 60)
        if (minutes < 1) return qsTr("just now")
        if (minutes < 60) return qsTr("%n min ago", "", minutes)
        const hours = Math.round(minutes / 60)
        if (hours < 24) return qsTr("%n h ago", "", hours)
        return new Date(unixSeconds * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    function formatChargeEta(minutes) {
        if (!minutes || minutes <= 0)
            return ""
        if (minutes < 60)
            return qsTr("~%1 min to full").arg(minutes)
        const h = Math.floor(minutes / 60)
        const m = minutes % 60
        return m > 0 ? qsTr("~%1h %2m to full").arg(h).arg(m) : qsTr("~%1h to full").arg(h)
    }

    function roundedRectSvg(x, y, w, h, tl, tr, br, bl) {
        if (w <= 0 || h <= 0) return ""
        const maxR = Math.min(w, h) * 0.48
        const rTl = Math.max(0, Math.min(tl, maxR))
        const rTr = Math.max(0, Math.min(tr, maxR))
        const rBr = Math.max(0, Math.min(br, maxR))
        const rBl = Math.max(0, Math.min(bl, maxR))
        const f = (n) => Number(n).toFixed(2)
        return "M " + f(x + rTl) + " " + f(y)
            + " L " + f(x + w - rTr) + " " + f(y)
            + (rTr > 0 ? " A " + f(rTr) + " " + f(rTr) + " 0 0 1 " + f(x + w) + " " + f(y + rTr) : "")
            + " L " + f(x + w) + " " + f(y + h - rBr)
            + (rBr > 0 ? " A " + f(rBr) + " " + f(rBr) + " 0 0 1 " + f(x + w - rBr) + " " + f(y + h) : "")
            + " L " + f(x + rBl) + " " + f(y + h)
            + (rBl > 0 ? " A " + f(rBl) + " " + f(rBl) + " 0 0 1 " + f(x) + " " + f(y + h - rBl) : "")
            + " L " + f(x) + " " + f(y + rTl)
            + (rTl > 0 ? " A " + f(rTl) + " " + f(rTl) + " 0 0 1 " + f(x + rTl) + " " + f(y) : "")
            + " Z"
    }

    function scaleSvgPath(d, w, h, ox, oy) {
        if (!d || d.length === 0 || w <= 0 || h <= 0) return ""
        const tokens = d.match(/[a-zA-Z]|[-+]?(?:\d+\.?\d*|\.\d+)/g)
        if (!tokens) return ""
        let out = []
        let i = 0
        let cmd = ""
        const fx = (v) => (ox + parseFloat(v) * w).toFixed(2)
        const fy = (v) => (oy + parseFloat(v) * h).toFixed(2)
        const sw = (v) => (parseFloat(v) * w).toFixed(2)
        const sh = (v) => (parseFloat(v) * h).toFixed(2)
        while (i < tokens.length) {
            const t = tokens[i]
            if (/^[a-zA-Z]$/.test(t)) {
                cmd = t.toUpperCase()
                out.push(cmd)
                i++
                if (cmd === "Z") continue
            }
            if (cmd === "M" || cmd === "L" || cmd === "T") {
                if (i + 1 >= tokens.length) break
                out.push(fx(tokens[i]), fy(tokens[i + 1]))
                i += 2
            } else if (cmd === "H") {
                out.push(fx(tokens[i]))
                i += 1
            } else if (cmd === "V") {
                out.push(fy(tokens[i]))
                i += 1
            } else if (cmd === "Q" || cmd === "S") {
                if (i + 3 >= tokens.length) break
                out.push(fx(tokens[i]), fy(tokens[i + 1]), fx(tokens[i + 2]), fy(tokens[i + 3]))
                i += 4
            } else if (cmd === "C") {
                if (i + 5 >= tokens.length) break
                out.push(
                    fx(tokens[i]), fy(tokens[i + 1]),
                    fx(tokens[i + 2]), fy(tokens[i + 3]),
                    fx(tokens[i + 4]), fy(tokens[i + 5])
                )
                i += 6
            } else if (cmd === "A") {
                if (i + 6 >= tokens.length) break
                out.push(
                    sw(tokens[i]), sh(tokens[i + 1]),
                    tokens[i + 2], tokens[i + 3], tokens[i + 4],
                    fx(tokens[i + 5]), fy(tokens[i + 6])
                )
                i += 7
            } else {
                i++
            }
        }
        return out.join(" ")
    }

    function cutoutsToSvgPath(cutouts, w, h, ox, oy) {
        if (!cutouts || !cutouts.length || w <= 0 || h <= 0) return ""
        let parts = []
        for (let i = 0; i < cutouts.length; i++) {
            const c = cutouts[i]
            const rx = ox + c.x * w
            const ry = oy + c.y * h
            const rw = c.w * w
            const rh = c.h * h
            if (rw <= 0.5 || rh <= 0.5) continue
            const topFlush = c.y <= 0.006
            const r = topFlush ? Math.min(rw * 0.35, rh * 0.55) : Math.min(rw, rh) * 0.5
            parts.push(roundedRectSvg(rx, ry, rw, rh, topFlush ? 0 : r, topFlush ? 0 : r, r, r))
        }
        return parts.join(" ")
    }

    Flickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.contentPadding * 2
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: column
            readonly property real maxContentWidth: 1680
            width: Math.min(parent.width - Theme.contentPadding * 2, maxContentWidth)
            x: Math.round((parent.width - width) / 2)
            y: Theme.contentPadding
            spacing: Theme.gutter

            // Gentle "What's new" banner for returning or upgraded users.
            Card {
                id: whatsNewCard
                width: column.width
                visible: Preferences.walkthroughCompleted && !Preferences.whatsNewDismissed
                padding: 16
                Item {
                    width: parent.width
                    height: Math.max(whatsNewTextCol.implicitHeight, whatsNewBtnRow.implicitHeight)
                    Column {
                        id: whatsNewTextCol
                        anchors.left: parent.left
                        anchors.right: whatsNewBtnRow.left
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 4
                        Txt {
                            width: parent.width
                            role: "title"
                            text: qsTr("New: Features guide")
                            wrapMode: Text.WordWrap
                        }
                        Txt {
                            width: parent.width
                            role: "bodySmall"
                            muted: true
                            text: qsTr("See what each feature needs and where to find it.")
                            wrapMode: Text.WordWrap
                        }
                    }
                    Row {
                        id: whatsNewBtnRow
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8
                        Button {
                            variant: "text"
                            size: "sm"
                            text: qsTr("Dismiss")
                            onClicked: Preferences.whatsNewDismissed = true
                        }
                        Button {
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.book
                            text: qsTr("Open features guide")
                            onClicked: {
                                Preferences.whatsNewDismissed = true
                                AppController.currentPage = "guide"
                            }
                        }
                    }
                }
            }

            // A one-time invitation to try Hive, after the What's new card.
            Card {
                id: hiveInvite
                width: column.width
                visible: Preferences.walkthroughCompleted && Preferences.whatsNewDismissed
                         && !Preferences.hiveInviteDismissed && !Theme.hive
                padding: 14
                Binding {
                    target: Theme
                    property: "combPreviewShown"
                    value: hiveInvite.visible
                }
                Item {
                    width: parent.width
                    height: Math.max(hiveSample.height, hiveInviteText.implicitHeight, hiveInviteButtons.implicitHeight)

                    // A small window onto the comb, in Hive's own colors.
                    Rectangle {
                        id: hiveSample
                        anchors.verticalCenter: parent.verticalCenter
                        width: 56; height: 56
                        radius: 14
                        color: Theme.dark ? "#121214" : "#FFF8F3"
                        border.width: 1
                        border.color: Theme.dark ? Qt.rgba(1.0, 0.73, 0.37, 0.24) : Qt.rgba(0.54, 0.32, 0.0, 0.20)
                        HiveBackdrop {
                            anchors.fill: parent
                            corner: parent.radius
                            ownClock: false
                            growIn: false
                            time: Theme.combTime
                            animate: !Theme.reduceMotion && !AppController.powerSaver
                            cell: 9
                            lineColor: Theme.dark ? Qt.rgba(1.0, 0.73, 0.37, 0.28) : Qt.rgba(0.54, 0.32, 0.0, 0.16)
                            glowColor: Theme.dark ? Qt.rgba(1.0, 0.62, 0.18, 0.70) : Qt.rgba(1.0, 0.72, 0.37, 0.75)
                            origin: Qt.point(width / 2, height / 2)
                        }
                        Icon {
                            anchors.centerIn: parent
                            width: 22; height: 22
                            path: Icons.mark
                            stroke: 2.2
                            color: Theme.dark ? "#FFB95F" : "#8A5100"
                        }
                    }
                    Column {
                        id: hiveInviteText
                        anchors.left: hiveSample.right
                        anchors.leftMargin: 14
                        anchors.right: hiveInviteButtons.left
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 4
                        Txt {
                            width: parent.width
                            role: "title"
                            text: qsTr("Try Hive, the new theme")
                            wrapMode: Text.WordWrap
                        }
                        Txt {
                            width: parent.width
                            role: "bodySmall"
                            muted: true
                            text: qsTr("Honey on charcoal or warm wax, with a living honeycomb behind the app. Change it any time in Settings › Appearance.")
                            wrapMode: Text.WordWrap
                        }
                    }
                    Row {
                        id: hiveInviteButtons
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8
                        Button {
                            variant: "text"
                            size: "sm"
                            text: qsTr("Not now")
                            onClicked: Preferences.hiveInviteDismissed = true
                        }
                        Button {
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.sparkle
                            text: qsTr("Try Hive")
                            onClicked: {
                                Preferences.hiveInviteDismissed = true
                                Preferences.theme = "hive"
                            }
                        }
                    }
                }
            }

            // Card shown when no phone is paired yet (for example after skipping first-run setup).
            Card {
                width: column.width
                visible: DeviceList.count === 0
                padding: 20
                Item {
                    width: parent.width
                    height: Math.max(noDeviceTextCol.implicitHeight, noDeviceBtnRow.implicitHeight)
                    Column {
                        id: noDeviceTextCol
                        anchors.left: parent.left
                        anchors.right: noDeviceBtnRow.left
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 4
                        Txt {
                            width: parent.width
                            role: "title"
                            text: qsTr("No phone paired yet")
                            wrapMode: Text.WordWrap
                        }
                        Txt {
                            width: parent.width
                            role: "bodySmall"
                            muted: true
                            text: qsTr("Pair your Android phone over Wi-Fi or USB.")
                            wrapMode: Text.WordWrap
                        }
                    }
                    Row {
                        id: noDeviceBtnRow
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8
                        Button {
                            variant: "text"
                            size: "sm"
                            iconPath: Icons.book
                            text: qsTr("Features guide")
                            onClicked: AppController.currentPage = "guide"
                        }
                        Button {
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.compass
                            text: qsTr("Setup walkthrough")
                            onClicked: Preferences.restartWalkthrough()
                        }
                        Button {
                            size: "sm"
                            iconPath: Icons.plus
                            text: qsTr("Pair a phone")
                            onClicked: page.pairRequested()
                        }
                    }
                }
            }

            // Device switcher (only with more than one device).
            Row {
                spacing: 8
                visible: DeviceList.count > 1
                Repeater {
                    model: DeviceList
                    delegate: Chip {
                        required property int index
                        required property string name
                        required property string kind
                        interactive: true
                        text: name
                        iconPath: kind === "tablet" || kind === "phone" ? Icons.phone : Icons.laptop
                        selected: index === page.current
                        onClicked: page.current = index
                    }
                }
            }

            Repeater {
                model: DeviceList
                delegate: DeviceHome {
                    required property int index
                    visible: index === page.current
                    width: visible ? column.width : 0
                }
            }

            Card {
                width: column.width
                visible: DeviceList.count === 0 && TransferList.count > 0
                Column {
                    width: parent.width
                    spacing: 8
                    Item {
                        width: parent.width
                        height: Math.max(standaloneTransfersHeader.height, standaloneClearTransfers.height)
                        Txt {
                            id: standaloneTransfersHeader
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: qsTr("Transfers")
                        }
                        Button {
                            id: standaloneClearTransfers
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            visible: TransferList.count > TransferList.active
                            variant: "text"
                            size: "sm"
                            text: qsTr("Clear finished")
                            onClicked: TransferList.clearFinished()
                        }
                    }
                    Column {
                        id: standaloneTransferList
                        width: parent.width
                        Repeater {
                            model: TransferList
                            delegate: TransferItem { width: standaloneTransferList.width }
                        }
                    }
                }
            }

            LocalSendCard {
                width: column.width
                visible: DeviceList.count === 0 && AppController.localsendEnabled && page.localsendPeers.length > 0
            }
        }
    }

    // Files dragged onto the page are sent to the device on screen.
    // Holding Shift while dropping a single document hands it off so it opens
    // with the default app upon arrival.
    DropArea {
        id: drop
        anchors.fill: parent
        enabled: page.currentCanReceive
        onEntered: (drag) => drag.accepted = drag.hasUrls
        onDropped: (drag) => {
            if (drag.hasUrls) {
                const urls = drag.urls.map(url => url.toString())
                if ((drag.modifiers & Qt.ShiftModifier) && urls.length === 1)
                    TransferList.sendHandoff(page.currentDeviceId, urls)
                else
                    TransferList.send(page.currentDeviceId, urls)
                drag.accept(Qt.CopyAction)
            }
        }
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: Theme.contentPadding / 2
        radius: Theme.radiusXl
        visible: opacity > 0
        opacity: drop.containsDrag ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        color: Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b, 0.92)
        border.width: 2
        border.color: Theme.primary
        Column {
            anchors.centerIn: parent
            spacing: 8
            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 40; height: 40
                stroke: 1.6
                path: Icons.send
                color: Theme.primaryContainerContent
            }
            Txt {
                anchors.horizontalCenter: parent.horizontalCenter
                role: "headline"
                color: Theme.primaryContainerContent
                text: qsTr("Drop to send to %1").arg(page.currentDeviceName)
            }
            Txt {
                anchors.horizontalCenter: parent.horizontalCenter
                role: "bodySmall"
                color: Theme.primaryContainerContent
                opacity: 0.85
                text: qsTr("Hold Shift while dropping a document to open it on arrival")
            }
        }
    }

    // A conversation in the Messages widget.
    component ThreadRow: Item {
        id: tr
        property var thread: ({})
        readonly property bool unread: (thread.unread || 0) > 0
        height: 46
        Rectangle {
            id: trAvatar
            anchors.verticalCenter: parent.verticalCenter
            width: 34
            height: 34
            radius: 17
            color: Theme.secondaryContainer
            Txt {
                anchors.centerIn: parent
                role: "label"
                weight: 700
                color: Theme.secondaryContainerContent
                text: (tr.thread.title || "?").trim().charAt(0).toUpperCase()
            }
        }
        Column {
            anchors.left: trAvatar.right
            anchors.leftMargin: 10
            anchors.right: trWhen.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            Txt {
                width: parent.width
                role: "body"
                size: 13
                weight: tr.unread ? 700 : 500
                elide: Text.ElideRight
                maximumLineCount: 1
                text: (tr.thread.title || "").replace(/\s+/g, " ").trim()
            }
            Txt {
                width: parent.width
                role: "caption"
                muted: !tr.unread
                elide: Text.ElideRight
                maximumLineCount: 1
                text: (tr.thread.snippet || "").replace(/\s+/g, " ").trim()
            }
        }
        Column {
            id: trWhen
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4
            Txt {
                anchors.right: parent.right
                role: "caption"
                muted: true
                text: tr.thread.date ? page.relativeTime(tr.thread.date > 1e12 ? tr.thread.date / 1000 : tr.thread.date) : ""
            }
            Rectangle {
                anchors.right: parent.right
                visible: tr.unread
                width: 8
                height: 8
                radius: 4
                color: Theme.primary
            }
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: AppController.currentPage = "messages"
        }
    }

    // A call in the Recent calls widget, with Call back.
    component CallRow: Item {
        id: cr
        property var call: ({})
        property bool canCall: true
        readonly property bool missed: call.direction === "missed"
        height: 38
        Icon {
            id: crDir
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            path: cr.missed ? Icons.callMissed : cr.call.direction === "incoming" ? Icons.callIncoming : Icons.callOutgoing
            color: cr.missed ? Theme.error : Theme.surfaceContentVariant
        }
        Column {
            anchors.left: crDir.right
            anchors.leftMargin: 10
            anchors.right: crBack.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            Txt {
                width: parent.width
                role: "body"
                size: 13
                color: cr.missed ? Theme.error : Theme.surfaceContent
                elide: Text.ElideRight
                text: cr.call.name && cr.call.name.length > 0 ? cr.call.name : (cr.call.number || "")
            }
            Txt {
                width: parent.width
                role: "caption"
                muted: true
                elide: Text.ElideRight
                text: cr.call.date ? page.relativeTime(cr.call.date > 1e12 ? cr.call.date / 1000 : cr.call.date) : ""
            }
        }
        IconButton {
            id: crBack
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            visible: cr.canCall && (cr.call.number || "").length > 0
            iconPath: Icons.call
            label: qsTr("Call back")
            onClicked: PhoneCall.dial(cr.call.number)
        }
    }

    // A one-line text field in a widget (city, stock symbols). Enter applies it.
    component WidgetInput: Rectangle {
        id: wi
        property string placeholder
        signal accepted(string text)
        height: 32
        radius: 16
        color: Theme.surfaceContainerHigh
        border.width: wiInput.activeFocus ? 1 : 0
        border.color: Theme.primary
        TextInput {
            id: wiInput
            anchors.fill: parent
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            verticalAlignment: TextInput.AlignVCenter
            color: Theme.surfaceContent
            selectionColor: Theme.primary
            font.pixelSize: 12
            clip: true
            selectByMouse: true
            Accessible.name: wi.placeholder
            onAccepted: {
                wi.accepted(text)
                text = ""
                focus = false
            }
        }
        Txt {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            visible: wiInput.text.length === 0 && !wiInput.activeFocus
            role: "caption"
            muted: true
            elide: Text.ElideRight
            text: wi.placeholder
        }
    }

    // A widget beside the phone: a header that opens its page, then content.
    component GlanceCard: Rectangle {
        id: glance
        property string title
        property string iconPath
        property string target
        default property alias content: glanceBody.data
        height: glanceBody.y + glanceBody.implicitHeight + 16
        radius: Theme.radiusLg
        color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.42 : 0.60)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.40)
        Item {
            id: glanceHead
            x: 16
            y: 12
            width: parent.width - 32
            height: 26
            Icon {
                id: glanceIcon
                anchors.verticalCenter: parent.verticalCenter
                width: 16
                height: 16
                path: glance.iconPath
                color: Theme.primary
            }
            Txt {
                anchors.left: glanceIcon.right
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                role: "title"
                size: 14
                text: glance.title
            }
            Row {
                id: seeAll
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                visible: glance.target.length > 0
                spacing: 2
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    role: "label"
                    color: Theme.primary
                    text: qsTr("See all")
                }
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14
                    height: 14
                    path: Icons.chevronRight
                    color: Theme.primary
                }
            }
            // Over the link, not inside its Row: sized to the Row, it would resize it forever.
            MouseArea {
                anchors.fill: seeAll
                anchors.margins: -4
                visible: seeAll.visible
                cursorShape: Qt.PointingHandCursor
                onClicked: AppController.currentPage = glance.target
            }
        }
        Column {
            id: glanceBody
            x: 16
            y: glanceHead.y + glanceHead.height + 8
            width: parent.width - 32
            spacing: 6
        }
    }

    // A round button in the phone-shaped Home's dock.
    component PhoneDockButton: Item {
        id: dockBtn
        property string iconPath
        property string label
        property bool available: true
        property bool active: false
        signal clicked
        width: 46
        height: 46
        opacity: available ? 1 : 0.4
        Accessible.role: Accessible.Button
        Accessible.name: label
        activeFocusOnTab: available
        Keys.onSpacePressed: if (available) clicked()
        Keys.onReturnPressed: if (available) clicked()
        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: dockBtn.active ? Theme.primary
                 : (dockMouse.containsMouse && dockBtn.available) || dockBtn.activeFocus
                   ? Theme.surfaceContainerHighest : Theme.surfaceContainerHigh
            scale: dockMouse.pressed && dockBtn.available ? 0.92 : 1
            Behavior on scale { NumberAnimation { duration: 90 } }
        }
        Icon {
            anchors.centerIn: parent
            width: 20
            height: 20
            path: dockBtn.iconPath
            color: dockBtn.active ? Theme.primaryContent : Theme.surfaceContent
        }
        Rectangle {
            visible: dockMouse.containsMouse
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.top
            anchors.bottomMargin: 8
            width: dockTip.implicitWidth + 16
            height: 24
            radius: 12
            color: Theme.surfaceContainerHighest
            Txt {
                id: dockTip
                anchors.centerIn: parent
                role: "caption"
                text: dockBtn.label
            }
        }
        MouseArea {
            id: dockMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: dockBtn.available ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: if (dockBtn.available) dockBtn.clicked()
        }
    }

    component DeviceHome: Row {
        id: home
        required property string deviceId
        required property string name
        required property string rawName
        required property bool hasNickname
        required property string kind
        required property string osVersion
        required property string model
        required property bool online
        required property string path
        required property int rttMs
        required property int battery
        required property bool charging
        required property string batteryPlugged
        required property int batteryFullIn
        required property int cellBars
        required property string cellType
        required property int wifiBars
        required property string power
        required property real lastSeen
        required property real pairedAt
        required property string accent
        required property string screen
        required property bool reconnect

        // Dropped files go to the device on screen (and Messages shows it).
        // Kept while the page is hidden.
        Binding { target: page; property: "currentDeviceId"; value: home.deviceId; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding { target: page; property: "currentDeviceName"; value: home.name; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding {
            target: page
            property: "currentCanReceive"
            value: home.feature("files.send").state === "available"
                   || (!home.online
                       && home.feature("files.send").state !== "locked"
                       && home.feature("files.send").state !== "unsupported")
            when: home.visible
        }

        // Re-read capabilities whenever any matrix changes.
        function feature(id) {
            return AppController.capsRevision >= 0 ? AppController.featureState(deviceId, id) : ({})
        }
        readonly property var screenData: {
            if (!home.screen || home.screen.length === 0)
                return null
            try {
                return JSON.parse(home.screen)
            } catch (e) {
                return null
            }
        }
        readonly property var summaryData: {
            if (AppController.homeSummaryRevision < 0)
                return ({})
            const raw = AppController.homeSummary(home.deviceId)
            if (!raw || raw.length === 0)
                return ({})
            try {
                return JSON.parse(raw)
            } catch (e) {
                return ({})
            }
        }
        readonly property bool canAnimateHero: !Theme.reduceMotion && !page.resizing && home.visible && page.visible
                                               && Qt.application.state === Qt.ApplicationActive
        readonly property bool ringing: AppController.ringingDevices.includes(home.deviceId)

        spacing: Theme.gutter
        readonly property real sideWidth: Math.round(Math.min(450, Math.max(320, home.width * 0.34)))

        Column {
            width: home.width - home.sideWidth - home.spacing
            spacing: Theme.gutter

            // ---- Phone: Home drawn as the phone itself ----
            // A phone-shaped dashboard (status bar, lock-screen clock, media,
            // notifications, dock), or with Live on, the phone's real screen.
            Item {
                id: phoneStage
                visible: page.phoneLayout
                width: parent.width
                height: Math.max(phoneFrame.height, stageSide.implicitHeight) + 8

                property bool live: false
                property date now: new Date()
                Timer {
                    interval: 15000
                    repeat: true
                    running: phoneStage.visible
                    onTriggered: phoneStage.now = new Date()
                }

                // The phone's screen session (session 0), whoever started it.
                readonly property var liveInfo: {
                    try {
                        return JSON.parse(Mirror.windows).find(w => w.device === home.deviceId && w.session === 0) || null
                    } catch (e) {
                        return null
                    }
                }
                readonly property string liveKey: liveInfo ? liveInfo.key : ""
                readonly property string livePhase: liveInfo ? (liveInfo.phase || "") : ""
                readonly property bool liveControl: liveInfo ? liveInfo.canControl === true : false
                readonly property bool canLive: home.online && home.feature("mirroring.view").state === "available"

                // Last known status, so the status bar stays filled (dimmed) offline.
                property int knownBattery: -1
                property bool knownCharging: false
                property int knownCellBars: -1
                property string knownCellType: ""
                property int knownWifiBars: -1
                property bool knownBluetooth: false
                function remember() {
                    if (!home.online)
                        return
                    if (home.battery >= 0) { knownBattery = home.battery; knownCharging = home.charging }
                    knownCellBars = home.cellBars
                    knownCellType = home.cellType
                    knownWifiBars = home.wifiBars
                    if (phoneControls.state)
                        knownBluetooth = phoneControls.state.bluetooth === true
                    if (home.rttMs >= 1)
                        rttLog = rttLog.concat([home.rttMs]).slice(-30)
                }
                Timer {
                    interval: 5000
                    repeat: true
                    triggeredOnStart: true
                    running: home.online
                    onTriggered: phoneStage.remember()
                }
                property var rttLog: []

                Settings {
                    id: widgetPrefs
                    location: StandardPaths.writableLocation(StandardPaths.AppConfigLocation) + "/home-widgets.ini"
                    property string batteryLog: "{}"
                    property string city: ""
                    property real lat: 0
                    property real lon: 0
                }

                // Battery samples for the last 24 hours, kept per phone.
                readonly property var batteryLog: {
                    try {
                        return (JSON.parse(widgetPrefs.batteryLog) || {})[home.deviceId] || []
                    } catch (e) {
                        return []
                    }
                }
                function logBattery() {
                    if (!home.online || home.battery < 0)
                        return
                    const now = Date.now()
                    let all = {}
                    try { all = JSON.parse(widgetPrefs.batteryLog) || {} } catch (e) {}
                    const log = (all[home.deviceId] || []).filter(p => now - p.t <= 24 * 3600 * 1000)
                    const last = log[log.length - 1]
                    if (last && now - last.t < 9 * 60 * 1000 && last.b === home.battery)
                        return
                    log.push({ t: now, b: home.battery })
                    all[home.deviceId] = log
                    widgetPrefs.batteryLog = JSON.stringify(all)
                }
                Timer {
                    interval: 10 * 60 * 1000
                    repeat: true
                    triggeredOnStart: true
                    running: home.online
                    onTriggered: phoneStage.logBattery()
                }

                // Timeline events per hour, last 12 hours (oldest first).
                property var activity: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                function countActivity() {
                    const now = Date.now()
                    const b = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                    for (let i = 0; i < timelineRows.count; i++) {
                        const o = timelineRows.objectAt(i)
                        if (!o)
                            continue
                        const t = o.timestamp > 1e12 ? o.timestamp : o.timestamp * 1000
                        const h = Math.floor((now - t) / 3600000)
                        if (h >= 0 && h < 12)
                            b[11 - h]++
                    }
                    activity = b
                }
                Instantiator {
                    id: timelineRows
                    model: TimelineModel
                    delegate: QtObject { required property real timestamp }
                    onObjectAdded: activityRecount.restart()
                    onObjectRemoved: activityRecount.restart()
                }
                Timer { id: activityRecount; interval: 400; onTriggered: phoneStage.countActivity() }
                Timer { interval: 60000; repeat: true; running: phoneStage.visible; onTriggered: phoneStage.countActivity() }

                // Weather for the lock screen: the city you set, from Open-Meteo.
                property var weather: null
                property string weatherError: ""
                readonly property string weatherLine: {
                    if (widgetPrefs.city.length === 0)
                        return weatherError.length > 0 ? weatherError : qsTr("Set your city for weather")
                    if (!weather || !weather.current)
                        return weatherError.length > 0 ? weatherError : widgetPrefs.city
                    return qsTr("%1° · %2 · %3").arg(Math.round(weather.current.temperature_2m))
                        .arg(describeWeather(weather.current.weather_code)).arg(widgetPrefs.city)
                }
                function describeWeather(code) {
                    if (code === 0) return qsTr("Clear")
                    if (code <= 3) return qsTr("Partly cloudy")
                    if (code <= 48) return qsTr("Fog")
                    if (code <= 57) return qsTr("Drizzle")
                    if (code <= 67) return qsTr("Rain")
                    if (code <= 77) return qsTr("Snow")
                    if (code <= 82) return qsTr("Showers")
                    return qsTr("Thunderstorm")
                }
                function loadWeather() {
                    if (widgetPrefs.city.length === 0)
                        return
                    page.getJson("https://api.open-meteo.com/v1/forecast?latitude=" + widgetPrefs.lat
                                 + "&longitude=" + widgetPrefs.lon + "&current=temperature_2m,weather_code&timezone=auto",
                                 (data) => { phoneStage.weather = data; phoneStage.weatherError = "" },
                                 () => phoneStage.weatherError = qsTr("Weather unavailable"))
                }
                function setCity(name) {
                    if (name.trim().length === 0)
                        return
                    page.getJson("https://geocoding-api.open-meteo.com/v1/search?count=1&name=" + encodeURIComponent(name.trim()),
                                 (data) => {
                                     const r = data.results && data.results[0]
                                     if (!r) {
                                         phoneStage.weatherError = qsTr("No place called “%1”").arg(name.trim())
                                         return
                                     }
                                     widgetPrefs.city = r.name
                                     widgetPrefs.lat = r.latitude
                                     widgetPrefs.lon = r.longitude
                                     phoneStage.weather = null
                                     phoneStage.loadWeather()
                                 },
                                 () => phoneStage.weatherError = qsTr("Weather unavailable"))
                }
                Timer {
                    interval: 30 * 60 * 1000
                    repeat: true
                    triggeredOnStart: true
                    running: phoneStage.visible
                    onTriggered: phoneStage.loadWeather()
                }

                // What the widgets beside the phone show.
                readonly property var threads: {
                    try {
                        return JSON.parse(Messages.threads) || []
                    } catch (e) {
                        return []
                    }
                }
                readonly property var calls: {
                    try {
                        return JSON.parse(PhoneCall.callLog) || []
                    } catch (e) {
                        return []
                    }
                }
                function ago(date) {
                    if (!date)
                        return ""
                    return page.relativeTime(date > 1e12 ? date / 1000 : date)
                }
                function loadGlance() {
                    if (!home.online || !home.visible)
                        return
                    Messages.open(home.deviceId)
                    PhoneCall.open(home.deviceId)
                    Photos.open(home.deviceId)
                }
                Component.onCompleted: Qt.callLater(loadGlance)

                function startLive() {
                    if (!canLive)
                        return
                    HomeLive.embed(home.deviceId)
                    live = true
                    if (!liveInfo || livePhase === "ended")
                        Mirror.start(home.deviceId)
                }
                function stopLive() {
                    if (!live)
                        return
                    live = false
                    if (liveKey.length > 0)
                        Mirror.stop(liveKey)
                    HomeLive.release(home.deviceId)
                }
                // Moves the live screen into a window of its own; the session keeps going.
                function popOut() {
                    live = false
                    HomeLive.release(home.deviceId)
                    if (!liveInfo || livePhase === "ended")
                        Mirror.start(home.deviceId)
                }
                Connections {
                    target: home
                    function onOnlineChanged() {
                        if (!home.online)
                            phoneStage.stopLive()
                        else
                            phoneStage.loadGlance()
                    }
                    function onVisibleChanged() {
                        if (!home.visible)
                            phoneStage.stopLive()
                    }
                }
                Component.onDestruction: stopLive()

                // ---- The phone ----
                Item {
                    id: phoneFrame
                    y: 4
                    height: Math.round(Math.min(800, Math.max(620, page.height - 120)))
                    width: Math.round(height * phoneArt.aspect)
                    readonly property real bezel: Math.max(8, Math.round(width * 0.032))
                    readonly property real innerW: width - bezel * 2
                    readonly property real innerH: height - bezel * 2
                    function corner(k) {
                        const s = home.screenData
                        return s && s.corners ? Math.max(14, s.corners[k] * width) : width * 0.13
                    }
                    readonly property real rTl: corner("tl")
                    readonly property real rTr: corner("tr")
                    readonly property real rBr: corner("br")
                    readonly property real rBl: corner("bl")
                    readonly property color chassis: Theme.dark ? "#0B0C0F" : "#17181C"
                    readonly property color ink: phoneArt.screenInk
                    readonly property string cutoutPath: {
                        const s = home.screenData
                        if (s && s.cutout_path && s.cutout_path.length > 0)
                            return page.scaleSvgPath(s.cutout_path, innerW, innerH, bezel, bezel)
                        return ""
                    }
                    // Android reports a punch hole as a box reaching the top
                    // edge; at this size draw the round camera inside it.
                    readonly property var hole: {
                        const s = home.screenData
                        const c = s && s.cutouts && s.cutouts.length > 0 ? s.cutouts[0] : null
                        const d = Math.round(Math.max(9, Math.min(16, width * 0.036)))
                        if (!c || c.w <= 0 || c.h <= 0)
                            return { x: (width - d) / 2, y: bezel + 10, d: d }
                        const cx = bezel + (c.x + c.w / 2) * innerW
                        const cy = bezel + (c.y + c.h / 2) * innerH
                        return { x: cx - d / 2, y: cy - d / 2, d: d }
                    }

                    // Accent glow while connected.
                    Rectangle {
                        anchors.centerIn: parent
                        width: parent.width + 40
                        height: parent.height + 40
                        radius: phoneFrame.rTl + 20
                        color: Theme.graphite ? Theme.primary : phoneArt.seedColor
                        opacity: home.online ? (Theme.dark ? 0.20 : 0.14) : 0.0
                        visible: opacity > 0
                        Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
                    }

                    // Side keys: volume and power.
                    Rectangle { x: phoneFrame.width - 1; y: phoneFrame.height * 0.20; width: 3.5; height: phoneFrame.height * 0.09; radius: 2; color: Qt.lighter(phoneFrame.chassis, 1.9) }
                    Rectangle { x: phoneFrame.width - 1; y: phoneFrame.height * 0.32; width: 3.5; height: phoneFrame.height * 0.05; radius: 2; color: Qt.lighter(phoneFrame.chassis, 1.9) }

                    // ---- The screen ----
                    Item {
                        id: phoneScreen
                        x: phoneFrame.bezel
                        y: phoneFrame.bezel
                        width: phoneFrame.innerW
                        height: phoneFrame.innerH
                        clip: true

                        // Wallpaper, in the phone's own colors.
                        Rectangle {
                            anchors.fill: parent
                            gradient: Gradient {
                                GradientStop { position: 0.0; color: phoneArt.screenTop }
                                GradientStop { position: 1.0; color: phoneArt.screenBottom }
                            }
                        }
                        Rectangle {
                            x: -parent.width * 0.3
                            y: -parent.width * 0.4
                            width: parent.width * 1.2
                            height: width
                            radius: width / 2
                            color: phoneArt.seedColor
                            opacity: Theme.dark ? 0.22 : 0.18
                        }

                        // ---- Dashboard ----
                        Item {
                            id: dash
                            anchors.fill: parent
                            visible: !phoneStage.live

                            // Status bar, drawn the way the phone draws it.
                            Item {
                                id: statusBar
                                x: 26
                                y: 11
                                width: parent.width - 52
                                height: 22
                                readonly property var toggles: phoneControls.state
                                readonly property color ink: phoneFrame.ink
                                Txt {
                                    anchors.left: parent.left
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "label"
                                    size: 12
                                    weight: 600
                                    color: statusBar.ink
                                    text: Qt.formatTime(phoneStage.now, "hh:mm")
                                }
                                Row {
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 5
                                    // Last known while offline, dimmed.
                                    opacity: home.online ? 1 : 0.55
                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: !!statusBar.toggles && statusBar.toggles.bluetooth === true || (!home.online && phoneStage.knownBluetooth)
                                        width: 12
                                        height: 12
                                        path: Icons.bluetooth
                                        color: statusBar.ink
                                    }
                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownWifiBars >= 0 || (!!statusBar.toggles && statusBar.toggles.wifi === true)
                                        width: 13
                                        height: 13
                                        path: Icons.wifi
                                        color: statusBar.ink
                                    }
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownCellBars >= 0 && phoneStage.knownCellType.length > 0
                                        role: "label"
                                        size: 9
                                        weight: 700
                                        color: statusBar.ink
                                        text: phoneStage.knownCellType
                                    }
                                    Item {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownCellBars >= 0
                                        width: 4 * 2.5 + 3 * 1.5
                                        height: 10
                                        Repeater {
                                            model: 4
                                            Rectangle {
                                                required property int index
                                                x: index * 4
                                                width: 2.5
                                                height: 3 + index * 2.3
                                                y: 10 - height
                                                radius: 1
                                                color: statusBar.ink
                                                opacity: index < phoneStage.knownCellBars ? 1 : 0.3
                                            }
                                        }
                                    }
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownBattery >= 0
                                        leftPadding: 2
                                        role: "label"
                                        size: 10
                                        weight: 600
                                        color: statusBar.ink
                                        text: phoneStage.knownBattery + "%"
                                    }
                                    // A small battery, nub on the right.
                                    Item {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownBattery >= 0
                                        width: 19
                                        height: 9
                                        Rectangle {
                                            width: 17
                                            height: 9
                                            radius: 2.5
                                            color: "transparent"
                                            border.width: 1
                                            border.color: Qt.rgba(statusBar.ink.r, statusBar.ink.g, statusBar.ink.b, 0.6)
                                            Rectangle {
                                                x: 1.5
                                                y: 1.5
                                                height: parent.height - 3
                                                width: Math.max(1, (parent.width - 3) * Math.max(0, Math.min(100, phoneStage.knownBattery)) / 100)
                                                radius: 1.5
                                                color: phoneStage.knownCharging ? "#4ADE80"
                                                     : phoneStage.knownBattery <= 15 ? "#F87171" : statusBar.ink
                                            }
                                        }
                                        Rectangle {
                                            x: 17.5
                                            y: 3
                                            width: 1.5
                                            height: 3
                                            radius: 0.75
                                            color: Qt.rgba(statusBar.ink.r, statusBar.ink.g, statusBar.ink.b, 0.6)
                                        }
                                    }
                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: phoneStage.knownCharging && phoneStage.knownBattery >= 0
                                        width: 10
                                        height: 10
                                        path: Icons.bolt
                                        color: statusBar.ink
                                    }
                                }
                            }

                            Flickable {
                                id: dashFlick
                                y: statusBar.y + statusBar.height + 4
                                width: parent.width
                                height: dock.y - y - 10
                                contentWidth: width
                                contentHeight: dashCol.height + 12
                                clip: true
                                boundsBehavior: Flickable.StopAtBounds

                                Column {
                                    id: dashCol
                                    x: 12
                                    width: dashFlick.width - 24
                                    spacing: 10

                                    // Lock-screen clock.
                                    Column {
                                        width: parent.width
                                        topPadding: 22
                                        bottomPadding: 12
                                        spacing: 0
                                        Txt {
                                            anchors.horizontalCenter: parent.horizontalCenter
                                            role: "displayLarge"
                                            size: Math.round(phoneScreen.width * 0.21)
                                            weight: 300
                                            color: phoneFrame.ink
                                            text: Qt.formatTime(phoneStage.now, "hh:mm")
                                        }
                                        Txt {
                                            anchors.horizontalCenter: parent.horizontalCenter
                                            role: "body"
                                            size: 14
                                            color: phoneFrame.ink
                                            opacity: 0.85
                                            text: Qt.formatDate(phoneStage.now, "dddd, d MMMM")
                                        }
                                        // Weather, a line on the lock screen (Open-Meteo, no account).
                                        Txt {
                                            id: lockWeather
                                            anchors.horizontalCenter: parent.horizontalCenter
                                            topPadding: 4
                                            visible: !weatherEdit.visible
                                            role: "label"
                                            color: phoneFrame.ink
                                            opacity: 0.9
                                            text: phoneStage.weatherLine
                                            MouseArea {
                                                anchors.fill: parent
                                                anchors.margins: -6
                                                cursorShape: Qt.PointingHandCursor
                                                onClicked: weatherEdit.visible = true
                                            }
                                        }
                                        WidgetInput {
                                            id: weatherEdit
                                            anchors.horizontalCenter: parent.horizontalCenter
                                            width: Math.min(220, dashCol.width)
                                            visible: false
                                            placeholder: qsTr("City, then Enter")
                                            onAccepted: (text) => {
                                                visible = false
                                                phoneStage.setCity(text)
                                            }
                                        }
                                        Txt {
                                            anchors.horizontalCenter: parent.horizontalCenter
                                            topPadding: 8
                                            visible: !home.online
                                            role: "label"
                                            color: phoneFrame.ink
                                            opacity: 0.8
                                            text: !home.reconnect ? qsTr("Not reconnecting")
                                                  : home.lastSeen > 0 ? qsTr("Offline · %1").arg(page.relativeTime(home.lastSeen))
                                                  : qsTr("Offline")
                                        }
                                    }

                                    // Now playing.
                                    Repeater {
                                        model: MediaList
                                        delegate: NowPlaying {
                                            required property int index
                                            visible: MediaList.revision >= 0 && index === heroCard.activeMediaRow
                                            width: visible ? dashCol.width : 0
                                            embeddedInHero: true
                                            hasMultipleSources: heroCard.hasBothMedia
                                            phoneVolume: phoneControls.state ? phoneControls.state.volume : 65
                                            onSourceSelected: (src) => heroCard.preferredMediaSource = src
                                        }
                                    }

                                    // Notification shade.
                                    Item {
                                        width: parent.width
                                        height: 26
                                        visible: phoneFeed.count > 0
                                        Txt {
                                            anchors.left: parent.left
                                            anchors.leftMargin: 4
                                            anchors.verticalCenter: parent.verticalCenter
                                            role: "label"
                                            color: phoneFrame.ink
                                            opacity: 0.85
                                            text: qsTr("Notifications · %1").arg(phoneFeed.count)
                                        }
                                        Txt {
                                            id: phoneClearAll
                                            anchors.right: parent.right
                                            anchors.rightMargin: 4
                                            anchors.verticalCenter: parent.verticalCenter
                                            role: "label"
                                            color: phoneFrame.ink
                                            text: qsTr("Clear all")
                                            MouseArea {
                                                anchors.fill: parent
                                                anchors.margins: -6
                                                cursorShape: Qt.PointingHandCursor
                                                onClicked: NotificationList.dismissAll(home.deviceId)
                                            }
                                        }
                                    }
                                    Rectangle {
                                        width: parent.width
                                        height: phoneUndo.height + 14
                                        visible: NotificationList.undoLabel.length > 0
                                        radius: 16
                                        color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.62 : 0.74)
                                        Row {
                                            id: phoneUndo
                                            x: 12
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: parent.width - 24
                                            spacing: 8
                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                width: parent.width - phoneUndoBtn.width - 8
                                                role: "bodySmall"
                                                elide: Text.ElideRight
                                                text: NotificationList.undoLabel
                                            }
                                            Button {
                                                id: phoneUndoBtn
                                                anchors.verticalCenter: parent.verticalCenter
                                                variant: "tonal"
                                                size: "sm"
                                                text: qsTr("Undo")
                                                onClicked: NotificationList.undoDismiss()
                                            }
                                        }
                                    }
                                    Rectangle {
                                        id: phoneFeed
                                        readonly property int count: NotificationList.count >= 0 ? NotificationList.countFor(home.deviceId) : 0
                                        width: parent.width
                                        height: phoneNotifs.height + 12
                                        visible: count > 0
                                        radius: 20
                                        color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.55 : 0.70)
                                        border.width: 1
                                        border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.08 : 0.35)
                                        Timer {
                                            id: phoneFeedClock
                                            property int minutes: 0
                                            interval: 60000
                                            repeat: true
                                            running: phoneFeed.count > 0
                                            onTriggered: minutes++
                                        }
                                        Column {
                                            id: phoneNotifs
                                            x: 4
                                            y: 6
                                            width: parent.width - 8
                                            spacing: 2
                                            Repeater {
                                                model: NotificationList
                                                delegate: NotificationItem {
                                                    width: phoneNotifs.width
                                                    clock: phoneFeedClock.minutes
                                                    visible: deviceId === home.deviceId
                                                    height: visible ? implicitHeight : 0
                                                    canOpenApp: home.feature("mirroring.app_windows").state === "available" && app.length > 0
                                                    onOpenAppRequested: Mirror.startApp(deviceId, app, appName.length > 0 ? appName : app)
                                                    onOptionsRequested: appSheet.openFor(app, appName)
                                                    onImageClicked: (url) => imagePreviewSheet.openFor(url, title.length > 0 ? title : appName)
                                                }
                                            }
                                        }
                                    }
                                    Txt {
                                        width: parent.width
                                        visible: phoneFeed.count === 0 && home.online
                                        horizontalAlignment: Text.AlignHCenter
                                        role: "bodySmall"
                                        color: phoneFrame.ink
                                        opacity: 0.7
                                        wrapMode: Text.WordWrap
                                        text: home.feature("notifications.mirror").state === "available"
                                              ? qsTr("No notifications") : qsTr("Notifications unavailable")
                                    }
                                }
                            }

                            // Dock.
                            Rectangle {
                                id: dock
                                x: 14
                                width: parent.width - 28
                                height: 66
                                y: parent.height - height - 26
                                radius: 28
                                color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.48 : 0.60)
                                border.width: 1
                                border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.40)
                                Row {
                                    anchors.centerIn: parent
                                    spacing: Math.max(8, (dock.width - 4 * 46) / 5)
                                    PhoneDockButton {
                                        iconPath: Icons.mirror
                                        label: qsTr("Mirror screen in a window")
                                        available: home.online && home.feature("mirroring.view").state === "available"
                                        active: phoneStage.livePhase.length > 0 && phoneStage.livePhase !== "ended"
                                        onClicked: phoneStage.popOut()
                                    }
                                    PhoneDockButton {
                                        iconPath: Icons.send
                                        label: qsTr("Send files")
                                        available: page.currentCanReceive
                                        onClicked: filePicker.open()
                                    }
                                    PhoneDockButton {
                                        iconPath: Icons.clipboard
                                        label: qsTr("Send clipboard")
                                        available: home.feature("clipboard.pc_to_phone").state === "available"
                                                   || (!home.online && home.feature("clipboard.pc_to_phone").state !== "locked"
                                                       && home.feature("clipboard.pc_to_phone").state !== "unsupported")
                                        onClicked: AppController.sendClipboard(home.deviceId)
                                    }
                                    PhoneDockButton {
                                        iconPath: Icons.ring
                                        label: home.ringing ? qsTr("Stop ringing") : qsTr("Find phone")
                                        available: home.online && home.feature("device.find_phone").state === "available"
                                        active: home.ringing
                                        onClicked: {
                                            home.ringing = !home.ringing
                                            if (home.ringing)
                                                ringTimeout.restart()
                                            AppController.ring(home.deviceId, home.ringing)
                                        }
                                    }
                                }
                            }
                        }

                        // ---- Live: the real screen ----
                        Rectangle {
                            anchors.fill: parent
                            visible: phoneStage.live
                            color: "black"
                        }
                        Loader {
                            id: liveLoader
                            anchors.fill: parent
                            active: phoneStage.live && phoneStage.liveKey.length > 0
                            sourceComponent: VideoView {
                                id: liveVideo
                                stream: phoneStage.liveKey
                                focus: true
                                acceptsText: phoneStage.livePhase === "showing" && phoneStage.liveControl
                                onTextCommitted: (text) => Mirror.text(phoneStage.liveKey, text)

                                function at(x, y) {
                                    const r = pictureRect()
                                    return { x: (x - r.x) / Math.max(1, r.width), y: (y - r.y) / Math.max(1, r.height),
                                             inside: x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height }
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
                                    property bool down: false
                                    property real lastMove: 0
                                    onPressed: (mouse) => {
                                        liveVideo.forceActiveFocus()
                                        if (!phoneStage.liveControl) {
                                            Mirror.requestControl(phoneStage.liveKey)
                                            return
                                        }
                                        if (mouse.button === Qt.RightButton) { Mirror.press(phoneStage.liveKey, "back"); return }
                                        if (mouse.button === Qt.MiddleButton) { Mirror.press(phoneStage.liveKey, "home"); return }
                                        const p = liveVideo.at(mouse.x, mouse.y)
                                        if (!p.inside)
                                            return
                                        down = true
                                        Mirror.touch(phoneStage.liveKey, "down", p.x, p.y)
                                    }
                                    onPositionChanged: (mouse) => {
                                        if (!down || !phoneStage.liveControl)
                                            return
                                        const now = Date.now()
                                        if (now - lastMove < 16)
                                            return
                                        lastMove = now
                                        const p = liveVideo.at(mouse.x, mouse.y)
                                        Mirror.touch(phoneStage.liveKey, "move", p.x, p.y)
                                    }
                                    onReleased: (mouse) => {
                                        if (!down)
                                            return
                                        down = false
                                        if (!phoneStage.liveControl)
                                            return
                                        const p = liveVideo.at(mouse.x, mouse.y)
                                        Mirror.touch(phoneStage.liveKey, "up", p.x, p.y)
                                    }
                                    onWheel: (wheel) => {
                                        if (!phoneStage.liveControl) {
                                            Mirror.requestControl(phoneStage.liveKey)
                                            return
                                        }
                                        const p = liveVideo.at(wheel.x, wheel.y)
                                        if (p.inside)
                                            Mirror.scroll(phoneStage.liveKey, p.x, p.y, -wheel.angleDelta.x / 120, -wheel.angleDelta.y / 120)
                                    }
                                }

                                Keys.onPressed: (event) => {
                                    if (!phoneStage.liveControl)
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
                                    if (keys[event.key] !== undefined) {
                                        Mirror.press(phoneStage.liveKey, keys[event.key])
                                        event.accepted = true
                                    }
                                }
                            }
                        }
                        // What the live screen is doing, until the picture arrives.
                        Column {
                            anchors.centerIn: parent
                            width: parent.width - 48
                            spacing: 12
                            visible: phoneStage.live && phoneStage.livePhase !== "showing"
                            Spinner {
                                anchors.horizontalCenter: parent.horizontalCenter
                                visible: phoneStage.livePhase !== "ended"
                            }
                            Txt {
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.WordWrap
                                role: "body"
                                color: "#FFFFFF"
                                text: phoneStage.livePhase === "ended" ? qsTr("The live screen ended")
                                      : qsTr("Asking %1 for its screen…").arg(home.name)
                            }
                            Button {
                                anchors.horizontalCenter: parent.horizontalCenter
                                visible: phoneStage.livePhase === "ended"
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Start again")
                                onClicked: Mirror.start(home.deviceId)
                            }
                        }
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: 28
                            visible: phoneStage.live && phoneStage.livePhase === "showing" && !phoneStage.liveControl
                            width: controlHint.implicitWidth + 28
                            height: 34
                            radius: 17
                            color: Qt.rgba(0, 0, 0, 0.66)
                            Txt {
                                id: controlHint
                                anchors.centerIn: parent
                                role: "label"
                                color: "#FFFFFF"
                                text: qsTr("Click to control the phone")
                            }
                        }

                        // Gesture bar.
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: parent.height - 12
                            width: Math.round(parent.width * 0.34)
                            height: 4
                            radius: 2
                            visible: !phoneStage.live
                            color: phoneFrame.ink
                            opacity: 0.55
                        }
                    }

                    // Chassis over the screen's corners, the outline, and the camera cutout.
                    Shape {
                        anchors.fill: parent
                        preferredRendererType: Shape.CurveRenderer
                        ShapePath {
                            strokeWidth: -1
                            fillColor: phoneFrame.chassis
                            fillRule: ShapePath.OddEvenFill
                            PathSvg {
                                path: page.roundedRectSvg(0, 0, phoneFrame.width, phoneFrame.height,
                                                          phoneFrame.rTl, phoneFrame.rTr, phoneFrame.rBr, phoneFrame.rBl)
                                      + " " + page.roundedRectSvg(phoneFrame.bezel, phoneFrame.bezel, phoneFrame.innerW, phoneFrame.innerH,
                                                                  Math.max(4, phoneFrame.rTl - phoneFrame.bezel),
                                                                  Math.max(4, phoneFrame.rTr - phoneFrame.bezel),
                                                                  Math.max(4, phoneFrame.rBr - phoneFrame.bezel),
                                                                  Math.max(4, phoneFrame.rBl - phoneFrame.bezel))
                            }
                        }
                        ShapePath {
                            strokeWidth: 1.5
                            strokeColor: Qt.rgba(1, 1, 1, Theme.dark ? 0.16 : 0.30)
                            fillColor: "transparent"
                            PathSvg {
                                path: page.roundedRectSvg(0.75, 0.75, phoneFrame.width - 1.5, phoneFrame.height - 1.5,
                                                          phoneFrame.rTl, phoneFrame.rTr, phoneFrame.rBr, phoneFrame.rBl)
                            }
                        }
                        ShapePath {
                            strokeWidth: -1
                            fillColor: "#000000"
                            PathSvg { path: "" }
                        }
                    }
                    // The punch-hole camera. Android reports it as a box (or an outline
                    // with that box's shape); a round lens reads truer at this size.
                    Rectangle {
                        width: phoneFrame.hole.d
                        height: phoneFrame.hole.d
                        radius: width / 2
                        x: Math.round(phoneFrame.hole.x)
                        y: Math.round(phoneFrame.hole.y)
                        color: "#000000"
                        border.width: 1
                        border.color: "#1E2027"
                    }
                }

                // ---- Beside the phone: two columns, or one when narrow ----
                Grid {
                    id: stageSide
                    x: phoneFrame.width + 32
                    y: 4
                    width: Math.max(260, parent.width - x)
                    columns: width >= 820 ? 2 : 1
                    columnSpacing: 16
                    rowSpacing: 16
                    readonly property real colAWidth: columns === 2 ? Math.round((width - columnSpacing) * 0.46) : width
                    readonly property real colBWidth: columns === 2 ? width - colAWidth - columnSpacing : width

                    // ---- Identity, Live, messages, photos, calls ----
                    Column {
                        width: stageSide.colAWidth
                        spacing: 14

                        Row {
                            width: parent.width
                            spacing: 6
                            Txt {
                                id: stageName
                                width: Math.min(implicitWidth, parent.width - stageEdit.width - parent.spacing)
                                role: "displaySmall"
                                size: 28
                                elide: Text.ElideRight
                                text: home.name
                            }
                            IconButton {
                                id: stageEdit
                                anchors.verticalCenter: stageName.verticalCenter
                                iconPath: Icons.edit
                                iconSize: 14
                                label: home.hasNickname ? qsTr("Change nickname") : qsTr("Set nickname")
                                onClicked: nicknameSheet.openFor(home.deviceId, home.name, home.rawName)
                            }
                        }
                        Txt {
                            width: parent.width
                            role: "body"
                            muted: true
                            visible: text.length > 0
                            text: home.osVersion.length > 0 ? qsTr("Android %1").arg(home.osVersion) : ""
                        }
                        Flow {
                            width: parent.width
                            spacing: 6
                            Chip {
                                interactive: true
                                onClicked: AppController.runDoctor()
                                iconPath: !home.online ? Icons.unlink
                                          : home.path === "usb" ? Icons.usb
                                          : home.path === "relay" ? Icons.globe
                                          : (home.wifiBars < 0 && home.cellBars >= 0 ? Icons.signal : Icons.wifi)
                                text: home.online
                                      ? (home.path === "relay" ? qsTr("Away")
                                         : home.path === "usb" ? qsTr("USB") : qsTr("Wi‑Fi"))
                                        + (home.rttMs >= 1 ? qsTr(" · %1 ms").arg(home.rttMs) : "")
                                      : !home.reconnect ? qsTr("Not reconnecting")
                                      : (home.lastSeen > 0 ? qsTr("Offline · %1").arg(page.relativeTime(home.lastSeen)) : qsTr("Offline"))
                            }
                            Chip {
                                visible: home.online && home.charging
                                iconPath: Icons.bolt
                                text: {
                                    const eta = page.formatChargeEta(home.batteryFullIn)
                                    return eta.length > 0 ? eta : qsTr("Charging")
                                }
                            }
                            Chip {
                                visible: home.reconnect
                                interactive: true
                                text: home.online ? qsTr("Disconnect") : qsTr("Stop reconnecting")
                                onClicked: AppController.setReconnect(home.deviceId, false)
                            }
                            Chip {
                                visible: !home.online
                                interactive: true
                                text: home.reconnect ? qsTr("Troubleshoot") : qsTr("Reconnect")
                                onClicked: home.reconnect ? AppController.runDoctor() : AppController.setReconnect(home.deviceId, true)
                            }
                        }

                        // The Live switch.
                        GlanceCard {
                            width: parent.width
                            title: qsTr("Live screen")
                            iconPath: Icons.mirror
                            target: ""
                            Item {
                                width: parent.width
                                height: Math.max(liveText.implicitHeight, liveToggle.height)
                                Txt {
                                    id: liveText
                                    anchors.left: parent.left
                                    anchors.right: liveToggle.left
                                    anchors.rightMargin: 12
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "caption"
                                    muted: true
                                    wrapMode: Text.WordWrap
                                    text: phoneStage.live
                                          ? qsTr("Click to tap, scroll to swipe, right-click for Back.")
                                          : phoneStage.canLive
                                            ? qsTr("Shows the phone's real screen in the frame. Uses more battery while on.")
                                            : qsTr("Needs the phone connected with screen mirroring available.")
                                }
                                Toggle {
                                    id: liveToggle
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    label: qsTr("Live screen")
                                    enabled: phoneStage.canLive || phoneStage.live
                                    opacity: enabled ? 1 : 0.5
                                    checked: phoneStage.live
                                    onToggled: (on) => on ? phoneStage.startLive() : phoneStage.stopLive()
                                }
                            }
                            Flow {
                                width: parent.width
                                spacing: 8
                                visible: phoneStage.live
                                Button { variant: "tonal"; size: "sm"; iconPath: Icons.back; text: qsTr("Back"); onClicked: Mirror.press(phoneStage.liveKey, "back") }
                                Button { variant: "tonal"; size: "sm"; iconPath: Icons.home; text: qsTr("Home"); onClicked: Mirror.press(phoneStage.liveKey, "home") }
                                Button { variant: "tonal"; size: "sm"; iconPath: Icons.lock; text: qsTr("Lock"); onClicked: Mirror.press(phoneStage.liveKey, "lock") }
                                Button { variant: "tonal"; size: "sm"; iconPath: Icons.openExternal; text: qsTr("Pop out"); onClicked: phoneStage.popOut() }
                            }
                        }

                        GlanceCard {
                            width: parent.width
                            title: qsTr("Messages")
                            iconPath: Icons.messages
                            target: "messages"
                            Repeater {
                                model: phoneStage.threads.slice(0, 3)
                                delegate: ThreadRow {
                                    required property var modelData
                                    width: parent.width
                                    thread: modelData
                                }
                            }
                            Txt {
                                width: parent.width
                                visible: phoneStage.threads.length === 0
                                role: "bodySmall"
                                muted: true
                                text: home.online ? qsTr("No recent messages") : qsTr("Connect to see messages")
                            }
                        }

                        GlanceCard {
                            width: parent.width
                            title: qsTr("Photos")
                            iconPath: Icons.photo
                            target: "photos"
                            ListView {
                                width: parent.width
                                height: Photos.count > 0 ? 72 : 0
                                visible: Photos.count > 0
                                orientation: ListView.Horizontal
                                interactive: false
                                clip: true
                                spacing: 6
                                model: Photos
                                delegate: Item {
                                    required property string itemId
                                    required property string thumb
                                    required property bool isVideo
                                    width: 72
                                    height: 72
                                    Rectangle {
                                        anchors.fill: parent
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerHigh
                                    }
                                    RoundedImage {
                                        anchors.fill: parent
                                        radius: Theme.radiusMd
                                        fillMode: Image.PreserveAspectCrop
                                        source: thumb
                                    }
                                    Icon {
                                        anchors.right: parent.right
                                        anchors.bottom: parent.bottom
                                        anchors.margins: 5
                                        visible: isVideo
                                        width: 14
                                        height: 14
                                        path: Icons.play
                                        color: "#FFFFFF"
                                    }
                                    Component.onCompleted: if (thumb.length === 0 && itemId.length > 0) Photos.needThumb(itemId)
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: AppController.currentPage = "photos"
                                    }
                                }
                            }
                            Txt {
                                width: parent.width
                                visible: Photos.count === 0
                                role: "bodySmall"
                                muted: true
                                text: home.online ? qsTr("No photos yet") : qsTr("Connect to see photos")
                            }
                        }

                        GlanceCard {
                            width: parent.width
                            title: qsTr("Recent calls")
                            iconPath: Icons.call
                            target: "calls"
                            Repeater {
                                model: phoneStage.calls.slice(0, 3)
                                delegate: CallRow {
                                    required property var modelData
                                    width: parent.width
                                    canCall: home.online
                                    call: modelData
                                }
                            }
                            Txt {
                                width: parent.width
                                visible: phoneStage.calls.length === 0
                                role: "bodySmall"
                                muted: true
                                text: home.online ? qsTr("No recent calls") : qsTr("Connect to see calls")
                            }
                        }
                    }

                    // ---- Widgets, then the radio ----
                    Column {
                        width: stageSide.colBWidth
                        spacing: 14
                    Grid {
                        id: widgetGrid
                        width: parent.width
                        columns: width >= 400 ? 2 : 1
                        spacing: 14
                        readonly property real cellW: (width - spacing * (columns - 1)) / columns

                        // Phone controls: quick toggles and media volume.
                        GlanceCard {
                            width: widgetGrid.cellW
                            title: qsTr("Device controls")
                            iconPath: Icons.phone
                            target: ""
                            Row {
                                visible: home.online && !!phoneControls.state
                                spacing: 8
                                Repeater {
                                    model: [
                                        { id: "dnd", icon: Icons.moon, label: qsTr("Do not disturb") },
                                        { id: "flashlight", icon: Icons.flashlight, label: qsTr("Torch") },
                                        { id: "wifi", icon: Icons.wifi, label: qsTr("Wi‑Fi") },
                                        { id: "bluetooth", icon: Icons.bluetooth, label: qsTr("Bluetooth") }
                                    ]
                                    delegate: PhoneDockButton {
                                        required property var modelData
                                        readonly property var st: phoneControls.state
                                        width: 40
                                        height: 40
                                        iconPath: modelData.icon
                                        label: modelData.label
                                        active: !!st && st[modelData.id] === true
                                        onClicked: AppController.setPhoneToggle(home.deviceId, modelData.id, String(!active))
                                    }
                                }
                            }
                            ToggleSlider {
                                visible: home.online && !!phoneControls.state
                                width: parent.width
                                deviceId: home.deviceId
                                toggleId: "volume"
                                title: qsTr("Volume")
                                iconPath: shownValue === 0 ? Icons.soundOff : Icons.speaker
                                phoneValue: phoneControls.state ? phoneControls.state.volume : 0
                                feature: phoneControls.volumeFeature
                            }
                            Txt {
                                width: parent.width
                                visible: !home.online || !phoneControls.state
                                role: "bodySmall"
                                muted: true
                                wrapMode: Text.WordWrap
                                text: qsTr("Connect the phone to use its controls")
                            }
                        }

                        // Connection health: round-trip time, sampled every few seconds.
                        GlanceCard {
                            width: widgetGrid.cellW
                            title: qsTr("Connection health")
                            iconPath: Icons.signal
                            target: ""
                            Item {
                                id: rttChart
                                width: parent.width
                                height: 64
                                readonly property int maxRtt: Math.max(20, ...phoneStage.rttLog)
                                Row {
                                    height: 64
                                    spacing: 3
                                    Repeater {
                                        model: phoneStage.rttLog
                                        Rectangle {
                                            required property var modelData
                                            width: Math.max(3, (rttChart.width - 29 * 3) / 30)
                                            height: Math.max(3, 60 * modelData / rttChart.maxRtt)
                                            y: 64 - height
                                            radius: 1.5
                                            color: modelData > 150 ? Theme.error : modelData > 60 ? "#FBBF24" : Theme.primary
                                        }
                                    }
                                }
                            }
                            Txt {
                                width: parent.width
                                role: "caption"
                                muted: true
                                text: !home.online ? qsTr("Offline")
                                      : phoneStage.rttLog.length === 0 ? qsTr("Measuring…")
                                      : qsTr("%1 · %2 ms now · %3 ms average")
                                            .arg(home.path === "usb" ? qsTr("USB") : home.path === "relay" ? qsTr("Away") : qsTr("Wi‑Fi"))
                                            .arg(home.rttMs)
                                            .arg(Math.round(phoneStage.rttLog.reduce((a, b) => a + b, 0) / phoneStage.rttLog.length))
                            }
                        }

                        // Battery, with the last 24 hours as recorded while the app ran.
                        GlanceCard {
                            width: widgetGrid.cellW
                            title: qsTr("Battery")
                            iconPath: phoneStage.knownCharging ? Icons.bolt : Icons.battery
                            target: ""
                            Row {
                                spacing: 10
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "displaySmall"
                                    size: 30
                                    text: phoneStage.knownBattery >= 0 ? phoneStage.knownBattery + "%" : "–"
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "caption"
                                    muted: true
                                    text: !home.online ? (phoneStage.knownBattery >= 0 ? qsTr("Last known") : qsTr("Not known yet"))
                                          : home.charging
                                            ? (page.formatChargeEta(home.batteryFullIn).length > 0 ? page.formatChargeEta(home.batteryFullIn) : qsTr("Charging"))
                                            : home.battery >= 0 && home.battery <= 15 ? qsTr("Low battery") : qsTr("On battery")
                                }
                            }
                            Canvas {
                                width: parent.width
                                height: 64
                                property var points: phoneStage.batteryLog
                                onPointsChanged: requestPaint()
                                onWidthChanged: requestPaint()
                                onPaint: {
                                    const ctx = getContext("2d")
                                    ctx.reset()
                                    const now = Date.now(), span = 24 * 3600 * 1000
                                    ctx.strokeStyle = Qt.rgba(Theme.outlineVariant.r, Theme.outlineVariant.g, Theme.outlineVariant.b, 0.6)
                                    ctx.lineWidth = 1
                                    for (const f of [0, 0.5, 1]) {
                                        ctx.beginPath()
                                        ctx.moveTo(0, 2 + f * (height - 4))
                                        ctx.lineTo(width, 2 + f * (height - 4))
                                        ctx.stroke()
                                    }
                                    const pts = points.filter(p => now - p.t <= span)
                                    if (pts.length < 2)
                                        return
                                    const xOf = t => width * (1 - (now - t) / span)
                                    const yOf = b => 2 + (height - 4) * (1 - b / 100)
                                    const grad = ctx.createLinearGradient(0, 0, 0, height)
                                    grad.addColorStop(0, Qt.rgba(0.29, 0.87, 0.5, 0.45))
                                    grad.addColorStop(1, Qt.rgba(0.29, 0.87, 0.5, 0.02))
                                    ctx.beginPath()
                                    ctx.moveTo(xOf(pts[0].t), height)
                                    for (const p of pts)
                                        ctx.lineTo(xOf(p.t), yOf(p.b))
                                    ctx.lineTo(xOf(pts[pts.length - 1].t), height)
                                    ctx.closePath()
                                    ctx.fillStyle = grad
                                    ctx.fill()
                                    ctx.beginPath()
                                    pts.forEach((p, i) => i === 0 ? ctx.moveTo(xOf(p.t), yOf(p.b)) : ctx.lineTo(xOf(p.t), yOf(p.b)))
                                    ctx.strokeStyle = "#4ADE80"
                                    ctx.lineWidth = 2
                                    ctx.stroke()
                                }
                            }
                            Txt {
                                width: parent.width
                                role: "caption"
                                muted: true
                                wrapMode: Text.WordWrap
                                text: phoneStage.batteryLog.length < 2 ? qsTr("The last 24 hours fill in while NectarLink runs")
                                                                       : qsTr("Last 24 hours")
                            }
                        }

                        // Activity: Timeline events per hour, last 12 hours.
                        GlanceCard {
                            width: widgetGrid.cellW
                            title: qsTr("Activity")
                            iconPath: Icons.history
                            target: "timeline"
                            Item {
                                id: activityChart
                                width: parent.width
                                height: 64
                                readonly property int peak: Math.max(1, ...phoneStage.activity)
                                Row {
                                    height: 64
                                    spacing: 4
                                    Repeater {
                                        model: phoneStage.activity
                                        Rectangle {
                                            required property var modelData
                                            width: Math.max(4, (activityChart.width - 11 * 4) / 12)
                                            height: Math.max(2, 60 * modelData / activityChart.peak)
                                            y: 64 - height
                                            radius: 2
                                            color: Theme.primary
                                            opacity: modelData > 0 ? 0.9 : 0.25
                                        }
                                    }
                                }
                            }
                            Txt {
                                width: parent.width
                                role: "caption"
                                muted: true
                                text: qsTr("%1 events in the last 12 hours").arg(phoneStage.activity.reduce((a, b) => a + b, 0))
                            }
                        }

                    }

                    // Internet radio, drawn as an analogue tuner. Only where Qt
                    // Multimedia is installed: otherwise there's no radio, and no warning.
                    Loader {
                        width: parent.width
                        active: page.radioAvailable
                        visible: status === Loader.Ready
                        source: Qt.resolvedUrl("../components/CarRadio.qml")
                        onLoaded: item.width = Qt.binding(() => width)
                    }
                    }
                }
            }

            // ---- Hero ----
            Rectangle {
                id: heroCard
                visible: !page.phoneLayout
                width: parent.width
                readonly property bool wide: width >= 540
                readonly property int phoneMediaRow: MediaList.revision >= 0 ? MediaList.firstFor(home.deviceId) : -1
                readonly property int localMediaRow: MediaList.revision >= 0 ? MediaList.firstFor("local") : -1
                readonly property bool hasBothMedia: phoneMediaRow >= 0 && localMediaRow >= 0
                readonly property bool phoneMediaPlaying: phoneMediaRow >= 0 && MediaList.revision >= 0 && MediaList.isPlaying(phoneMediaRow)
                readonly property bool localMediaPlaying: localMediaRow >= 0 && MediaList.revision >= 0 && MediaList.isPlaying(localMediaRow)
                property string preferredMediaSource: ""
                onPhoneMediaPlayingChanged: {
                    if (phoneMediaPlaying && !localMediaPlaying && preferredMediaSource !== "phone")
                        Qt.callLater(() => {
                            if (phoneMediaPlaying && !localMediaPlaying)
                                preferredMediaSource = "phone"
                        })
                }
                onLocalMediaPlayingChanged: {
                    if (localMediaPlaying && !phoneMediaPlaying && preferredMediaSource !== "local")
                        Qt.callLater(() => {
                            if (localMediaPlaying && !phoneMediaPlaying)
                                preferredMediaSource = "local"
                        })
                }
                readonly property int activeMediaRow: {
                    if (preferredMediaSource === "local" && localMediaRow >= 0)
                        return localMediaRow
                    if (preferredMediaSource === "phone" && phoneMediaRow >= 0)
                        return phoneMediaRow
                    if (localMediaPlaying && !phoneMediaPlaying && localMediaRow >= 0)
                        return localMediaRow
                    if (phoneMediaPlaying && !localMediaPlaying && phoneMediaRow >= 0)
                        return phoneMediaRow
                    if (phoneMediaRow >= 0)
                        return phoneMediaRow
                    return localMediaRow
                }
                readonly property bool hasGlanceSummary: true
                height: wide
                    ? Math.max(188, Math.max(phoneArt.height, Math.max(identityCol.height, summaryPanel.height)) + 36)
                    : topHeroRow.height + 1 + summaryPanel.height + 60
                Behavior on height {
                    enabled: home.canAnimateHero
                    NumberAnimation { duration: 220; easing.type: Easing.OutCubic }
                }
                radius: Theme.radiusXl
                color: Theme.heroColor
                border.width: (Theme.graphite || Theme.amoled) && !Theme.glass ? 1 : 0
                border.color: Theme.outlineVariant
                Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }

                GlassRim { radius: heroCard.radius }

                Item {
                    id: topHeroRow
                    x: 18
                    y: heroCard.wide ? Math.round((heroCard.height - height) / 2) : 20
                    width: heroCard.wide
                        ? heroCard.width - 36 - summaryPanel.width - 28
                        : heroCard.width - 36
                    height: Math.max(phoneArt.height, identityCol.height)

                    // Vector front-only phone preview measured on the phone, with live status bar & circular battery arc inside.
                    Item {
                        id: phoneArt
                        anchors.verticalCenter: parent.verticalCenter
                        readonly property real aspect: {
                            const s = home.screenData
                            if (s && s.aspect >= 0.25 && s.aspect <= 2.5)
                                return s.aspect
                            return home.kind === "tablet" ? 0.70 : 0.47
                        }
                        height: heroCard.wide ? 156 : 142
                        width: Math.round(height * aspect)
                        readonly property real bezel: 3.5
                        readonly property real innerW: Math.max(10, width - bezel * 2)
                        readonly property real innerH: Math.max(20, height - bezel * 2)
                        readonly property real defaultCorner: width * 0.14
                        readonly property real rTl: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.tl * width) : defaultCorner
                        readonly property real rTr: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.tr * width) : defaultCorner
                        readonly property real rBr: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.br * width) : defaultCorner
                        readonly property real rBl: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.bl * width) : defaultCorner
                        readonly property var phonePalette: Theme.phonePalette(home.deviceId)
                        // Hive keeps its own honey here too, whatever the phone's colors.
                        readonly property color seedColor: Theme.hive
                            ? Theme.primary
                            : phonePalette
                            ? Qt.color(phonePalette.primary)
                            : (home.accent && home.accent.length > 0 ? Qt.color(home.accent) : Theme.primary)
                        readonly property color frameFill: Theme.hive
                            ? (Theme.dark ? "#0E0E10" : "#2A2420")
                            : Theme.graphite
                            ? (Theme.dark ? "#1B1D21" : "#282B30")
                            : Qt.darker(seedColor, Theme.dark ? 2.8 : 2.3)
                        readonly property color frameStroke: Theme.graphite
                            ? Theme.outline
                            : Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.42)
                        readonly property color screenTop: Theme.hive
                            ? (Theme.dark ? "#3A2709" : "#FFE2BC")
                            : Theme.graphite
                            ? (Theme.dark ? "#262930" : "#E4E7EC")
                            : (Theme.dark ? Qt.darker(seedColor, 1.85) : Qt.lighter(seedColor, 1.35))
                        readonly property color screenBottom: Theme.hive
                            ? (Theme.dark ? "#141210" : "#F1C27F")
                            : Theme.graphite
                            ? (Theme.dark ? "#17191D" : "#CFD4DC")
                            : (Theme.dark ? Qt.darker(seedColor, 2.55) : Qt.darker(seedColor, 1.25))
                        readonly property color screenInk: Theme.hive
                            ? (Theme.dark ? "#FFDDB3" : "#2C1600")
                            : (Theme.graphite && !Theme.dark ? "#1E2025" : "#FFFFFF")
                        readonly property string cutoutSvg: {
                            const s = home.screenData
                            if (!s) return ""
                            if (s.cutout_path && s.cutout_path.length > 0)
                                return page.scaleSvgPath(s.cutout_path, innerW, innerH, bezel, bezel)
                            if (s.cutouts && s.cutouts.length > 0)
                                return page.cutoutsToSvgPath(s.cutouts, innerW, innerH, bezel, bezel)
                            return ""
                        }

                        opacity: home.online ? 1.0 : 0.56
                        Behavior on opacity {
                            enabled: home.canAnimateHero
                            NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic }
                        }

                        // Soft accent glow behind the phone front while connected.
                        Rectangle {
                            anchors.centerIn: parent
                            width: parent.width + 18
                            height: parent.height + 18
                            radius: Math.max(phoneArt.rTl, phoneArt.rTr) + 9
                            color: Theme.graphite ? Theme.primary : phoneArt.seedColor
                            opacity: home.online ? (Theme.dark ? 0.24 : 0.18) : 0.0
                            visible: opacity > 0
                            Behavior on opacity {
                                enabled: home.canAnimateHero
                                NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic }
                            }
                        }

                        Shape {
                            id: phoneShape
                            anchors.fill: parent
                            preferredRendererType: Shape.CurveRenderer

                            // 1. Outer front chassis outline with measured per-corner radii.
                            ShapePath {
                                strokeWidth: 1.5
                                strokeColor: phoneArt.frameStroke
                                fillColor: phoneArt.frameFill
                                PathSvg {
                                    path: page.roundedRectSvg(
                                        0.75, 0.75,
                                        phoneArt.width - 1.5, phoneArt.height - 1.5,
                                        phoneArt.rTl, phoneArt.rTr, phoneArt.rBr, phoneArt.rBl
                                    )
                                }
                            }

                            // 2. Active front display surface.
                            ShapePath {
                                strokeWidth: -1
                                fillGradient: LinearGradient {
                                    x1: phoneArt.bezel; y1: phoneArt.bezel
                                    x2: phoneArt.width - phoneArt.bezel; y2: phoneArt.height - phoneArt.bezel
                                    GradientStop { position: 0.0; color: phoneArt.screenTop }
                                    GradientStop { position: 1.0; color: phoneArt.screenBottom }
                                }
                                PathSvg {
                                    path: page.roundedRectSvg(
                                        phoneArt.bezel, phoneArt.bezel,
                                        phoneArt.innerW, phoneArt.innerH,
                                        Math.max(2, phoneArt.rTl - phoneArt.bezel),
                                        Math.max(2, phoneArt.rTr - phoneArt.bezel),
                                        Math.max(2, phoneArt.rBr - phoneArt.bezel),
                                        Math.max(2, phoneArt.rBl - phoneArt.bezel)
                                    )
                                }
                            }

                        }

                        // Hive: the comb on the phone's screen, in step with the app's.
                        HiveBackdrop {
                            visible: Theme.hive
                            x: phoneArt.bezel
                            y: phoneArt.bezel
                            width: phoneArt.innerW
                            height: phoneArt.innerH
                            corner: Math.max(2, Math.min(phoneArt.rTl, phoneArt.rTr) - phoneArt.bezel)
                            ownClock: false
                            growIn: false
                            time: Theme.combTime
                            animate: Theme.combMotion
                            cell: 7
                            strength: home.online ? 1 : 0.6
                            frameMs: Theme.comb.frameMs
                            breathSeconds: Theme.comb.breathSeconds
                            pulseSeconds: Theme.comb.pulseSeconds
                            lineColor: Theme.dark ? Qt.rgba(1.0, 0.73, 0.37, 0.26) : Qt.rgba(0.54, 0.32, 0.0, 0.16)
                            glowColor: Theme.combGlow
                            origin: Qt.point(width / 2, height * 0.42)
                        }

                        // 3. Measured front camera cutout (SVG path or bounding rects; empty when plain/unknown).
                        Shape {
                            anchors.fill: parent
                            visible: phoneArt.cutoutSvg.length > 0
                            preferredRendererType: Shape.CurveRenderer
                            ShapePath {
                                strokeWidth: 0.8
                                strokeColor: Qt.rgba(1, 1, 1, Theme.dark ? 0.16 : 0.22)
                                fillColor: phoneArt.frameFill
                                PathSvg { path: phoneArt.cutoutSvg }
                            }
                        }

                        // Fallback punch-hole camera dot when no measured cutout path is reported
                        Rectangle {
                            visible: phoneArt.cutoutSvg.length === 0
                            width: 5
                            height: 5
                            radius: 2.5
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: phoneArt.bezel + 4
                            color: Qt.rgba(0, 0, 0, 0.72)
                            border.width: 0.5
                            border.color: Qt.rgba(1, 1, 1, 0.16)
                        }

                        // ---- Refined miniature status bar inside the phone screen ----
                        Item {
                            id: miniStatusBar
                            x: phoneArt.bezel + 5
                            y: phoneArt.bezel + 3.5
                            width: phoneArt.innerW - 10
                            height: 9
                            visible: home.online

                            // Left of punch-hole: 4 crisp cellular signal bars + compact 5G/LTE tag
                            Row {
                                anchors.left: parent.left
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1.5
                                visible: home.cellBars >= 0

                                Row {
                                    height: 6.5
                                    spacing: 1.1
                                    anchors.verticalCenter: parent.verticalCenter
                                    Repeater {
                                        model: 4
                                        Rectangle {
                                            required property int index
                                            width: 1.5
                                            height: 2.3 + index * 1.4
                                            anchors.bottom: parent.bottom
                                            radius: 0.75
                                            color: index < Math.min(4, Math.max(0, home.cellBars))
                                                ? phoneArt.screenInk
                                                : Qt.rgba(phoneArt.screenInk.r, phoneArt.screenInk.g, phoneArt.screenInk.b, 0.28)
                                        }
                                    }
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    size: 6
                                    weight: Font.Bold
                                    color: phoneArt.screenInk
                                    opacity: 0.90
                                    text: home.cellType.length > 0 && home.cellType.length <= 3 ? home.cellType : ""
                                    visible: text.length > 0
                                }
                            }

                            // Right of punch-hole: crisp Wi-Fi icon
                            Icon {
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                width: 8.5; height: 8.5
                                stroke: 1.45
                                visible: home.wifiBars >= 0
                                path: Icons.wifi
                                color: phoneArt.screenInk
                                opacity: 0.92
                            }
                        }

                        // ---- Center of phone screen: Sleek Lockscreen Circular Battery Ring ----
                        Column {
                            id: inPhoneBatteryWidget
                            anchors.centerIn: parent
                            anchors.verticalCenterOffset: 2
                            spacing: 4
                            visible: home.battery >= 0

                            readonly property color ringAccentColor: (home.battery <= 20 && !home.charging)
                                ? Theme.error
                                : (home.charging ? "#4ADE80" : phoneArt.screenInk)

                            Item {
                                width: 46
                                height: 46
                                anchors.horizontalCenter: parent.horizontalCenter

                                // Frosted circular glass dial well
                                Rectangle {
                                    anchors.fill: parent
                                    anchors.margins: 2
                                    radius: width / 2
                                    color: Qt.rgba(0, 0, 0, Theme.graphite && !Theme.dark ? 0.10 : 0.28)
                                    border.width: 0.75
                                    border.color: Qt.rgba(1, 1, 1, 0.16)
                                }

                                Shape {
                                    anchors.fill: parent
                                    preferredRendererType: Shape.CurveRenderer

                                    // Background circular ring track
                                    ShapePath {
                                        strokeColor: Qt.rgba(phoneArt.screenInk.r, phoneArt.screenInk.g, phoneArt.screenInk.b, 0.20)
                                        fillColor: "transparent"
                                        strokeWidth: 3.2
                                        capStyle: ShapePath.RoundCap
                                        PathAngleArc {
                                            centerX: 23; centerY: 23
                                            radiusX: 19.5; radiusY: 19.5
                                            startAngle: 0
                                            sweepAngle: 360
                                        }
                                    }

                                    // Active battery level arc
                                    ShapePath {
                                        strokeColor: inPhoneBatteryWidget.ringAccentColor
                                        fillColor: "transparent"
                                        strokeWidth: 3.2
                                        capStyle: ShapePath.RoundCap
                                        PathAngleArc {
                                            centerX: 23; centerY: 23
                                            radiusX: 19.5; radiusY: 19.5
                                            startAngle: -90
                                            sweepAngle: 360 * (Math.max(0, Math.min(100, home.battery)) / 100.0)
                                        }
                                    }
                                }

                                Column {
                                    anchors.centerIn: parent
                                    spacing: 0

                                    Icon {
                                        visible: home.charging
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        width: 8.5; height: 8.5
                                        stroke: 1.9
                                        path: Icons.bolt
                                        color: "#4ADE80"
                                    }

                                    Txt {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        size: 11
                                        weight: Font.Bold
                                        color: phoneArt.screenInk
                                        text: home.battery + "%"
                                    }
                                }
                            }

                            // Only show a pill below the battery ring when actively charging or low battery (never redundant LTE/Battery text)
                            Rectangle {
                                visible: statusMiniLabel.text.length > 0
                                anchors.horizontalCenter: parent.horizontalCenter
                                height: 13
                                width: Math.min(phoneArt.innerW - 6, Math.max(34, statusMiniLabel.implicitWidth + 10))
                                radius: 6.5
                                color: Qt.rgba(0, 0, 0, Theme.graphite && !Theme.dark ? 0.10 : 0.26)
                                border.width: 0.6
                                border.color: Qt.rgba(1, 1, 1, 0.14)

                                Txt {
                                    id: statusMiniLabel
                                    anchors.centerIn: parent
                                    width: parent.width - 6
                                    horizontalAlignment: Text.AlignHCenter
                                    elide: Text.ElideRight
                                    size: 6.5
                                    weight: Font.DemiBold
                                    color: phoneArt.screenInk
                                    opacity: 0.90
                                    text: {
                                        if (home.charging) {
                                            const eta = page.formatChargeEta(home.batteryFullIn)
                                            return eta.length > 0 ? eta : qsTr("Charging")
                                        }
                                        if (home.battery <= 20)
                                            return qsTr("Low")
                                        return ""
                                    }
                                }
                            }
                        }

                        // Subtle gesture pill at the bottom of the screen.
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: phoneArt.bezel + 4
                            width: Math.round(phoneArt.innerW * 0.36)
                            height: 2.5
                            radius: 1.25
                            color: Qt.rgba(phoneArt.screenInk.r, phoneArt.screenInk.g, phoneArt.screenInk.b, 0.38)
                        }
                    }

                    Column {
                        id: identityCol
                        anchors.left: phoneArt.right
                        anchors.leftMargin: 14
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 6
                        Row {
                            width: parent.width
                            spacing: 6
                            Txt {
                                id: nameTxt
                                width: Math.min(implicitWidth, parent.width - editNicknameBtn.width - parent.spacing)
                                text: home.name
                                role: "displaySmall"
                                size: home.name.length > 14 ? 20 : (home.name.length > 10 ? 22 : 25)
                                fontSizeMode: Text.HorizontalFit
                                minimumPixelSize: 16
                                color: Theme.heroContent
                                elide: Text.ElideRight
                            }
                            IconButton {
                                id: editNicknameBtn
                                anchors.verticalCenter: nameTxt.verticalCenter
                                iconPath: Icons.edit
                                iconSize: 14
                                label: home.hasNickname ? qsTr("Change nickname") : qsTr("Set nickname")
                                onClicked: nicknameSheet.openFor(home.deviceId, home.name, home.rawName)
                            }
                        }
                        Txt {
                            width: parent.width
                            role: "body"
                            color: Theme.heroContent
                            opacity: 0.78
                            elide: Text.ElideRight
                            visible: text.length > 0
                            // The model is often a code (like "SM-X000"), so only the version.
                            text: home.osVersion.length > 0 ? qsTr("Android %1").arg(home.osVersion) : ""
                        }
                        Flow {
                            width: parent.width
                            spacing: 6
                            Chip {
                                interactive: true
                                onClicked: AppController.runDoctor()
                                iconPath: !home.online ? Icons.unlink
                                          : home.path === "usb" ? Icons.usb
                                          : home.path === "relay" ? Icons.globe
                                          : (home.wifiBars < 0 && home.cellBars >= 0 ? Icons.signal : Icons.wifi)
                                text: home.online
                                      ? (home.path === "relay"
                                         ? (home.rttMs >= 1 ? qsTr("Away · %1 ms").arg(home.rttMs) : qsTr("Away"))
                                         : (home.path === "usb"
                                            ? (home.rttMs >= 1 ? qsTr("USB · %1 ms").arg(home.rttMs) : qsTr("USB"))
                                            : (home.wifiBars >= 0
                                               ? (home.rttMs >= 1 ? qsTr("Wi‑Fi · %1 ms").arg(home.rttMs) : qsTr("Wi‑Fi"))
                                               : (home.cellBars >= 0
                                                  ? ((home.cellType.length > 0 ? home.cellType : qsTr("Cellular"))
                                                     + (home.rttMs >= 1 ? qsTr(" · %1 ms").arg(home.rttMs) : ""))
                                                  : (home.rttMs >= 1 ? qsTr("Wi‑Fi · %1 ms").arg(home.rttMs) : qsTr("Wi‑Fi"))))))
                                      : !home.reconnect ? qsTr("Not reconnecting")
                                      : (home.lastSeen > 0 ? qsTr("Offline · %1").arg(page.relativeTime(home.lastSeen))
                                                           : qsTr("Offline"))
                            }
                            Chip {
                                visible: home.online && home.charging
                                iconPath: Icons.bolt
                                text: {
                                    const eta = page.formatChargeEta(home.batteryFullIn)
                                    return eta.length > 0 ? eta : qsTr("Charging")
                                }
                            }
                            // Stays paired, and stays disconnected until Reconnect (GitHub #48).
                            Chip {
                                visible: home.reconnect
                                interactive: true
                                text: home.online ? qsTr("Disconnect") : qsTr("Stop reconnecting")
                                onClicked: AppController.setReconnect(home.deviceId, false)
                            }
                        }
                        Button {
                            visible: !home.online
                            variant: "tonal"
                            size: "sm"
                            text: home.reconnect ? qsTr("Troubleshoot") : qsTr("Reconnect")
                            onClicked: home.reconnect ? AppController.runDoctor()
                                                      : AppController.setReconnect(home.deviceId, true)
                        }
                    }
                }

                // Vertical or horizontal divider between phone identity and media player.
                Rectangle {
                    id: summaryDivider
                    visible: heroCard.hasGlanceSummary
                    x: heroCard.wide ? heroCard.width - summaryPanel.width - 26 : 18
                    y: heroCard.wide ? 22 : topHeroRow.y + topHeroRow.height + 12
                    width: heroCard.wide ? 1 : heroCard.width - 36
                    height: heroCard.wide ? heroCard.height - 44 : 1
                    color: Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.14)
                }

                // Media Player (Phone + This PC) embedded in the Hero card's right half, scaling dynamically in wide/fullscreen.
                Column {
                    id: summaryPanel
                    visible: heroCard.hasGlanceSummary
                    width: heroCard.wide
                        ? Math.min(480, Math.max(256, Math.round(heroCard.width * 0.46)))
                        : heroCard.width - 36
                    x: heroCard.wide ? heroCard.width - width - 16 : 18
                    y: heroCard.wide
                        ? Math.round((heroCard.height - height) / 2)
                        : summaryDivider.y + summaryDivider.height + 12
                    spacing: 8

                    Repeater {
                        model: MediaList
                        delegate: NowPlaying {
                            required property int index
                            visible: MediaList.revision >= 0 && index === heroCard.activeMediaRow
                            width: visible ? summaryPanel.width : 0
                            embeddedInHero: true
                            hasMultipleSources: heroCard.hasBothMedia
                            phoneVolume: phoneControls.state ? phoneControls.state.volume : 65
                            onSourceSelected: (src) => heroCard.preferredMediaSource = src
                        }
                    }

                    // Calm glass standby media card when neither Phone nor PC has an active media session
                    Rectangle {
                        width: parent.width
                        height: idleMediaCol.implicitHeight + 24
                        visible: heroCard.activeMediaRow < 0
                        radius: Theme.radiusLg
                        color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.glass ? (Theme.dark ? 0.42 : 0.52) : (Theme.dark ? 0.44 : 0.58))
                        border.width: 1
                        border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.12 : 0.40)

                        Column {
                            id: idleMediaCol
                            x: 14
                            y: 12
                            width: parent.width - 28
                            spacing: 8

                            Row {
                                width: parent.width
                                spacing: 10
                                Rectangle {
                                    width: 40; height: 40
                                    radius: Theme.radiusMd
                                    color: Theme.secondaryContainer
                                    anchors.verticalCenter: parent.verticalCenter
                                    Icon {
                                        anchors.centerIn: parent
                                        width: 18; height: 18
                                        path: Icons.music
                                        color: Theme.secondaryContainerContent
                                    }
                                }
                                Column {
                                    width: parent.width - 50
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 1
                                    Txt {
                                        width: parent.width
                                        role: "title"
                                        size: 13
                                        text: qsTr("Media player")
                                        elide: Text.ElideRight
                                    }
                                    Txt {
                                        width: parent.width
                                        role: "caption"
                                        muted: true
                                        text: qsTr("Nothing playing")
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---- Quick actions ----
            Grid {
                id: actions
                visible: !page.phoneLayout
                width: parent.width
                columns: width > 640 ? 4 : 2
                spacing: 12
                readonly property real tileWidth: (width - spacing * (columns - 1)) / columns

                ActionTile {
                    width: actions.tileWidth
                    deviceOnline: home.online
                    title: qsTr("Mirror screen")
                    subtitle: ""
                    iconPath: Icons.mirror
                    feature: home.feature("mirroring.view")
                    // The phone's screen is showing (its session is 0).
                    active: {
                        try {
                            return JSON.parse(Mirror.windows).some(w => w.device === home.deviceId && w.session === 0
                                                                       && w.phase !== "ended")
                        } catch (e) {
                            return false
                        }
                    }
                    onClicked: Mirror.start(home.deviceId)
                    sideIcon: home.feature("mirroring.app_windows").state === "available" ? Icons.apps : ""
                    sideLabel: qsTr("Open an app")
                    onSideClicked: appsSheet.openFor(home.deviceId, home.name)
                }
                ActionTile {
                    width: actions.tileWidth
                    deviceOnline: home.online
                    queueableWhenOffline: true
                    title: qsTr("Send files")
                    subtitle: ""
                    iconPath: Icons.send
                    feature: home.feature("files.send")
                    onClicked: filePicker.open()
                    sideIcon: Icons.folder
                    sideLabel: qsTr("Send a folder")
                    onSideClicked: folderPicker.open()
                }
                ActionTile {
                    width: actions.tileWidth
                    deviceOnline: home.online
                    queueableWhenOffline: true
                    title: qsTr("Send clipboard")
                    subtitle: ""
                    iconPath: Icons.clipboard
                    feature: home.feature("clipboard.pc_to_phone")
                    onClicked: AppController.sendClipboard(home.deviceId)
                    sideIcon: Preferences.clipboardHistory ? Icons.clipboardList : ""
                    sideLabel: qsTr("Clipboard history")
                    sideAlwaysAvailable: true
                    onSideClicked: {
                        clipboardSearch.text = ""
                        clipboardSheet.open()
                    }
                }
                ActionTile {
                    width: actions.tileWidth
                    deviceOnline: home.online
                    title: home.ringing ? qsTr("Stop ringing") : qsTr("Find phone")
                    subtitle: ""
                    iconPath: Icons.ring
                    feature: home.feature("device.find_phone")
                    active: home.ringing
                    onClicked: AppController.ring(home.deviceId, !home.ringing)
                }
            }
            FileDialog {
                id: filePicker
                title: qsTr("Send to %1").arg(home.name)
                fileMode: FileDialog.OpenFiles
                onAccepted: TransferList.send(home.deviceId, selectedFiles.map(url => url.toString()))
            }
            FileDialog {
                id: handoffFilePicker
                title: qsTr("Hand off & open on %1").arg(home.name)
                fileMode: FileDialog.OpenFile
                onAccepted: TransferList.sendHandoff(home.deviceId, [selectedFile.toString()])
            }
            Connections {
                target: AppController
                enabled: home.visible
                function onFilePickerRequested() {
                    filePicker.open()
                }
                function onHandoffFilePickerRequested() {
                    handoffFilePicker.open()
                }
            }
            FolderDialog {
                id: folderPicker
                title: qsTr("Send a folder to %1").arg(home.name)
                onAccepted: TransferList.send(home.deviceId, [selectedFolder.toString()])
            }

            // ---- Waiting to send ----
            Card {
                id: sendQueueCard
                width: parent.width
                visible: sendQueueModel.count > 0
                opacity: visible ? 1 : 0
                Behavior on opacity {
                    enabled: !Theme.reduceMotion && !page.resizing
                    NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                }
                Behavior on implicitHeight {
                    enabled: !Theme.reduceMotion && !page.resizing
                    NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                }
                Column {
                    width: parent.width
                    spacing: 4
                    Item {
                        width: parent.width
                        height: Math.max(queueHeader.height, queueHeaderActions.height)
                        Txt {
                            id: queueHeader
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: page.sendQueueWaiting > 0
                                  ? qsTr("Waiting to send · %1").arg(page.sendQueueWaiting)
                                  : qsTr("Waiting to send")
                        }
                        Row {
                            id: queueHeaderActions
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 6
                            Button {
                                visible: TransferList.hasFailedOrExpiredQueue
                                variant: "text"
                                size: "sm"
                                text: qsTr("Clear")
                                onClicked: TransferList.clearFinishedQueue()
                            }
                            Button {
                                visible: TransferList.waitingCount > 0
                                variant: "text"
                                size: "sm"
                                text: qsTr("Cancel all")
                                onClicked: TransferList.cancelAllQueued("")
                            }
                        }
                    }
                    Column {
                        id: sendQueueList
                        width: parent.width
                        move: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { properties: "y"; duration: 180; easing.type: Easing.OutCubic }
                        }
                        add: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 180; easing.type: Easing.OutCubic }
                        }
                        Repeater {
                            model: sendQueueModel
                            delegate: Item {
                                id: queueRow
                                required property var model
                                width: sendQueueList.width
                                implicitHeight: queueItemCol.height + 18

                                readonly property string itemStatus: queueRow.model.status || "waiting"
                                readonly property bool isFailedOrExpired: itemStatus === "failed" || itemStatus === "expired"
                                readonly property real totalBytes: Number(queueRow.model.totalBytes || 0)
                                readonly property real progress: Number(queueRow.model.progress || 0)

                                Row {
                                    id: queueRowInner
                                    x: 4; y: 9
                                    width: parent.width - 8
                                    spacing: 12

                                    Rectangle {
                                        width: 32; height: 32
                                        anchors.verticalCenter: parent.verticalCenter
                                        radius: Theme.graphite ? Theme.radiusSm : 16
                                        color: Theme.graphite ? "transparent" : Theme.secondaryContainer
                                        border.width: Theme.graphite ? 1 : 0
                                        border.color: Theme.outlineVariant
                                        Icon {
                                            anchors.centerIn: parent
                                            width: 18; height: 18
                                            path: queueRow.model.icon === "folder" ? Icons.folder
                                                : queueRow.model.icon === "clipboard" ? Icons.clipboard
                                                : queueRow.model.icon === "link" ? Icons.globe
                                                : queueRow.model.icon === "task" ? Icons.bolt
                                                : queueRow.model.icon === "camera" ? Icons.camera
                                                : Icons.send
                                            color: Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
                                        }
                                    }

                                    Column {
                                        id: queueItemCol
                                        width: Math.max(80, queueRowInner.width - 32 - queueItemActions.width - queueRowInner.spacing * 2)
                                        anchors.verticalCenter: parent.verticalCenter
                                        spacing: 4
                                        Txt {
                                            width: parent.width
                                            role: "title"
                                            size: 14
                                            elide: (queueRow.model.kind === "link"
                                                    || queueRow.model.icon === "link"
                                                    || (queueRow.model.title || "").startsWith("http://")
                                                    || (queueRow.model.title || "").startsWith("https://"))
                                                ? Text.ElideMiddle
                                                : Text.ElideRight
                                            text: queueRow.model.title || ""
                                        }
                                        Rectangle {
                                            width: parent.width
                                            height: 4
                                            radius: 2
                                            visible: queueRow.itemStatus === "sending" && queueRow.totalBytes > 0
                                            color: Theme.surfaceContainerHighest
                                            Rectangle {
                                                height: parent.height
                                                radius: parent.radius
                                                width: parent.width * Math.max(0, Math.min(1, queueRow.progress))
                                                color: Theme.primary
                                                Behavior on width {
                                                    enabled: !Theme.reduceMotion
                                                    NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                                                }
                                            }
                                        }
                                        Txt {
                                            width: parent.width
                                            role: "bodySmall"
                                            muted: !queueRow.isFailedOrExpired
                                            color: queueRow.isFailedOrExpired ? Theme.error : Theme.surfaceContentVariant
                                            elide: Text.ElideRight
                                            text: {
                                                const devName = (queueRow.model.deviceName && queueRow.model.deviceName.length > 0)
                                                    ? queueRow.model.deviceName : home.name
                                                const sz = page.formatQueueSize(queueRow.totalBytes)
                                                if (queueRow.itemStatus === "sending") {
                                                    if (queueRow.totalBytes > 0) {
                                                        const pct = Math.round(Math.max(0, Math.min(1, queueRow.progress)) * 100)
                                                        return qsTr("Sending to %1 · %2%").arg(devName).arg(pct)
                                                    }
                                                    return qsTr("Sending to %1…").arg(devName)
                                                }
                                                if (queueRow.itemStatus === "failed") {
                                                    const reason = queueRow.model.reason || ""
                                                    return reason.length > 0
                                                        ? qsTr("Failed · %1").arg(reason)
                                                        : qsTr("Failed")
                                                }
                                                if (queueRow.itemStatus === "expired") {
                                                    const reason = queueRow.model.reason || ""
                                                    return reason.length > 0 ? reason : qsTr("Expired")
                                                }
                                                return sz.length > 0
                                                    ? qsTr("Waiting for %1 · %2").arg(devName).arg(sz)
                                                    : qsTr("Waiting for %1").arg(devName)
                                            }
                                        }
                                    }

                                    Row {
                                        id: queueItemActions
                                        anchors.verticalCenter: parent.verticalCenter
                                        spacing: 4
                                        IconButton {
                                            visible: queueRow.isFailedOrExpired
                                            iconPath: Icons.refresh
                                            label: qsTr("Retry")
                                            onClicked: TransferList.retryQueueItem(queueRow.model.queueId)
                                        }
                                        IconButton {
                                            iconPath: Icons.close
                                            label: queueRow.isFailedOrExpired ? qsTr("Dismiss") : qsTr("Cancel")
                                            onClicked: TransferList.cancelQueueItem(queueRow.model.queueId)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---- Transfers ----
            Card {
                width: parent.width
                visible: TransferList.count > 0
                Column {
                    width: parent.width
                    spacing: 4
                    Item {
                        width: parent.width
                        height: Math.max(transfersHeader.height, clearTransfers.height)
                        Txt {
                            id: transfersHeader
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: qsTr("Transfers")
                        }
                        Row {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 6
                            Button {
                                variant: "text"
                                size: "sm"
                                text: qsTr("Timeline")
                                onClicked: AppController.currentPage = "timeline"
                            }
                            Button {
                                id: clearTransfers
                                visible: TransferList.count > TransferList.active
                                variant: "text"
                                size: "sm"
                                text: qsTr("Clear finished")
                                onClicked: TransferList.clearFinished()
                            }
                        }
                    }
                    Column {
                        id: transferList
                        width: parent.width
                        move: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
                        }
                        add: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 200 }
                        }
                        Repeater {
                            model: TransferList
                            delegate: TransferItem { width: transferList.width }
                        }
                    }
                }
            }

            // ---- LocalSend nearby ----
            LocalSendCard {
                width: parent.width
                visible: AppController.localsendEnabled && page.localsendPeers.length > 0
            }

            // ---- Notifications ----
            Card {
                id: feed
                visible: !page.phoneLayout
                width: parent.width
                readonly property var state: home.feature("notifications.mirror")
                readonly property int count: NotificationList.count >= 0 ? NotificationList.countFor(home.deviceId) : 0

                Column {
                    width: parent.width
                    spacing: 8
                    Item {
                        width: parent.width
                        height: Math.max(header.height, clearAll.height)
                        Txt {
                            id: header
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            muted: true
                            text: feed.count > 0 ? qsTr("Notifications · %1").arg(feed.count) : qsTr("Notifications")
                        }
                        Button {
                            anchors.right: historyButton.visible
                                ? historyButton.left
                                : (clearAll.visible ? clearAll.left : parent.right)
                            anchors.rightMargin: historyButton.visible || clearAll.visible ? 8 : 0
                            anchors.verticalCenter: parent.verticalCenter
                            variant: AppController.notificationsPaused ? "tonal" : "text"
                            size: "sm"
                            iconPath: Icons.bell
                            text: AppController.notificationsPaused ? qsTr("Resume pop-ups") : qsTr("Pause pop-ups")
                            onClicked: AppController.toggleNotificationsPaused()
                        }
                        IconButton {
                            id: historyButton
                            anchors.right: clearAll.visible ? clearAll.left : parent.right
                            anchors.rightMargin: clearAll.visible ? 8 : 0
                            anchors.verticalCenter: parent.verticalCenter
                            visible: NotificationList.historyEnabled && NotificationHistory.count > 0
                            iconPath: Icons.history
                            text: qsTr("History")
                            label: qsTr("Notification history (last 24 hours)")
                            onClicked: {
                                historySearch.text = ""
                                historySheet.open()
                            }
                        }
                        Button {
                            id: clearAll
                            anchors.right: parent.right
                            visible: feed.count > 0
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.close
                            text: qsTr("Clear all")
                            onClicked: NotificationList.dismissAll(home.deviceId)
                        }
                    }
                    // Calm inline Undo bar after dismissing a notification or app group.
                    Rectangle {
                        id: undoBanner
                        width: parent.width
                        height: undoRow.height + 16
                        visible: NotificationList.undoLabel.length > 0
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: Theme.graphite ? 1 : 0
                        border.color: Theme.outlineVariant

                        Timer {
                            id: undoTimer
                            interval: 6000
                            running: NotificationList.undoLabel.length > 0
                            onTriggered: NotificationList.clearUndo()
                        }
                        Connections {
                            target: NotificationList
                            function onUndoLabelChanged() {
                                if (NotificationList.undoLabel.length > 0)
                                    undoTimer.restart()
                            }
                        }
                        Row {
                            id: undoRow
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 24
                            spacing: 8
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                width: Math.max(60, parent.width - undoBtn.width - closeUndoBtn.width - 16)
                                role: "bodySmall"
                                elide: Text.ElideRight
                                text: NotificationList.undoLabel
                            }
                            Button {
                                id: undoBtn
                                anchors.verticalCenter: parent.verticalCenter
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Undo")
                                onClicked: NotificationList.undoDismiss()
                            }
                            IconButton {
                                id: closeUndoBtn
                                anchors.verticalCenter: parent.verticalCenter
                                iconPath: Icons.close
                                label: qsTr("Dismiss")
                                onClicked: NotificationList.clearUndo()
                            }
                        }
                    }
                    // Ticks every minute so "5 min" ages without new events.
                    Timer {
                        id: feedClock
                        property int minutes: 0
                        interval: 60000
                        repeat: true
                        running: feed.count > 0
                        onTriggered: minutes++
                    }
                    // The page scrolls; this Column lays out every notification cleanly so
                    // images, replies, and group expansions always size the Card accurately.
                    Column {
                        id: list
                        width: parent.width
                        spacing: 4
                        move: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
                        }
                        add: Transition {
                            enabled: !Theme.reduceMotion && !page.resizing
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 200; easing.type: Easing.OutCubic }
                        }
                        Repeater {
                            model: NotificationList
                            delegate: NotificationItem {
                                width: list.width
                                clock: feedClock.minutes
                                visible: deviceId === home.deviceId
                                height: visible ? implicitHeight : 0
                                canOpenApp: home.feature("mirroring.app_windows").state === "available" && app.length > 0
                                onOpenAppRequested: Mirror.startApp(deviceId, app, appName.length > 0 ? appName : app)
                                onOptionsRequested: appSheet.openFor(app, appName)
                                onImageClicked: (url) => imagePreviewSheet.openFor(url, title.length > 0 ? title : appName)
                            }
                        }
                    }
                    // Empty: say why, and what turns it on.
                    Txt {
                        width: parent.width
                        visible: feed.count === 0 && !lockedHint.visible
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: feed.state.state === "available"
                              ? qsTr("No notifications")
                              : home.lastSeen > 0 || home.online
                                ? qsTr("Notifications unavailable")
                                : qsTr("Connect %1 to view notifications").arg(home.name)
                    }
                    LockChip { id: lockedHint; visible: feed.count === 0 && label.length > 0; feature: feed.state }
                    // Windows hides every app's notifications when they're
                    // turned off in Settings; say so instead of staying quiet.
                    Rectangle {
                        width: parent.width
                        height: toastsOff.height + 20
                        visible: !AppController.toastsEnabled && feed.state.state === "available"
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: Theme.graphite ? 1 : 0
                        border.color: Theme.outlineVariant
                        Row {
                            id: toastsOff
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 24
                            spacing: 12
                            Icon { anchors.verticalCenter: parent.verticalCenter; path: Icons.bell; color: Theme.surfaceContentVariant }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 32 - (turnOn.visible ? turnOn.width + 24 : 0)
                                role: "bodySmall"
                                wrapMode: Text.WordWrap
                                text: Qt.platform.os === "linux"
                                    ? qsTr("Desktop notifications are off. Turn them on in your system's notification settings.")
                                    : qsTr("Windows notifications are off.")
                            }
                            Button {
                                id: turnOn
                                anchors.verticalCenter: parent.verticalCenter
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Turn on")
                                visible: Qt.platform.os === "windows"
                                onClicked: Qt.openUrlExternally("ms-settings:notifications")
                            }
                        }
                    }
                }
            }
        }

        // ---- Details ----
        Column {
            width: home.sideWidth
            spacing: Theme.gutter

            // The call in progress on the phone.
            Binding { target: PhoneCall; property: "device"; value: home.deviceId; when: home.visible; restoreMode: Binding.RestoreNone }
            CallCard {
                width: home.sideWidth
                visible: PhoneCall.active && PhoneCall.device === home.deviceId
            }

            // Active webcam session on this phone.
            Card {
                id: webcamHomeCard
                width: parent.width
                readonly property var webcamFeature: home.feature("camera.webcam")
                readonly property bool isThisPhone: (Webcam.phase === "streaming" || Webcam.phase === "asking")
                                                    && Webcam.activeDevice === home.deviceId
                visible: isThisPhone
                readonly property bool previewWanted: isThisPhone && visible && home.visible
                    && webcamHomeCard.Window.window !== null && webcamHomeCard.Window.window.visible
                    && webcamHomeCard.Window.window.visibility !== Window.Minimized
                onPreviewWantedChanged: Webcam.setPreviewVisible("home", previewWanted)
                Component.onCompleted: Webcam.setPreviewVisible("home", previewWanted)
                Component.onDestruction: Webcam.setPreviewVisible("home", false)
                Column {
                    width: parent.width
                    spacing: 10
                    Item {
                        width: parent.width
                        height: Math.max(webcamHeaderLeft.height, webcamStopBtn.height)
                        Row {
                            id: webcamHeaderLeft
                            anchors.left: parent.left
                            anchors.right: webcamStopBtn.left
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                path: Icons.video
                                color: Theme.primary
                            }
                            Column {
                                width: parent.width - 28
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    text: qsTr("Webcam")
                                    role: "label"
                                    elide: Text.ElideRight
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: Webcam.statusText.length > 0
                                          ? Webcam.statusText
                                          : qsTr("Streaming from %1").arg(home.name)
                                }
                            }
                        }
                        Button {
                            id: webcamStopBtn
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.close
                            text: qsTr("Stop")
                            onClicked: Webcam.stop()
                        }
                    }
                    Rectangle {
                        width: parent.width
                        height: Math.round(width * 9 / 16)
                        radius: Theme.radiusMd
                        color: "#121318"
                        border.width: 1
                        border.color: Theme.outlineVariant
                        clip: true
                        VideoView {
                            anchors.fill: parent
                            stream: "webcam"
                        }
                    }
                    Flow {
                        width: parent.width
                        spacing: 6
                        Segmented {
                            options: [
                                { value: "back", label: qsTr("Back") },
                                { value: "front", label: qsTr("Front") }
                            ]
                            value: Webcam.camera
                            onPicked: (v) => Webcam.setCamera(v)
                        }
                        Segmented {
                            // 4K only when the phone's camera in use can stream it.
                            options: [
                                { value: "720", label: "720p" },
                                { value: "1080", label: "1080p" }
                            ].concat(Webcam.uhd ? [{ value: "2160", label: "4K" }] : [])
                            value: String(Webcam.height)
                            onPicked: (v) => Webcam.setResolution(Number(v))
                        }
                        Button {
                            variant: Webcam.mirror ? "tonal" : "outline"
                            size: "sm"
                            iconPath: Icons.mirror
                            text: qsTr("Mirror")
                            onClicked: Webcam.setMirrorImage(!Webcam.mirror)
                        }
                    }
                }
            }

            // Quick settings on the phone (toggles, plus expandable ringer/volume/brightness).
            Card {
                id: phoneControls
                width: parent.width
                visible: home.online && state !== null
                property bool expanded: false
                property bool confirmWifiOff: false
                property bool hotspotActive: false
                property var hoveredLockedFeature: null
                // The phone doesn't report its hotspot, so this is only ever what was asked.
                Connections {
                    target: AppController
                    function onPhoneToggleFailed(device, id) {
                        if (device === home.deviceId && id === "hotspot")
                            phoneControls.hotspotActive = !phoneControls.hotspotActive
                    }
                }
                readonly property var state: {
                    if (AppController.phoneTogglesRevision < 0)
                        return null
                    const raw = AppController.phoneToggles(home.deviceId)
                    if (!raw || raw.length === 0)
                        return null
                    try {
                        return JSON.parse(raw)
                    } catch (e) {
                        return null
                    }
                }
                readonly property bool hasFlashlight: state !== null
                    && state.flashlight !== undefined && state.flashlight !== null
                readonly property var dndFeature: home.feature("toggles.dnd")
                readonly property var flashlightFeature: home.feature("toggles.flashlight")
                readonly property var ringerFeature: home.feature("toggles.ringer")
                readonly property var volumeFeature: home.feature("toggles.volume")
                readonly property var brightnessFeature: home.feature("toggles.brightness")
                readonly property var wifiFeature: home.feature("toggles.wifi")
                readonly property var bluetoothFeature: home.feature("toggles.bluetooth")
                readonly property var webcamFeature: home.feature("camera.webcam")
                readonly property var activeLockFeature: {
                    if (hoveredLockedFeature && hoveredLockedFeature.state === "locked")
                        return hoveredLockedFeature
                    return ({})
                }
                onVisibleChanged: if (!visible) confirmWifiOff = false

                Shortcut {
                    sequence: "Esc"
                    enabled: page.active && phoneControls.confirmWifiOff
                    onActivated: phoneControls.confirmWifiOff = false
                }

                Column {
                    width: parent.width
                    spacing: 12

                    Item {
                        width: parent.width
                        height: 24
                        Txt {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Phone controls")
                            role: "label"
                            muted: true
                        }
                        Button {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "text"
                            size: "sm"
                            text: phoneControls.expanded ? qsTr("Less") : qsTr("Sound & display")
                            onClicked: phoneControls.expanded = !phoneControls.expanded
                        }
                    }

                    Row {
                        id: toggleRow
                        width: parent.width
                        spacing: 6
                        readonly property int buttonCount: phoneControls.hasFlashlight ? 5 : 4
                        readonly property real buttonWidth: (width - spacing * (buttonCount - 1)) / buttonCount

                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.moon
                            shortTitle: qsTr("DND")
                            label: qsTr("Do Not Disturb")
                            active: phoneControls.state ? Boolean(phoneControls.state.dnd) : false
                            feature: phoneControls.dndFeature
                            onLockedClicked: phoneControls.hoveredLockedFeature = (phoneControls.hoveredLockedFeature === feature ? ({}) : feature)
                            onClicked: AppController.setPhoneToggle(home.deviceId, "dnd", String(!active))
                        }
                        QuickToggleButton {
                            visible: phoneControls.hasFlashlight
                            width: toggleRow.buttonWidth
                            iconPath: Icons.flashlight
                            shortTitle: qsTr("Torch")
                            label: qsTr("Flashlight")
                            active: phoneControls.state !== null && phoneControls.state.flashlight === true
                            feature: phoneControls.flashlightFeature
                            onLockedClicked: phoneControls.hoveredLockedFeature = (phoneControls.hoveredLockedFeature === feature ? ({}) : feature)
                            onClicked: AppController.setPhoneToggle(home.deviceId, "flashlight", String(!active))
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.wifi
                            shortTitle: qsTr("Wi‑Fi")
                            label: qsTr("Wi‑Fi")
                            active: phoneControls.state ? Boolean(phoneControls.state.wifi) : false
                            feature: phoneControls.wifiFeature
                            onLockedClicked: phoneControls.hoveredLockedFeature = (phoneControls.hoveredLockedFeature === feature ? ({}) : feature)
                            onClicked: {
                                if (active)
                                    phoneControls.confirmWifiOff = true
                                else
                                    AppController.setPhoneToggle(home.deviceId, "wifi", "true")
                            }
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.bluetooth
                            shortTitle: qsTr("BT")
                            label: qsTr("Bluetooth")
                            active: phoneControls.state ? Boolean(phoneControls.state.bluetooth) : false
                            feature: phoneControls.bluetoothFeature
                            onLockedClicked: phoneControls.hoveredLockedFeature = (phoneControls.hoveredLockedFeature === feature ? ({}) : feature)
                            onClicked: AppController.setPhoneToggle(home.deviceId, "bluetooth", String(!active))
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.globe
                            shortTitle: qsTr("Hotspot")
                            label: qsTr("Instant Hotspot")
                            active: phoneControls.hotspotActive
                            feature: phoneControls.wifiFeature
                            onLockedClicked: phoneControls.hoveredLockedFeature = (phoneControls.hoveredLockedFeature === feature ? ({}) : feature)
                            onClicked: {
                                phoneControls.hotspotActive = !phoneControls.hotspotActive
                                AppController.setPhoneToggle(home.deviceId, "hotspot", String(phoneControls.hotspotActive))
                            }
                        }
                    }

                    SwitchBar {
                        width: parent.width
                        iconPath: Icons.speaker
                        title: qsTr("Phone as speaker")
                        caption: checked ? qsTr("Streaming PC audio") : ""
                        checked: Preferences.streamPcAudioToPhone
                        onToggled: (on) => Preferences.streamPcAudioToPhone = on
                    }

                    // Play on phone: the video playing on this PC, on the phone.
                    // Greyed out until a video plays.
                    SwitchBar {
                        readonly property string watch: MediaList.playOnPhoneState
                        width: parent.width
                        iconPath: Icons.video
                        title: qsTr("Play on phone")
                        enabled: watch !== "off" || MediaList.canPlayOnPhone
                        checked: watch !== "off"
                        caption: watch === "watching" ? qsTr("Watching on %1").arg(MediaList.playOnPhoneName)
                            : watch === "asking" ? qsTr("Tap the notification on %1").arg(MediaList.playOnPhoneName)
                            : MediaList.canPlayOnPhone ? ""
                            : qsTr("Play a video on this PC first")
                        onToggled: (on) => MediaList.setPlayOnPhone(on)
                    }

                    // The phone's sound out of this PC's speakers, without a mirror window.
                    SwitchBar {
                        readonly property string sound: MediaList.revision >= 0 ? MediaList.phoneSoundState(home.deviceId) : "off"
                        width: parent.width
                        visible: sound !== "off" || (home.online && home.feature("mirroring.sound").state === "available")
                        iconPath: Icons.music
                        title: qsTr("Phone sound on this PC")
                        checked: sound !== "off"
                        caption: sound === "playing" ? qsTr("Playing %1's sound").arg(home.name)
                            : sound === "asking" ? qsTr("Allow it on %1").arg(home.name)
                            : ""
                        onToggled: (on) => MediaList.setPhoneSound(home.deviceId, on)
                    }

                    LockChip {
                        visible: label.length > 0
                        feature: phoneControls.activeLockFeature
                        maxWidth: parent.width
                    }

                    // Confirm before turning Wi-Fi off, since that can sever the link.
                    Rectangle {
                        width: parent.width
                        height: wifiConfirmCol.height + 20
                        visible: phoneControls.confirmWifiOff && phoneControls.state !== null && Boolean(phoneControls.state.wifi)
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: 1
                        border.color: Theme.outlineVariant
                        Column {
                            id: wifiConfirmCol
                            x: 12
                            y: 10
                            width: parent.width - 24
                            spacing: 8
                            Txt {
                                width: parent.width
                                role: "bodySmall"
                                wrapMode: Text.WordWrap
                                text: qsTr("Turning off Wi‑Fi may disconnect %1.")
                                      .arg(home.name)
                            }
                            Row {
                                anchors.right: parent.right
                                spacing: 8
                                Button {
                                    variant: "text"
                                    size: "sm"
                                    text: qsTr("Cancel")
                                    onClicked: phoneControls.confirmWifiOff = false
                                }
                                Button {
                                    variant: "tonal"
                                    size: "sm"
                                    text: qsTr("Turn off")
                                    onClicked: {
                                        phoneControls.confirmWifiOff = false
                                        AppController.setPhoneToggle(home.deviceId, "wifi", "false")
                                    }
                                }
                            }
                        }
                    }

                    Column {
                        width: parent.width
                        spacing: 12
                        visible: phoneControls.expanded

                        // Ringer mode segmented control.
                        Column {
                            width: parent.width
                            spacing: 6
                            Segmented {
                                width: parent.width
                                enabled: phoneControls.ringerFeature.state === "available"
                                         || phoneControls.ringerFeature.state === "partial"
                                opacity: enabled ? 1 : 0.5
                                options: [
                                    { value: "ring", label: qsTr("Ring") },
                                    { value: "vibrate", label: qsTr("Vibrate") },
                                    { value: "silent", label: qsTr("Silent") }
                                ]
                                value: phoneControls.state ? phoneControls.state.ringer : "ring"
                                onPicked: (mode) => AppController.setPhoneToggle(home.deviceId, "ringer", mode)
                            }
                            LockChip {
                                visible: phoneControls.ringerFeature.state === "locked" && label.length > 0
                                feature: phoneControls.ringerFeature
                                maxWidth: parent.width
                            }
                        }

                        // Media volume slider.
                        ToggleSlider {
                            width: parent.width
                            deviceId: home.deviceId
                            toggleId: "volume"
                            title: qsTr("Volume")
                            iconPath: shownValue === 0 ? Icons.soundOff : Icons.speaker
                            phoneValue: phoneControls.state ? phoneControls.state.volume : 0
                            feature: phoneControls.volumeFeature
                        }

                        // Screen brightness slider.
                        ToggleSlider {
                            width: parent.width
                            deviceId: home.deviceId
                            toggleId: "brightness"
                            title: qsTr("Brightness")
                            iconPath: Icons.brightness
                            phoneValue: phoneControls.state ? phoneControls.state.brightness : 0
                            feature: phoneControls.brightnessFeature
                        }
                    }
                }
            }

            // Camera & remote typing above Timeline.
            Row {
                id: sideCameraGrid
                width: parent.width
                spacing: 10
                readonly property real tileWidth: (width - spacing) / 2

                ActionTile {
                    width: sideCameraGrid.tileWidth
                    deviceOnline: home.online
                    queueableWhenOffline: true
                    accented: true
                    title: qsTr("Continuity Camera")
                    subtitle: AppController.continuityCameraBusy
                              ? AppController.continuityCameraStatus
                              : ""
                    iconPath: Icons.camera
                    feature: home.feature("camera.continuity")
                    active: AppController.continuityCameraBusy
                    onClicked: AppController.startContinuityCamera("photo")
                    sideIcon: AppController.continuityCameraBusy ? Icons.close : Icons.scan
                    sideLabel: AppController.continuityCameraBusy
                               ? qsTr("Cancel")
                               : qsTr("Scan document")
                    onSideClicked: {
                        if (AppController.continuityCameraBusy)
                            AppController.cancelContinuityCamera()
                        else
                            AppController.startContinuityCamera("scan")
                    }
                }

                ActionTile {
                    width: sideCameraGrid.tileWidth
                    deviceOnline: home.online
                    playableWhenAddonMissing: true
                    title: qsTr("Phone as webcam")
                    subtitle: (Webcam.phase === "streaming" || Webcam.phase === "asking")
                              && Webcam.activeDevice === home.deviceId
                              ? (Webcam.statusText.length > 0 ? Webcam.statusText : qsTr("Streaming"))
                              : ""
                    iconPath: Icons.video
                    feature: home.feature("camera.webcam")
                    active: (Webcam.phase === "streaming" || Webcam.phase === "asking")
                            && Webcam.activeDevice === home.deviceId
                    onClicked: {
                        if ((Webcam.phase === "streaming" || Webcam.phase === "asking")
                            && Webcam.activeDevice === home.deviceId)
                            Webcam.stop()
                        else
                            Webcam.start(home.deviceId)
                    }
                }
            }

            // Type on phone (compact card below camera tiles; expands to capture live keystrokes).
            Card {
                id: typeOnPhoneCard
                width: parent.width
                readonly property var remoteInputFeature: home.feature("mirroring.control")
                readonly property bool available: remoteInputFeature.state === "available"
                readonly property bool canStart: home.online
                readonly property bool activeHere: Mirror.keyboardDevice === home.deviceId
                property string lastSent: ""
                // What's been typed lately, shown as it would read rather than key by key.
                property string typed: ""

                function sentText(text) {
                    typed = (typed + text).slice(-40)
                    lastSent = typed
                }
                function sentKey(name) {
                    if (name === "backspace" && typed.length > 0) {
                        const chars = Array.from(typed)
                        chars.pop()
                        typed = chars.join("")
                        if (typed.length > 0) {
                            lastSent = typed
                            return
                        }
                    }
                    typed = ""
                    lastSent = name
                }

                onActiveHereChanged: {
                    if (!activeHere) {
                        lastSent = ""
                        typed = ""
                    }
                }

                Column {
                    width: parent.width
                    spacing: 10
                    Item {
                        width: parent.width
                        height: Math.max(typeHeaderLeft.height, typeToggleBtn.height)
                        Row {
                            id: typeHeaderLeft
                            anchors.left: parent.left
                            anchors.right: typeToggleBtn.left
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 10
                            Rectangle {
                                width: 32; height: 32
                                anchors.verticalCenter: parent.verticalCenter
                                radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
                                color: typeOnPhoneCard.activeHere
                                    ? Theme.primary
                                    : (Theme.graphite ? "transparent" : Theme.secondaryContainer)
                                border.width: Theme.graphite ? 1 : 0
                                border.color: Theme.outlineVariant
                                opacity: typeOnPhoneCard.canStart ? 1 : 0.55
                                Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
                                Icon {
                                    anchors.centerIn: parent
                                    width: 17; height: 17
                                    path: Icons.keyboard
                                    color: typeOnPhoneCard.activeHere ? Theme.primaryContent : Theme.secondaryContainerContent
                                }
                            }
                            Column {
                                width: parent.width - 42
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    role: "title"
                                    size: 13
                                    elide: Text.ElideRight
                                    text: qsTr("Type on phone")
                                    opacity: typeOnPhoneCard.canStart ? 1 : 0.6
                                }
                                Txt {
                                    visible: typeOnPhoneCard.activeHere || !typeOnPhoneCard.canStart
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: typeOnPhoneCard.activeHere
                                        ? (typeOnPhoneCard.lastSent.length > 0
                                            ? qsTr("Sent: %1").arg(typeOnPhoneCard.lastSent)
                                            : qsTr("Tap a text box on the phone, then type"))
                                        : qsTr("When connected")
                                }
                            }
                        }
                        Button {
                            id: typeToggleBtn
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            visible: typeOnPhoneCard.canStart
                            variant: typeOnPhoneCard.activeHere ? "fill" : "tonal"
                            size: "sm"
                            text: typeOnPhoneCard.activeHere ? qsTr("Stop") : qsTr("Start")
                            onClicked: {
                                if (typeOnPhoneCard.activeHere) {
                                    Mirror.stopKeyboard()
                                } else {
                                    Mirror.toggleKeyboard(home.deviceId)
                                    remoteKeyCapture.forceActiveFocus()
                                }
                            }
                        }
                    }

                    LockChip {
                        id: typeLock
                        visible: !typeOnPhoneCard.available && label.length > 0
                                 && (home.online || typeOnPhoneCard.remoteInputFeature.action !== "updateApp")
                        feature: typeOnPhoneCard.remoteInputFeature
                        maxWidth: parent.width
                    }

                    Rectangle {
                        id: remoteKeyCapture
                        width: parent.width
                        height: typeOnPhoneCard.activeHere ? 54 : 0
                        visible: height > 0
                        clip: true
                        Behavior on height {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
                        }
                        radius: Theme.radiusMd
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
                        border.width: activeFocus ? 2 : 1
                        border.color: activeFocus ? Theme.primary : Theme.outlineVariant
                        activeFocusOnTab: typeOnPhoneCard.activeHere
                        // Screen readers can find and focus the box the keys go through.
                        Accessible.role: Accessible.EditableText
                        Accessible.name: qsTr("Type on phone")
                        Accessible.focusable: true

                        onVisibleChanged: {
                            if (visible) forceActiveFocus()
                        }

                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Escape) {
                                Mirror.stopKeyboard()
                                event.accepted = true
                                return
                            }
                            if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V) {
                                Mirror.keyboardPaste(home.deviceId)
                                typeOnPhoneCard.lastSent = qsTr("Clipboard")
                                event.accepted = true
                                return
                            }
                            let keyName = ""
                            switch (event.key) {
                            case Qt.Key_Return:
                            case Qt.Key_Enter: keyName = "enter"; break
                            case Qt.Key_Backspace: keyName = "backspace"; break
                            case Qt.Key_Delete: keyName = "delete"; break
                            case Qt.Key_Left: keyName = "left"; break
                            case Qt.Key_Right: keyName = "right"; break
                            case Qt.Key_Up: keyName = "up"; break
                            case Qt.Key_Down: keyName = "down"; break
                            case Qt.Key_Tab: keyName = "tab"; break
                            default: break
                            }
                            if (keyName.length > 0) {
                                Mirror.keyboardPress(home.deviceId, keyName)
                                typeOnPhoneCard.sentKey(keyName)
                                event.accepted = true
                                return
                            }
                            const edit = (event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.ShiftModifier | Qt.MetaModifier))
                                === Qt.ControlModifier
                                ? ({ [Qt.Key_C]: "copy", [Qt.Key_X]: "cut", [Qt.Key_A]: "select_all" })[event.key] : undefined
                            if (edit !== undefined) {
                                Mirror.keyboardPress(home.deviceId, edit)
                                event.accepted = true
                                return
                            }
                            if (event.text && event.text.length > 0 && event.text.charCodeAt(0) >= 32
                                    && (!(event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))
                                        // AltGr (@ on a French keyboard, say) arrives as Ctrl+Alt on Windows.
                                        // A plain letter, digit or space with Ctrl+Alt is a shortcut:
                                        // Qt gives Ctrl+Alt+2 the text "2" even without AltGr.
                                        || ((event.modifiers & Qt.ControlModifier) && (event.modifiers & Qt.AltModifier)
                                            && !/^[A-Za-z0-9 ]$/.test(event.text)))) {
                                Mirror.keyboardText(home.deviceId, event.text)
                                typeOnPhoneCard.sentText(event.text)
                                event.accepted = true
                            }
                        }

                        Row {
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 24
                            spacing: 8
                            Column {
                                width: parent.width - pasteBtn.width - 8
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2
                                Txt {
                                    width: parent.width
                                    role: "bodySmall"
                                    weight: 600
                                    elide: Text.ElideRight
                                    color: remoteKeyCapture.activeFocus ? Theme.primary : Theme.surfaceContent
                                    text: remoteKeyCapture.activeFocus
                                        ? qsTr("Type here: keys go to %1").arg(home.name)
                                        : qsTr("Click to focus keyboard")
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: qsTr("Ctrl+V pastes · Esc stops")
                                }
                            }
                            Button {
                                id: pasteBtn
                                anchors.verticalCenter: parent.verticalCenter
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.clipboard
                                text: qsTr("Paste")
                                onClicked: {
                                    Mirror.keyboardPaste(home.deviceId)
                                    typeOnPhoneCard.lastSent = qsTr("Clipboard")
                                    remoteKeyCapture.forceActiveFocus()
                                }
                            }
                        }
                        TapHandler {
                            onTapped: remoteKeyCapture.forceActiveFocus()
                        }
                    }
                }
            }

            // Recent timeline items across devices (placed below Continuity Camera, Phone as webcam, and Type on phone).
            Card {
                id: timelineHomeCard
                width: parent.width
                readonly property var previewItems: {
                    try {
                        return AppController.timelinePreview.length > 0
                            ? JSON.parse(AppController.timelinePreview) : []
                    } catch (e) {
                        return []
                    }
                }
                Column {
                    width: parent.width
                    spacing: 8
                    Item {
                        width: parent.width
                        height: 24
                        Txt {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Timeline")
                            role: "label"
                            muted: true
                        }
                        Button {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "text"
                            size: "sm"
                            text: qsTr("View all")
                            onClicked: AppController.currentPage = "timeline"
                        }
                    }
                    Txt {
                        width: parent.width
                        visible: timelineHomeCard.previewItems.length === 0
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: qsTr("No recent activity")
                    }
                    Repeater {
                        model: timelineHomeCard.previewItems
                        delegate: Item {
                            id: previewRow
                            required property var modelData
                            width: parent.width
                            height: 36
                            activeFocusOnTab: true
                            Accessible.role: Accessible.Button
                            Accessible.name: previewRow.modelData.title || ""
                            Accessible.description: previewRow.modelData.subtitle || ""
                            Accessible.onPressAction: AppController.currentPage = "timeline"
                            Keys.onPressed: (event) => {
                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                    AppController.currentPage = "timeline"
                                    event.accepted = true
                                }
                            }

                            Rectangle {
                                anchors.fill: parent
                                anchors.margins: -4
                                radius: Theme.radiusSm
                                color: previewHover.hovered ? Theme.surfaceContainerHigh : "transparent"
                                border.width: Theme.focusVisible(previewRow) ? 2 : 0
                                border.color: Theme.primary
                            }

                            Icon {
                                id: previewIcon
                                anchors.left: parent.left
                                anchors.verticalCenter: parent.verticalCenter
                                width: 16; height: 16
                                path: modelData.kind === "file" ? Icons.folder
                                    : modelData.kind === "clip" ? Icons.clipboard
                                    : modelData.kind === "link" ? Icons.globe
                                    : modelData.kind === "photo" ? Icons.photo
                                    : modelData.kind === "recording" ? Icons.mic
                                    : Icons.mirror
                                color: Theme.primary
                            }
                            Column {
                                anchors.left: previewIcon.right
                                anchors.leftMargin: 10
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    role: "bodySmall"
                                    weight: 600
                                    elide: Text.ElideRight
                                    text: previewRow.modelData.title || ""
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: previewRow.modelData.subtitle || ""
                                }
                            }
                            HoverHandler { id: previewHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: AppController.currentPage = "timeline" }
                        }
                    }
                }
            }
        }
    }

    // What one app's notifications do on this PC.
    Sheet {
        id: appSheet
        property string app
        property string appName
        property string rule: "show"
        cardWidth: 420
        function openFor(app, name) {
            appSheet.app = app
            appSheet.appName = name
            appSheet.rule = NotificationList.appRule(app)
            open()
        }
        Column {
            width: parent.width
            spacing: 16
            Txt { width: parent.width; text: qsTr("%1 notifications").arg(appSheet.appName); role: "headline"; wrapMode: Text.WordWrap }
            Segmented {
                options: [
                    { value: "show", label: qsTr("Show") },
                    { value: "quiet", label: qsTr("No pop-ups") },
                    { value: "hidden", label: qsTr("Hide") }
                ]
                value: appSheet.rule
                onPicked: (value) => {
                    appSheet.rule = value
                    NotificationList.setAppRule(appSheet.app, value)
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: appSheet.rule === "hidden"
                      ? qsTr("Hidden on this PC.")
                      : appSheet.rule === "quiet"
                        ? qsTr("Shown in feed without pop-ups.")
                        : Qt.platform.os === "linux"
                          ? qsTr("Shown in feed and as desktop pop-ups.")
                          : qsTr("Shown in feed and as Windows pop-ups.")
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: appSheet.close() }
            }
        }
    }

    // Set a custom PC-side nickname for a phone.
    Sheet {
        id: nicknameSheet
        property string targetDeviceId: ""
        property string currentName: ""
        property string rawName: ""
        cardWidth: 420
        function openFor(devId, name, raw) {
            targetDeviceId = devId
            currentName = name
            rawName = raw
            nicknameInput.text = (name !== raw) ? name : ""
            open()
            nicknameInput.forceActiveFocus()
        }
        function saveNickname() {
            Preferences.setNickname(targetDeviceId, nicknameInput.text.trim())
            nicknameSheet.close()
        }
        Column {
            width: parent.width
            spacing: 16
            Txt {
                width: parent.width
                text: qsTr("Phone nickname")
                role: "headline"
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: qsTr("Set a custom name for this phone on your PC. Leave blank to use \"%1\".").arg(nicknameSheet.rawName)
            }
            Rectangle {
                width: parent.width
                height: 40
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                border.width: nicknameInput.activeFocus ? 2 : 1
                border.color: nicknameInput.activeFocus ? Theme.primary : Theme.outlineVariant

                TextInput {
                    id: nicknameInput
                    anchors.fill: parent
                    anchors.margins: 10
                    color: Theme.surfaceContent
                    font.pixelSize: 14
                    selectByMouse: true
                    clip: true
                    onAccepted: nicknameSheet.saveNickname()
                }
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                Button {
                    text: qsTr("Cancel")
                    variant: "outline"
                    onClicked: nicknameSheet.close()
                }
                Button {
                    text: qsTr("Save")
                    onClicked: nicknameSheet.saveNickname()
                }
            }
        }
    }

    // Notifications from the last day that are gone from the phone.
    AppsSheet {
        id: appsSheet
    }

    Sheet {
        id: historySheet
        cardWidth: 560
        Column {
            width: parent.width
            spacing: 12
            Item {
                width: parent.width
                height: Math.max(historyTitle.height, clearHistory.height)
                Txt { id: historyTitle; anchors.verticalCenter: parent.verticalCenter; text: qsTr("Notification history"); role: "headline" }
                Button {
                    id: clearHistory
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    visible: NotificationHistory.count > 0
                    variant: "text"
                    size: "sm"
                    text: qsTr("Clear history")
                    onClicked: NotificationList.clearHistory()
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: NotificationHistory.count > 0
                      ? qsTr("Dismissed notifications from the last 24 hours.")
                      : qsTr("No recent notifications.")
            }
            // Search: by app, title or text, ignoring case.
            Rectangle {
                width: parent.width
                height: 40
                visible: NotificationHistory.count > 0
                radius: Theme.graphite ? Theme.radiusSm : height / 2
                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                border.width: historySearch.activeFocus ? 2 : 1
                border.color: historySearch.activeFocus ? Theme.primary : Theme.outlineVariant
                Icon {
                    id: searchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18; height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: historySearch
                    readonly property string query: text.trim().toLocaleLowerCase()
                    function matches(fields) {
                        return query.length === 0 || fields.join(" ").toLocaleLowerCase().indexOf(query) >= 0
                    }
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 10
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    clip: true
                    Keys.onEscapePressed: (event) => {
                        // Clears first; a second Escape closes the sheet.
                        event.accepted = text.length > 0
                        text = ""
                    }
                    Accessible.name: qsTr("Search notification history")
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: historySearch.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Search notification history")
                    }
                }
            }
            Flickable {
                width: parent.width
                height: Math.min(historyColumn.height, page.height - 260)
                contentHeight: historyColumn.height
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: historyColumn
                    width: parent.width
                    Repeater {
                        model: historySheet.opened ? NotificationHistory : null
                        delegate: HistoryItem {
                            required property string deviceId
                            required property string app
                            width: historyColumn.width
                            visible: historySearch.matches([appName, title, text, sub])
                            canOpenApp: (AppController.capsRevision >= 0
                                && AppController.featureState(deviceId, "mirroring.app_windows").state === "available")
                                && app.length > 0
                            onOpenAppRequested: {
                                historySheet.close()
                                Mirror.startApp(deviceId, app, appName.length > 0 ? appName : app)
                            }
                        }
                    }
                }
            }
            Txt {
                width: parent.width
                visible: historySearch.query.length > 0 && historyColumn.height === 0
                horizontalAlignment: Text.AlignHCenter
                role: "body"
                muted: true
                text: qsTr("Nothing matches “%1”.").arg(historySearch.text.trim())
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: historySheet.close() }
            }
        }
    }

    Sheet {
        id: clipboardSheet
        cardWidth: 560
        readonly property var items: {
            if (!clipboardSheet.opened)
                return []
            try {
                return JSON.parse(AppController.clipboardHistory)
            } catch (e) {
                return []
            }
        }
        Column {
            width: parent.width
            spacing: 12
            Item {
                width: parent.width
                height: Math.max(clipboardTitle.height, clearClipboard.height)
                Txt { id: clipboardTitle; anchors.verticalCenter: parent.verticalCenter; text: qsTr("Clipboard history"); role: "headline" }
                Button {
                    id: clearClipboard
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    visible: clipboardSheet.items.length > 0
                    variant: "text"
                    size: "sm"
                    text: qsTr("Clear history")
                    onClicked: AppController.clearClipboardHistory()
                }
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: clipboardSheet.items.length > 0
                      ? qsTr("Last 50 shared clips.")
                      : qsTr("No shared clips yet.")
            }
            Rectangle {
                width: parent.width
                height: 40
                visible: clipboardSheet.items.length > 0
                radius: Theme.graphite ? Theme.radiusSm : height / 2
                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                border.width: clipboardSearch.activeFocus ? 2 : 1
                border.color: clipboardSearch.activeFocus ? Theme.primary : Theme.outlineVariant
                Icon {
                    id: clipSearchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18; height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: clipboardSearch
                    readonly property string query: text.trim().toLocaleLowerCase()
                    function matches(fields) {
                        return query.length === 0 || fields.join(" ").toLocaleLowerCase().indexOf(query) >= 0
                    }
                    anchors.left: clipSearchIcon.right
                    anchors.leftMargin: 10
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    clip: true
                    Keys.onEscapePressed: (event) => {
                        event.accepted = text.length > 0
                        text = ""
                    }
                    Accessible.name: qsTr("Search clipboard history")
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: clipboardSearch.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Search clipboard history")
                    }
                }
            }
            Flickable {
                width: parent.width
                height: Math.min(clipboardColumn.height, page.height - 260)
                contentHeight: clipboardColumn.height
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: clipboardColumn
                    width: parent.width
                    Repeater {
                        model: clipboardSheet.items
                        delegate: Item {
                            id: clipRow
                            required property var modelData
                            required property int index
                            readonly property string sourceLabel: modelData.incoming
                                ? (modelData.deviceName ? qsTr("From %1").arg(modelData.deviceName) : qsTr("From phone"))
                                : (modelData.deviceName ? qsTr("Sent to %1").arg(modelData.deviceName) : qsTr("From this PC"))
                            readonly property string timeLabel: page.relativeTime(modelData.timestamp || 0)
                            width: clipboardColumn.width
                            visible: clipboardSearch.matches([
                                modelData.text || "",
                                sourceLabel,
                                modelData.kind === "image" ? qsTr("Image") : ""
                            ])
                            height: visible ? Math.max(clipIconBox.height, clipContent.height, clipActions.height) + 20 : 0

                            Divider {
                                width: parent.width
                                visible: clipRow.index > 0
                            }

                            Item {
                                id: clipIconBox
                                x: 4; y: 12
                                width: 28; height: 28
                                Icon {
                                    anchors.centerIn: parent
                                    width: 18; height: 18
                                    path: clipRow.modelData.pinned
                                          ? Icons.pin
                                          : (clipRow.modelData.incoming ? Icons.phone : Icons.laptop)
                                    color: clipRow.modelData.pinned ? Theme.primary : Theme.surfaceContentVariant
                                }
                            }

                            Column {
                                id: clipContent
                                anchors.left: clipIconBox.right
                                anchors.leftMargin: 12
                                anchors.right: clipActions.left
                                anchors.rightMargin: 8
                                y: 10
                                spacing: 4
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    text: [
                                        clipRow.sourceLabel,
                                        clipRow.timeLabel,
                                        clipRow.modelData.pinned ? qsTr("Pinned") : ""
                                    ].filter(s => s.length > 0).join(" · ")
                                    elide: Text.ElideRight
                                }
                                Txt {
                                    width: parent.width
                                    visible: clipRow.modelData.kind === "text" && Boolean(clipRow.modelData.text)
                                    role: "bodySmall"
                                    text: clipRow.modelData.text || ""
                                    wrapMode: Text.Wrap
                                    maximumLineCount: 3
                                    elide: Text.ElideRight
                                }
                                Rectangle {
                                    visible: clipRow.modelData.kind === "image"
                                    width: Math.min(parent.width, 180)
                                    height: 88
                                    radius: Theme.radiusSm
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant
                                    clip: true
                                    Image {
                                        anchors.fill: parent
                                        anchors.margins: 4
                                        source: clipRow.modelData.imageDataUrl || ""
                                        fillMode: Image.PreserveAspectFit
                                        asynchronous: true
                                        mipmap: true
                                    }
                                }
                            }

                            Row {
                                id: clipActions
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 4
                                IconButton {
                                    visible: page.currentDeviceId.length > 0
                                             && clipRow.modelData.kind === "text"
                                             && Boolean(clipRow.modelData.text)
                                             && AppController.isHandoffText(clipRow.modelData.text)
                                    iconPath: Icons.globe
                                    label: qsTr("Open on phone")
                                    onClicked: {
                                        clipboardSheet.close()
                                        AppController.openLinkOnPhone(page.currentDeviceId, clipRow.modelData.text)
                                    }
                                }
                                IconButton {
                                    visible: page.currentDeviceId.length > 0
                                    iconPath: Icons.send
                                    label: qsTr("Send to phone")
                                    onClicked: {
                                        clipboardSheet.close()
                                        AppController.sendClipboardHistory(page.currentDeviceId, clipRow.modelData.id)
                                    }
                                }
                                IconButton {
                                    iconPath: Icons.copy
                                    label: qsTr("Copy")
                                    onClicked: AppController.copyClipboardHistory(clipRow.modelData.id)
                                }
                                IconButton {
                                    iconPath: Icons.pin
                                    iconColor: clipRow.modelData.pinned ? Theme.primary : Theme.surfaceContentVariant
                                    label: clipRow.modelData.pinned ? qsTr("Unpin") : qsTr("Pin")
                                    onClicked: AppController.pinClipboardHistory(clipRow.modelData.id, !clipRow.modelData.pinned)
                                }
                                IconButton {
                                    iconPath: Icons.trash
                                    label: qsTr("Delete")
                                    onClicked: AppController.deleteClipboardHistory(clipRow.modelData.id)
                                }
                            }
                        }
                    }
                }
            }
            Txt {
                width: parent.width
                visible: clipboardSearch.query.length > 0 && clipboardColumn.height === 0
                horizontalAlignment: Text.AlignHCenter
                role: "body"
                muted: true
                text: qsTr("Nothing matches “%1”.").arg(clipboardSearch.text.trim())
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: clipboardSheet.close() }
            }
        }
    }

    // A quick action. Unavailable actions are dimmed and say what unlocks them.
    component ActionTile: Rectangle {
        id: tile
        property string title
        property string subtitle
        property string iconPath
        property var feature: ({})
        property bool active: false
        property bool accented: false
        property bool deviceOnline: true
        // A second, smaller action in the corner (e.g. "Send a folder").
        property string sideIcon
        property string sideLabel
        property bool sideAlwaysAvailable: false
        property bool playableWhenAddonMissing: false
        property bool queueableWhenOffline: false
        signal clicked
        signal sideClicked

        readonly property bool available: (feature.state === "available"
                                           || feature.state === "partial"
                                           || (playableWhenAddonMissing && feature.state === "locked" && feature.action === "enableAddon")
                                           || (queueableWhenOffline && (!tile.deviceOnline || feature.state === "unknown") && feature.state !== "locked" && feature.state !== "unsupported"))

        height: (tile.subtitle.length > 0 || !tile.available) ? 96 : 82
        activeFocusOnTab: available
        radius: Theme.radiusLg
        color: tile.active
            ? Theme.primary
            : (tile.accented && !Theme.graphite
                ? Qt.tint(Theme.tileColor, Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.14 : 0.10))
                : Theme.tileColor)
        border.width: Theme.focusVisible(tile) ? 2 : ((tile.accented || Theme.graphite) && !Theme.glass ? 1 : 0)
        border.color: Theme.focusVisible(tile)
            ? Theme.primary
            : (tile.accented && !Theme.graphite
                ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.36 : 0.30)
                : (Theme.ambientBlurActive ? Theme.cardBorder : Theme.outlineVariant))
        scale: tap.pressed && available ? Theme.pressScale : 1
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 170; easing.type: Easing.OutBack } }
        GlassRim { radius: tile.radius; visible: Theme.glass && !tile.active && !Theme.focusVisible(tile) }

        Accessible.role: Accessible.Button
        Accessible.name: title
        Accessible.description: available ? subtitle : lock.label
        Accessible.onPressAction: if (tile.available) tile.clicked()
        Keys.onPressed: (event) => {
            if (tile.available && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                tile.clicked()
                event.accepted = true
            }
        }

        readonly property color ink: tile.active ? Theme.primaryContent : Theme.surfaceContent
        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: tile.ink
            opacity: tap.pressed && tile.available ? 0.08 : 0
        }
        Column {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8
            Rectangle {
                width: 32; height: 32
                radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
                color: tile.active
                    ? Qt.rgba(1, 1, 1, 0.18)
                    : (Theme.graphite ? "transparent" : (tile.accented ? Theme.primaryContainer : Theme.secondaryContainer))
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                opacity: tile.available ? 1 : 0.55
                Icon {
                    anchors.centerIn: parent
                    width: 17; height: 17
                    path: tile.iconPath
                    color: tile.active
                        ? Theme.primaryContent
                        : (tile.accented && !Theme.graphite ? Theme.primaryContainerContent : Theme.secondaryContainerContent)
                }
            }
            Column {
                width: parent.width
                spacing: 2
                Txt { width: parent.width; text: tile.title; role: "title"; size: 13; elide: Text.ElideRight; color: tile.ink; opacity: tile.available ? 1 : 0.6 }
                Txt { width: parent.width; visible: tile.available && tile.subtitle.length > 0; text: tile.subtitle; role: "caption"; elide: Text.ElideRight; color: tile.ink; opacity: tile.active ? 0.82 : 0.78 }
                // Capabilities unknown until the device has connected once.
                Txt {
                    width: parent.width
                    visible: !tile.available && !lock.visible
                    text: qsTr("When connected")
                    role: "caption"
                    muted: true
                }
                LockChip { id: lock; visible: !tile.available && label.length > 0 && (tile.deviceOnline || tile.feature.action !== "updateApp"); feature: tile.feature }
            }
        }
        HoverHandler { id: hover; cursorShape: tile.available ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { id: tap; enabled: tile.available; onTapped: tile.clicked() }
        IconButton {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 6
            visible: tile.sideIcon.length > 0 && (tile.available || tile.sideAlwaysAvailable)
            iconPath: tile.sideIcon
            label: tile.sideLabel
            iconColor: tile.ink
            onClicked: tile.sideClicked()
        }
    }

    // Compact icon button for a phone quick toggle (DND, Flashlight, Wi-Fi, Bluetooth).
    component QuickToggleButton: FocusScope {
        id: qbtn
        property string iconPath
        property string shortTitle
        property string label
        property bool active: false
        property var feature: ({})
        readonly property bool available: feature.state === "available"
        readonly property alias hovered: qhover.hovered
        signal clicked
        signal lockedClicked

        height: 52
        activeFocusOnTab: available

        Accessible.role: Accessible.CheckBox
        Accessible.name: label
        Accessible.checked: active
        Accessible.onPressAction: if (qbtn.available) qbtn.clicked(); else qbtn.lockedClicked()
        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                if (qbtn.available)
                    qbtn.clicked()
                else
                    qbtn.lockedClicked()
                event.accepted = true
            }
        }

        readonly property color fillColor: {
            if (active && available)
                return Theme.graphite ? Theme.surfaceContent : Theme.primary
            if (active && !available)
                return Theme.graphite ? Theme.surfaceContainerHigh : Theme.secondaryContainer
            return Theme.graphite ? "transparent" : (Theme.glass ? Theme.tileColor : Theme.surfaceContainerHigh)
        }
        readonly property color contentColor: {
            if (active && available)
                return Theme.graphite ? Theme.surface : Theme.primaryContent
            if (active && !available)
                return Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
            return available ? Theme.surfaceContent : Theme.surfaceContentVariant
        }

        Rectangle {
            anchors.fill: parent
            radius: Theme.radiusMd
            color: qbtn.fillColor
            border.width: Theme.focusVisible(qbtn) ? 2 : (Theme.graphite || Theme.glass || (qbtn.active && !qbtn.available) ? 1 : 0)
            border.color: Theme.focusVisible(qbtn) ? Theme.primary : (Theme.glass ? Theme.cardBorder : Theme.outlineVariant)
            opacity: qbtn.available || qbtn.active ? 1 : 0.55
            scale: qtap.pressed && qbtn.available ? Theme.pressScale : 1
            Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 170; easing.type: Easing.OutBack } }

            Rectangle {
                anchors.fill: parent
                radius: parent.radius
                color: qbtn.contentColor
                opacity: qtap.pressed ? 0.09 : 0
            }

            Column {
                anchors.centerIn: parent
                width: parent.width - 4
                spacing: 3
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 17; height: 17
                    path: qbtn.iconPath
                    color: qbtn.contentColor
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    role: "caption"
                    size: Theme.graphite ? 9 : 11
                    font.letterSpacing: Theme.graphite ? 0 : (graphiteLabel ? Theme.labelStyle.tracking : spec.tracking) * size
                    color: qbtn.contentColor
                    text: qbtn.shortTitle
                    elide: Text.ElideRight
                }
            }

            Icon {
                visible: !qbtn.available
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 4
                width: 10; height: 10
                path: Icons.lock
                color: qbtn.contentColor
                opacity: 0.7
            }
        }

        HoverHandler { id: qhover; cursorShape: Qt.PointingHandCursor }
        TapHandler { id: qtap; gesturePolicy: TapHandler.ReleaseWithinBounds; onTapped: qbtn.available ? qbtn.clicked() : qbtn.lockedClicked() }
    }

    // Debounced 0..=100 slider for a phone setting (media volume, screen brightness).
    component ToggleSlider: Column {
        id: slider
        property string deviceId
        property string toggleId
        property string title
        property string iconPath
        property int phoneValue: 0
        property var feature: ({})
        readonly property bool available: feature.state === "available"
        property int dragValue: -1
        readonly property int shownValue: dragValue >= 0 ? dragValue : Math.max(0, Math.min(100, phoneValue))

        spacing: 4

        Timer {
            id: sendDebounce
            interval: 100
            onTriggered: if (slider.dragValue >= 0) {
                AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(slider.dragValue))
            }
        }
        Timer {
            id: settle
            interval: 400
            onTriggered: if (!dragArea.pressed) slider.dragValue = -1
        }

        Item {
            width: parent.width
            height: 20
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 15; height: 15
                    path: slider.iconPath
                    color: slider.available ? Theme.surfaceContent : Theme.surfaceContentVariant
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    role: "bodySmall"
                    muted: !slider.available
                    text: slider.title
                }
            }
            Txt {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                role: "caption"
                muted: true
                text: qsTr("%1%").arg(slider.shownValue)
            }
        }

        FocusScope {
            id: barScope
            width: parent.width
            height: 18
            activeFocusOnTab: slider.available
            Accessible.role: Accessible.Slider
            Accessible.name: slider.title
            Accessible.description: qsTr("%1%").arg(slider.shownValue)
            Accessible.onPressAction: if (slider.available) {
                const next = slider.shownValue >= 80 ? 25 : slider.shownValue + 20
                slider.dragValue = next
                settle.restart()
                AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
            }
            Keys.onPressed: (event) => {
                if (!slider.available) return
                if (event.key === Qt.Key_Left || event.key === Qt.Key_Down) {
                    const next = Math.max(0, slider.shownValue - 5)
                    slider.dragValue = next
                    settle.restart()
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
                    event.accepted = true
                } else if (event.key === Qt.Key_Right || event.key === Qt.Key_Up) {
                    const next = Math.min(100, slider.shownValue + 5)
                    slider.dragValue = next
                    settle.restart()
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(next))
                    event.accepted = true
                }
            }

            readonly property real fraction: slider.shownValue / 100.0

            Rectangle {
                anchors.fill: parent
                anchors.margins: -3
                radius: 6
                color: "transparent"
                border.width: 2
                border.color: Theme.primary
                visible: Theme.focusVisible(barScope)
            }

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 6
                radius: 3
                color: Theme.surfaceContainerHighest
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                opacity: slider.available ? 1 : 0.45

                Rectangle {
                    id: sliderFill
                    width: parent.width * barScope.fraction
                    height: parent.height
                    radius: parent.radius
                    color: slider.available
                           ? (Theme.graphite ? Theme.surfaceContent : Theme.primary)
                           : Theme.surfaceContentVariant
                    Behavior on width {
                        enabled: !Theme.reduceMotion && !dragArea.pressed
                        NumberAnimation { duration: 120; easing.type: Easing.OutCubic }
                    }
                }
            }

            Rectangle {
                visible: slider.available
                x: Math.max(0, Math.min(barScope.width - width, sliderFill.width - width / 2))
                anchors.verticalCenter: parent.verticalCenter
                width: 14; height: 14; radius: 7
                color: Theme.graphite ? Theme.surfaceContent : Theme.primary
                border.width: Theme.focusVisible(barScope) ? 2 : 0
                border.color: Theme.surface
                scale: dragArea.pressed ? 1.15 : 1.0
                Behavior on scale {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
                }
            }

            MouseArea {
                id: dragArea
                anchors.fill: parent
                anchors.margins: -4
                enabled: slider.available
                hoverEnabled: true
                cursorShape: slider.available ? Qt.PointingHandCursor : Qt.ArrowCursor
                function levelAt(x) {
                    return Math.round(Math.max(0, Math.min(1, (x - 4) / barScope.width)) * 100)
                }
                onPressed: (mouse) => {
                    settle.stop()
                    slider.dragValue = levelAt(mouse.x)
                    if (!sendDebounce.running) sendDebounce.start()
                }
                onPositionChanged: (mouse) => {
                    if (pressed) {
                        slider.dragValue = levelAt(mouse.x)
                        if (!sendDebounce.running) sendDebounce.start()
                    }
                }
                onReleased: (mouse) => {
                    sendDebounce.stop()
                    const finalVal = levelAt(mouse.x)
                    slider.dragValue = finalVal
                    AppController.setPhoneToggle(slider.deviceId, slider.toggleId, String(finalVal))
                    settle.restart()
                }
                onCanceled: {
                    sendDebounce.stop()
                    slider.dragValue = -1
                }
            }
        }

        LockChip {
            visible: !slider.available && label.length > 0 && slider.feature.action !== "enableToggle"
            feature: slider.feature
            maxWidth: parent.width
        }
    }

    // Nearby devices running LocalSend on the local Wi-Fi network.
    component LocalSendCard: Card {
        id: lsCard
        Column {
            width: parent.width
            spacing: 10
            Item {
                width: parent.width
                height: Math.max(lsHeaderRow.height, lsRefreshBtn.height)
                Row {
                    id: lsHeaderRow
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8
                    StatusDot {
                        anchors.verticalCenter: parent.verticalCenter
                        online: AppController.localsendReceiving
                    }
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        role: "label"
                        muted: true
                        text: page.localsendPeers.length > 0
                            ? qsTr("LocalSend nearby · %1").arg(page.localsendPeers.length)
                            : qsTr("LocalSend nearby")
                    }
                }
                Button {
                    id: lsRefreshBtn
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    variant: "text"
                    size: "sm"
                    text: qsTr("Refresh")
                    onClicked: AppController.refreshLocalsend()
                }
            }
            Txt {
                width: parent.width
                visible: page.localsendPeers.length === 0
                role: "bodySmall"
                muted: true
                wrapMode: Text.WordWrap
                text: AppController.localsendReceiving
                    ? qsTr("Listening for LocalSend devices on your Wi‑Fi network.")
                    : qsTr("LocalSend port 53317 is busy on this PC (another LocalSend app may be running).")
            }
            Repeater {
                model: page.localsendPeers
                delegate: Item {
                    id: peerRow
                    required property var modelData
                    required property int index
                    width: parent.width
                    height: Math.max(36, peerInfoCol.height + 8)

                    Row {
                        anchors.fill: parent
                        spacing: 12
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 32; height: 32
                            radius: Theme.graphite ? Theme.radiusSm : 16
                            color: Theme.graphite ? "transparent" : Theme.secondaryContainer
                            border.width: Theme.graphite ? 1 : 0
                            border.color: Theme.outlineVariant
                            Icon {
                                anchors.centerIn: parent
                                width: 18; height: 18
                                path: peerRow.modelData.deviceType === "mobile" || peerRow.modelData.deviceType === "tablet"
                                    ? Icons.phone : Icons.laptop
                                color: Theme.graphite ? Theme.surfaceContent : Theme.secondaryContainerContent
                            }
                        }
                        Column {
                            id: peerInfoCol
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 32 - sendPeerBtn.width - 24
                            spacing: 2
                            Txt {
                                width: parent.width
                                role: "title"
                                size: 14
                                elide: Text.ElideRight
                                text: peerRow.modelData.alias || qsTr("Nearby device")
                            }
                            Txt {
                                width: parent.width
                                role: "bodySmall"
                                muted: true
                                elide: Text.ElideRight
                                text: [peerRow.modelData.deviceModel, peerRow.modelData.ip]
                                    .filter(s => s && s.length > 0)
                                    .join(" · ")
                            }
                        }
                         Button {
                            id: sendPeerBtn
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            size: "sm"
                            iconPath: Icons.send
                            text: qsTr("Send files")
                            onClicked: {
                                localsendFilePicker.targetDeviceId = peerRow.modelData.id
                                localsendFilePicker.open()
                            }
                        }
                    }
                }
            }
        }
    }

    // Inline lightbox preview for notification images.
    Sheet {
        id: imagePreviewSheet
        property string imageUrl: ""
        property string caption: ""
        cardWidth: 680
        function openFor(url, title) {
            imageUrl = url || ""
            caption = title || ""
            if (imageUrl.length > 0)
                open()
        }
        Column {
            width: parent.width
            spacing: 14
            Item {
                width: parent.width
                height: Math.max(previewHeaderTxt.height, closePreviewBtn.height)
                Txt {
                    id: previewHeaderTxt
                    anchors.left: parent.left
                    anchors.right: closePreviewBtn.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    role: "headline"
                    elide: Text.ElideRight
                    text: imagePreviewSheet.caption.length > 0 ? imagePreviewSheet.caption : qsTr("Image preview")
                }
                IconButton {
                    id: closePreviewBtn
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.close
                    label: qsTr("Close")
                    onClicked: imagePreviewSheet.close()
                }
            }
            Rectangle {
                width: parent.width
                height: Math.min(480, Math.max(200, previewImg.status === Image.Ready
                    ? width * previewImg.implicitHeight / Math.max(1, previewImg.implicitWidth)
                    : 320))
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                clip: true
                Image {
                    id: previewImg
                    anchors.fill: parent
                    anchors.margins: 8
                    source: imagePreviewSheet.imageUrl
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    mipmap: true
                }
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                Button {
                    variant: "tonal"
                    iconPath: Icons.photo
                    text: qsTr("Open externally")
                    onClicked: Qt.openUrlExternally(imagePreviewSheet.imageUrl)
                }
                Button {
                    text: qsTr("Done")
                    onClicked: imagePreviewSheet.close()
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: 10
        radius: Theme.radiusLg
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: opacity > 0
        opacity: AppController.highlightTarget.startsWith("home:") ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    }
}
