// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import QtQuick.Shapes
import app.nectarlink

// Home: the paired phone at a glance (status, battery, power level) and
// quick actions. Actions reflect the capability matrix: what isn't
// available shows what unlocks it.
Item {
    id: page
    property bool active: true
    property int current: AppController.currentDevice
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

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

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
            x: Theme.contentPadding
            y: Theme.contentPadding
            width: parent.width - Theme.contentPadding * 2
            spacing: Theme.gutter

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
                    width: column.width
                    visible: index === page.current
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
                    ListView {
                        id: standaloneTransferList
                        width: parent.width
                        height: contentHeight
                        interactive: false
                        model: TransferList
                        delegate: TransferItem { width: standaloneTransferList.width }
                    }
                }
            }

            LocalSendCard {
                width: column.width
                visible: DeviceList.count === 0 && AppController.localsendEnabled
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

    component DeviceHome: Row {
        id: home
        required property string deviceId
        required property string name
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
        required property string power
        required property real lastSeen
        required property real pairedAt
        required property string accent
        required property string screen

        // Dropped files go to the device on screen (and Messages shows it).
        // Kept while the page is hidden.
        Binding { target: page; property: "currentDeviceId"; value: home.deviceId; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding { target: page; property: "currentDeviceName"; value: home.name; when: home.visible; restoreMode: Binding.RestoreNone }
        Binding {
            target: page
            property: "currentCanReceive"
            value: home.feature("files.send").state === "available"
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
        readonly property bool canAnimateHero: !Theme.reduceMotion && home.visible && page.visible
                                               && page.Window.visibility !== Window.Hidden
        property bool ringing: false
        Timer { id: ringTimeout; interval: 30000; onTriggered: home.ringing = false }

        spacing: Theme.gutter
        readonly property real sideWidth: 300

        Column {
            width: home.width - home.sideWidth - home.spacing
            spacing: Theme.gutter

            // ---- Hero ----
            Rectangle {
                id: heroCard
                width: parent.width
                readonly property bool wide: width >= 540
                readonly property bool hasSms: (home.online && home.feature("messages.sms").state === "available")
                                               || Boolean(home.summaryData.smsReady)
                readonly property bool hasCalls: (home.online && home.feature("calls.log").state === "available")
                                                 || Boolean(home.summaryData.callsReady)
                readonly property bool hasPhotos: home.feature("files.recent_photos").state === "available"
                                                  && Boolean(home.summaryData.photoReady)
                                                  && Boolean(home.summaryData.photoId && home.summaryData.photoId.length > 0)
                readonly property bool hasGlanceSummary: home.battery >= 0 || hasSms || hasCalls || hasPhotos
                height: wide
                    ? Math.max(190, Math.max(phoneArt.height, Math.max(identityCol.height, summaryPanel.height)) + 40)
                    : topHeroRow.height + (hasGlanceSummary ? summaryDivider.height + summaryPanel.height + 24 : 0) + 40
                radius: Theme.radiusXl
                color: Theme.heroColor
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant

                Item {
                    id: topHeroRow
                    x: 22
                    y: heroCard.wide ? Math.round((heroCard.height - height) / 2) : 20
                    width: heroCard.wide
                        ? heroCard.width - 44 - (heroCard.hasGlanceSummary ? summaryPanel.width + 36 : 0)
                        : heroCard.width - 44
                    height: Math.max(phoneArt.height, identityCol.height)

                    // Vector front-only phone preview measured on the phone (no assumed geometry).
                    Item {
                        id: phoneArt
                        anchors.verticalCenter: parent.verticalCenter
                        readonly property real aspect: {
                            const s = home.screenData
                            if (s && s.aspect >= 0.25 && s.aspect <= 2.5)
                                return s.aspect
                            return home.kind === "tablet" ? 0.70 : 0.45
                        }
                        height: heroCard.wide ? 148 : 132
                        width: Math.round(height * aspect)
                        readonly property real bezel: 3.5
                        readonly property real innerW: Math.max(10, width - bezel * 2)
                        readonly property real innerH: Math.max(20, height - bezel * 2)
                        readonly property real defaultCorner: width * 0.135
                        readonly property real rTl: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.tl * width) : defaultCorner
                        readonly property real rTr: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.tr * width) : defaultCorner
                        readonly property real rBr: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.br * width) : defaultCorner
                        readonly property real rBl: home.screenData && home.screenData.corners
                            ? Math.max(3, home.screenData.corners.bl * width) : defaultCorner
                        readonly property var phonePalette: Theme.phonePalette(home.deviceId)
                        readonly property color seedColor: phonePalette
                            ? Qt.color(phonePalette.primary)
                            : (home.accent && home.accent.length > 0 ? Qt.color(home.accent) : Theme.primary)
                        readonly property color frameFill: Theme.graphite
                            ? (Theme.dark ? "#1B1D21" : "#282B30")
                            : Qt.darker(seedColor, Theme.dark ? 2.8 : 2.3)
                        readonly property color frameStroke: Theme.graphite
                            ? Theme.outline
                            : Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.42)
                        readonly property color screenTop: Theme.graphite
                            ? (Theme.dark ? "#262930" : "#E4E7EC")
                            : (Theme.dark ? Qt.darker(seedColor, 1.85) : Qt.lighter(seedColor, 1.35))
                        readonly property color screenBottom: Theme.graphite
                            ? (Theme.dark ? "#17191D" : "#CFD4DC")
                            : (Theme.dark ? Qt.darker(seedColor, 2.55) : Qt.darker(seedColor, 1.25))
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
                            opacity: home.online ? (Theme.dark ? 0.22 : 0.16) : 0.0
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

                            // 3. Measured front camera cutout (SVG path or bounding rects; empty when plain/unknown).
                            ShapePath {
                                strokeWidth: phoneArt.cutoutSvg.length > 0 ? 0.8 : -1
                                strokeColor: Qt.rgba(1, 1, 1, Theme.dark ? 0.16 : 0.22)
                                fillColor: phoneArt.cutoutSvg.length > 0 ? phoneArt.frameFill : "transparent"
                                PathSvg { path: phoneArt.cutoutSvg }
                            }
                        }

                        // Subtle gesture pill at the bottom of the screen.
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: phoneArt.bezel + 4
                            width: Math.round(phoneArt.innerW * 0.34)
                            height: 2.5
                            radius: 1.25
                            color: Qt.rgba(1, 1, 1, Theme.dark ? 0.32 : 0.45)
                            visible: !home.charging
                        }

                        // Calm charging hint at the bottom edge while the phone charges.
                        Rectangle {
                            id: chargingHint
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: phoneArt.bezel + 4
                            height: 16
                            width: chargeRow.width + 10
                            radius: 8
                            color: Qt.rgba(0, 0, 0, Theme.dark ? 0.48 : 0.36)
                            border.width: 1
                            border.color: Qt.rgba(1, 1, 1, 0.22)
                            opacity: home.charging ? 1.0 : 0.0
                            visible: opacity > 0
                            Behavior on opacity {
                                enabled: home.canAnimateHero
                                NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic }
                            }
                            Row {
                                id: chargeRow
                                anchors.centerIn: parent
                                spacing: 3
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 10
                                    height: 10
                                    path: Icons.bolt
                                    color: "#F8D66D"
                                }
                                Txt {
                                    anchors.verticalCenter: parent.verticalCenter
                                    size: 9
                                    weight: Font.DemiBold
                                    color: "#FFFFFF"
                                    text: home.battery >= 0 ? (home.battery + "%") : ""
                                }
                            }
                        }
                    }

                    Column {
                        id: identityCol
                        anchors.left: phoneArt.right
                        anchors.leftMargin: 20
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 6
                        Txt {
                            width: parent.width
                            text: home.name
                            role: "displaySmall"
                            color: Theme.heroContent
                            elide: Text.ElideRight
                        }
                        Txt {
                            width: parent.width
                            role: "body"
                            color: Theme.heroContent
                            opacity: 0.78
                            elide: Text.ElideRight
                            visible: text.length > 0
                            // The model is often a code ("SM-A356E"), so only the version.
                            text: home.osVersion.length > 0 ? qsTr("Android %1").arg(home.osVersion) : ""
                        }
                        Flow {
                            width: parent.width
                            spacing: 6
                            Chip {
                                visible: home.battery >= 0 && !heroCard.hasGlanceSummary
                                iconPath: Icons.battery
                                text: home.charging ? qsTr("%1% · charging").arg(home.battery) : qsTr("%1%").arg(home.battery)
                            }
                            Chip {
                                iconPath: home.online ? Icons.wifi : Icons.unlink
                                text: home.online
                                      ? (home.rttMs >= 1
                                         ? (home.path === "relay" ? qsTr("Away · %1 ms") : qsTr("Wi‑Fi · %1 ms")).arg(home.rttMs)
                                         : (home.path === "relay" ? qsTr("Away") : qsTr("Wi‑Fi")))
                                      : (home.lastSeen > 0 ? qsTr("Offline · seen %1").arg(page.relativeTime(home.lastSeen))
                                                           : qsTr("Not connected yet"))
                            }
                            Chip {
                                visible: home.power === "assist" || home.power === "elevated"
                                iconPath: Icons.sparkle
                                text: home.power === "elevated" ? qsTr("Elevated") : qsTr("Assist")
                            }
                        }
                    }
                }

                // Vertical or horizontal divider between phone identity and glanceable summary.
                Rectangle {
                    id: summaryDivider
                    visible: heroCard.hasGlanceSummary
                    x: heroCard.wide ? heroCard.width - summaryPanel.width - 36 : 22
                    y: heroCard.wide ? 22 : topHeroRow.y + topHeroRow.height + 12
                    width: heroCard.wide ? 1 : heroCard.width - 44
                    height: heroCard.wide ? heroCard.height - 44 : 1
                    color: Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.14)
                }

                // Calm, glanceable summary filling the Hero card's right half (or reflowing below on narrow windows).
                Column {
                    id: summaryPanel
                    visible: heroCard.hasGlanceSummary
                    width: heroCard.wide
                        ? Math.min(312, Math.max(228, Math.round(heroCard.width * 0.45)))
                        : heroCard.width - 44
                    x: heroCard.wide ? heroCard.width - width - 20 : 22
                    y: heroCard.wide
                        ? Math.round((heroCard.height - height) / 2)
                        : summaryDivider.y + summaryDivider.height + 12
                    spacing: 8

                    // 1. Battery progress bar + charging / time to full.
                    Rectangle {
                        width: parent.width
                        height: batteryCol.height + 16
                        visible: home.battery >= 0
                        radius: Theme.radiusMd
                        color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.34 : 0.48)
                        border.width: 1
                        border.color: Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.10)

                        Column {
                            id: batteryCol
                            x: 12
                            y: 8
                            width: parent.width - 24
                            spacing: 6

                            Item {
                                width: parent.width
                                height: Math.max(batteryTitleRow.height, batteryStatusTxt.height)
                                Row {
                                    id: batteryTitleRow
                                    anchors.left: parent.left
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 6
                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 15
                                        height: 15
                                        path: Icons.battery
                                        color: home.battery <= 15 && !home.charging ? Theme.error : Theme.heroContent
                                    }
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        role: "label"
                                        color: Theme.heroContent
                                        text: qsTr("%1%").arg(home.battery)
                                    }
                                }
                                Txt {
                                    id: batteryStatusTxt
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Math.max(60, parent.width - batteryTitleRow.width - 8)
                                    horizontalAlignment: Text.AlignRight
                                    elide: Text.ElideRight
                                    role: "caption"
                                    color: Theme.heroContent
                                    opacity: 0.80
                                    text: {
                                        if (!home.charging)
                                            return home.battery <= 20 ? qsTr("Low battery") : qsTr("Battery")
                                        const eta = page.formatChargeEta(home.batteryFullIn)
                                        if (eta.length > 0)
                                            return eta
                                        if (home.batteryPlugged === "wireless")
                                            return qsTr("Charging wirelessly")
                                        if (home.batteryPlugged === "usb")
                                            return qsTr("Charging via USB")
                                        return qsTr("Charging")
                                    }
                                }
                            }

                            Rectangle {
                                width: parent.width
                                height: 6
                                radius: 3
                                color: Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.16)
                                Rectangle {
                                    width: Math.max(6, Math.round(parent.width * Math.min(100, Math.max(0, home.battery)) / 100))
                                    height: parent.height
                                    radius: 3
                                    color: home.battery <= 15 && !home.charging
                                        ? Theme.error
                                        : (Theme.graphite ? Theme.heroContent : Theme.primary)
                                }
                            }
                        }
                    }

                    // 2. Unread messages & missed calls pills (clickable to open Messages / Calls).
                    Row {
                        id: commRow
                        width: parent.width
                        spacing: 8
                        visible: heroCard.hasSms || heroCard.hasCalls
                        readonly property int pillCount: (heroCard.hasSms ? 1 : 0) + (heroCard.hasCalls ? 1 : 0)
                        readonly property real pillWidth: pillCount > 1 ? (width - spacing) / 2 : width

                        Rectangle {
                            id: smsPill
                            visible: heroCard.hasSms
                            width: commRow.pillWidth
                            height: smsPillRow.height + 14
                            radius: Theme.radiusMd
                            activeFocusOnTab: true
                            readonly property int unread: home.summaryData.unreadMessages || 0
                            color: smsHover.hovered
                                ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.48 : 0.68)
                                : Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.34 : 0.48)
                            border.width: Theme.focusVisible(smsPill) ? 2 : 1
                            border.color: Theme.focusVisible(smsPill)
                                ? Theme.primary
                                : (unread > 0
                                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.45)
                                    : Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.10))
                            Accessible.role: Accessible.Button
                            Accessible.name: unread > 0
                                ? qsTr("%n unread", "", unread)
                                : qsTr("Messages")
                            Accessible.onPressAction: AppController.currentPage = "messages"
                            Keys.onPressed: (event) => {
                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                    AppController.currentPage = "messages"
                                    event.accepted = true
                                }
                            }

                            Row {
                                id: smsPillRow
                                x: 10
                                y: 7
                                width: parent.width - 20
                                spacing: 8
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 15
                                    height: 15
                                    path: Icons.messages
                                    color: (home.summaryData.unreadMessages || 0) > 0 ? Theme.primary : Theme.heroContent
                                }
                                Column {
                                    width: parent.width - 23
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 1
                                    Txt {
                                        width: parent.width
                                        role: "label"
                                        color: Theme.heroContent
                                        elide: Text.ElideRight
                                        text: (home.summaryData.unreadMessages || 0) > 0
                                            ? qsTr("%n unread", "", home.summaryData.unreadMessages)
                                            : qsTr("Messages")
                                    }
                                    Txt {
                                        width: parent.width
                                        role: "caption"
                                        color: Theme.heroContent
                                        opacity: 0.80
                                        elide: Text.ElideRight
                                        text: (home.summaryData.unreadMessages || 0) > 0 && home.summaryData.unreadSender
                                            ? home.summaryData.unreadSender
                                            : qsTr("All read")
                                    }
                                }
                            }
                            HoverHandler { id: smsHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: AppController.currentPage = "messages" }
                        }

                        Rectangle {
                            id: callsPill
                            visible: heroCard.hasCalls
                            width: commRow.pillWidth
                            height: callsPillRow.height + 14
                            radius: Theme.radiusMd
                            activeFocusOnTab: true
                            readonly property int missed: home.summaryData.missedCalls || 0
                            color: callsHover.hovered
                                ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.48 : 0.68)
                                : Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.34 : 0.48)
                            border.width: Theme.focusVisible(callsPill) ? 2 : 1
                            border.color: Theme.focusVisible(callsPill)
                                ? Theme.primary
                                : (missed > 0
                                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.45)
                                    : Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.10))
                            Accessible.role: Accessible.Button
                            Accessible.name: missed > 0
                                ? qsTr("%n missed", "", missed)
                                : qsTr("Calls")
                            Accessible.onPressAction: AppController.currentPage = "calls"
                            Keys.onPressed: (event) => {
                                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                    AppController.currentPage = "calls"
                                    event.accepted = true
                                }
                            }

                            Row {
                                id: callsPillRow
                                x: 10
                                y: 7
                                width: parent.width - 20
                                spacing: 8
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 15
                                    height: 15
                                    path: Icons.call
                                    color: (home.summaryData.missedCalls || 0) > 0 ? Theme.primary : Theme.heroContent
                                }
                                Column {
                                    width: parent.width - 23
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 1
                                    Txt {
                                        width: parent.width
                                        role: "label"
                                        color: Theme.heroContent
                                        elide: Text.ElideRight
                                        text: (home.summaryData.missedCalls || 0) > 0
                                            ? qsTr("%n missed", "", home.summaryData.missedCalls)
                                            : qsTr("Calls")
                                    }
                                    Txt {
                                        width: parent.width
                                        role: "caption"
                                        color: Theme.heroContent
                                        opacity: 0.80
                                        elide: Text.ElideRight
                                        text: (home.summaryData.missedCalls || 0) > 0 && home.summaryData.missedCaller
                                            ? home.summaryData.missedCaller
                                            : qsTr("Recent calls")
                                    }
                                }
                            }
                            HoverHandler { id: callsHover; cursorShape: Qt.PointingHandCursor }
                            TapHandler { onTapped: AppController.currentPage = "calls" }
                        }
                    }

                    // 3. Latest photo or screenshot (rounded thumbnail + relative time; click opens Photos).
                    Rectangle {
                        id: photoPill
                        width: parent.width
                        height: 52
                        visible: heroCard.hasPhotos
                        radius: Theme.radiusMd
                        activeFocusOnTab: true
                        color: photoHover.hovered
                            ? Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.48 : 0.68)
                            : Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.34 : 0.48)
                        border.width: Theme.focusVisible(photoPill) ? 2 : 1
                        border.color: Theme.focusVisible(photoPill)
                            ? Theme.primary
                            : Qt.rgba(Theme.heroContent.r, Theme.heroContent.g, Theme.heroContent.b, 0.10)
                        Accessible.role: Accessible.Button
                        Accessible.name: home.summaryData.photoIsScreenshot
                            ? qsTr("Latest screenshot")
                            : (home.summaryData.photoIsVideo ? qsTr("Latest video") : qsTr("Latest photo"))
                        Accessible.onPressAction: AppController.currentPage = "photos"
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                AppController.currentPage = "photos"
                                event.accepted = true
                            }
                        }

                        Row {
                            x: 8
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 16
                            spacing: 10

                            Rectangle {
                                width: 36
                                height: 36
                                radius: Theme.radiusSm
                                color: Theme.surfaceContainerHigh
                                clip: true
                                anchors.verticalCenter: parent.verticalCenter

                                Image {
                                    id: latestThumbImg
                                    anchors.fill: parent
                                    source: home.summaryData.photoThumb || ""
                                    fillMode: Image.PreserveAspectCrop
                                    asynchronous: true
                                    cache: false
                                    sourceSize.width: 72
                                    sourceSize.height: 72
                                    visible: status === Image.Ready
                                }
                                Icon {
                                    anchors.centerIn: parent
                                    width: 16
                                    height: 16
                                    path: Icons.photo
                                    color: Theme.surfaceContentVariant
                                    visible: latestThumbImg.status !== Image.Ready
                                }
                            }

                            Column {
                                width: parent.width - 46
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    role: "label"
                                    color: Theme.heroContent
                                    elide: Text.ElideRight
                                    text: home.summaryData.photoIsScreenshot
                                        ? qsTr("Latest screenshot")
                                        : (home.summaryData.photoIsVideo ? qsTr("Latest video") : qsTr("Latest photo"))
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    color: Theme.heroContent
                                    opacity: 0.80
                                    elide: Text.ElideRight
                                    text: {
                                        const when = home.summaryData.photoDate
                                            ? page.relativeTime(Math.round(home.summaryData.photoDate / 1000))
                                            : ""
                                        const name = home.summaryData.photoName || ""
                                        return [when, name].filter(s => s.length > 0).join(" · ")
                                    }
                                }
                            }
                        }
                        HoverHandler { id: photoHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: AppController.currentPage = "photos" }
                    }
                }
            }

            // ---- Quick actions ----
            Grid {
                id: actions
                width: parent.width
                columns: width > 640 ? 4 : 2
                spacing: 12
                readonly property real tileWidth: (width - spacing * (columns - 1)) / columns

                ActionTile {
                    width: actions.tileWidth
                    title: home.ringing ? qsTr("Stop ringing") : qsTr("Find phone")
                    subtitle: qsTr("Rings even on silent")
                    iconPath: Icons.ring
                    feature: home.feature("device.find_phone")
                    active: home.ringing
                    onClicked: {
                        home.ringing = !home.ringing
                        if (home.ringing) ringTimeout.restart()
                        AppController.ring(home.deviceId, home.ringing)
                    }
                }
                ActionTile {
                    width: actions.tileWidth
                    title: qsTr("Send clipboard")
                    subtitle: Preferences.autoClipboard ? qsTr("Syncs automatically") : qsTr("To the phone")
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
                    title: qsTr("Mirror screen")
                    subtitle: qsTr("See it on this PC")
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
                    title: qsTr("Send files")
                    subtitle: qsTr("Or drop them here")
                    iconPath: Icons.send
                    feature: home.feature("files.send")
                    onClicked: filePicker.open()
                    sideIcon: Icons.folder
                    sideLabel: qsTr("Send a folder")
                    onSideClicked: folderPicker.open()
                }
            }

            // ---- Continuity Camera ----
            Card {
                id: continuityHomeCard
                width: parent.width
                readonly property var continuityFeature: home.feature("camera.continuity")
                readonly property bool available: continuityFeature.state === "available"
                visible: home.online && (available || continuityFeature.state === "locked" || AppController.continuityCameraBusy)
                Column {
                    width: parent.width
                    spacing: 8
                    Item {
                        width: parent.width
                        height: Math.max(continuityLeftRow.height, continuityRightRow.height)
                        Row {
                            id: continuityLeftRow
                            anchors.left: parent.left
                            anchors.right: continuityRightRow.left
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 10
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 18; height: 18
                                path: Icons.camera
                                color: Theme.primary
                            }
                            Column {
                                width: parent.width - 28
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    role: "label"
                                    elide: Text.ElideRight
                                    text: AppController.continuityCameraBusy
                                        ? AppController.continuityCameraStatus
                                        : qsTr("Continuity Camera")
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: AppController.continuityCameraBusy
                                        ? qsTr("It goes on the clipboard and into the active window when ready.")
                                        : qsTr("With %1's camera, pasted where your cursor is.").arg(home.name)
                                }
                            }
                        }
                        Row {
                            id: continuityRightRow
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Button {
                                visible: !AppController.continuityCameraBusy
                                enabled: continuityHomeCard.available
                                variant: "tonal"
                                size: "sm"
                                iconPath: Icons.camera
                                text: qsTr("Take photo")
                                onClicked: AppController.startContinuityCamera("photo")
                            }
                            Button {
                                visible: !AppController.continuityCameraBusy
                                enabled: continuityHomeCard.available
                                variant: "tonal"
                                size: "sm"
                                iconPath: Icons.clipboardList
                                text: qsTr("Scan document")
                                onClicked: AppController.startContinuityCamera("scan")
                            }
                            Button {
                                visible: AppController.continuityCameraBusy
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.close
                                text: qsTr("Cancel")
                                onClicked: AppController.cancelContinuityCamera()
                            }
                        }
                    }
                    LockChip {
                        visible: !continuityHomeCard.available && label.length > 0
                        feature: continuityHomeCard.continuityFeature
                    }
                }
            }

            // ---- Type on phone with PC keyboard (without mirroring) ----
            Card {
                id: typeOnPhoneCard
                width: parent.width
                readonly property var controlFeature: home.feature("mirroring.control")
                readonly property bool available: controlFeature.state === "available"
                readonly property bool typingActive: Mirror.keyboardDevice === home.deviceId
                visible: home.online && (available || controlFeature.state === "locked" || typingActive)
                onTypingActiveChanged: {
                    if (typingActive) {
                        liveKeyCapture.forceActiveFocus(Qt.OtherFocusReason)
                    }
                }
                Column {
                    width: parent.width
                    spacing: 10
                    Item {
                        width: parent.width
                        height: Math.max(typeLeftRow.height, typeRightRow.height)
                        Row {
                            id: typeLeftRow
                            anchors.left: parent.left
                            anchors.right: typeRightRow.left
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 10
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 18; height: 18
                                path: Icons.keyboard
                                color: Theme.primary
                            }
                            Column {
                                width: parent.width - 28
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Txt {
                                    width: parent.width
                                    role: "label"
                                    elide: Text.ElideRight
                                    text: typeOnPhoneCard.typingActive
                                        ? qsTr("Typing on %1").arg(home.name)
                                        : qsTr("Type on phone")
                                }
                                Txt {
                                    width: parent.width
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                    text: typeOnPhoneCard.typingActive
                                        ? qsTr("Press keys in the capture box below, or compose text to send (Esc to stop).")
                                        : qsTr("Use this PC's keyboard on %1 without mirroring its screen.").arg(home.name)
                                }
                            }
                        }
                        Row {
                            id: typeRightRow
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Button {
                                visible: !typeOnPhoneCard.typingActive
                                enabled: typeOnPhoneCard.available
                                variant: "tonal"
                                size: "sm"
                                iconPath: Icons.keyboard
                                text: qsTr("Start typing")
                                onClicked: Mirror.toggleKeyboard(home.deviceId)
                            }
                            Button {
                                visible: typeOnPhoneCard.typingActive
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.close
                                text: qsTr("Stop typing")
                                onClicked: Mirror.stopKeyboard()
                            }
                        }
                    }

                    Column {
                        width: parent.width
                        visible: typeOnPhoneCard.typingActive
                        spacing: 8

                        FocusScope {
                            id: liveKeyCapture
                            width: parent.width
                            height: 40
                            activeFocusOnTab: true
                            property string lastKeyStatus: ""
                            Accessible.role: Accessible.EditableText
                            Accessible.name: qsTr("Live keyboard capture for %1").arg(home.name)
                            Accessible.description: qsTr("Type directly to send keystrokes to the phone, or press Escape to stop.")

                            Rectangle {
                                anchors.fill: parent
                                anchors.margins: -3
                                radius: (Theme.graphite ? Theme.radiusSm : 20) + 3
                                color: "transparent"
                                border.width: 2
                                border.color: Theme.primary
                                visible: Theme.focusVisible(liveKeyCapture)
                            }

                            Rectangle {
                                anchors.fill: parent
                                radius: Theme.graphite ? Theme.radiusSm : height / 2
                                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                                border.width: liveKeyCapture.activeFocus ? 2 : 1
                                border.color: liveKeyCapture.activeFocus ? Theme.primary : Theme.outlineVariant

                                Row {
                                    anchors.left: parent.left
                                    anchors.leftMargin: 14
                                    anchors.right: parent.right
                                    anchors.rightMargin: 14
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 10
                                    Icon {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 16; height: 16
                                        path: Icons.keyboard
                                        color: liveKeyCapture.activeFocus ? Theme.primary : Theme.surfaceContentVariant
                                    }
                                    Txt {
                                        width: parent.width - 26
                                        anchors.verticalCenter: parent.verticalCenter
                                        role: "bodySmall"
                                        muted: !liveKeyCapture.activeFocus
                                        elide: Text.ElideRight
                                        text: liveKeyCapture.activeFocus
                                            ? (liveKeyCapture.lastKeyStatus.length > 0
                                               ? qsTr("Live keys active · Sent: %1").arg(liveKeyCapture.lastKeyStatus)
                                               : qsTr("Live keys active — type here, or press Ctrl+V to paste"))
                                            : qsTr("Click here to capture live keystrokes")
                                    }
                                }
                                TapHandler {
                                    onTapped: liveKeyCapture.forceActiveFocus(Qt.MouseFocusReason)
                                }
                            }

                            Keys.onPressed: (event) => {
                                if (event.key === Qt.Key_Escape) {
                                    Mirror.stopKeyboard()
                                    event.accepted = true
                                    return
                                }
                                if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V) {
                                    Mirror.keyboardPaste(home.deviceId)
                                    liveKeyCapture.lastKeyStatus = qsTr("Clipboard")
                                    event.accepted = true
                                    return
                                }
                                const names = {}
                                names[Qt.Key_Return] = "enter"
                                names[Qt.Key_Enter] = "enter"
                                names[Qt.Key_Backspace] = "backspace"
                                names[Qt.Key_Delete] = "delete"
                                names[Qt.Key_Left] = "left"
                                names[Qt.Key_Right] = "right"
                                names[Qt.Key_Up] = "up"
                                names[Qt.Key_Down] = "down"
                                names[Qt.Key_Tab] = "tab"
                                if (names[event.key]) {
                                    Mirror.keyboardPress(home.deviceId, names[event.key])
                                    liveKeyCapture.lastKeyStatus = names[event.key]
                                    event.accepted = true
                                } else if (event.text.length > 0
                                           && !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))
                                           && event.text.charCodeAt(0) >= 32) {
                                    Mirror.keyboardText(home.deviceId, event.text)
                                    liveKeyCapture.lastKeyStatus = event.text
                                    event.accepted = true
                                }
                            }
                        }

                        Row {
                            width: parent.width
                            spacing: 8
                            Rectangle {
                                id: composeBox
                                width: Math.max(120, parent.width - sendComposeBtn.width - pasteBtn.width - backspaceBtn.width - enterKeyBtn.width - 32)
                                height: 36
                                radius: Theme.graphite ? Theme.radiusSm : height / 2
                                color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                                border.width: composeInput.activeFocus ? 2 : 1
                                border.color: composeInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                TextInput {
                                    id: composeInput
                                    anchors.fill: parent
                                    anchors.leftMargin: 14
                                    anchors.rightMargin: 14
                                    verticalAlignment: TextInput.AlignVCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: 13
                                    color: Theme.surfaceContent
                                    selectionColor: Theme.primaryContainer
                                    selectedTextColor: Theme.primaryContainerContent
                                    clip: true
                                    Accessible.name: qsTr("Type text to send to %1").arg(home.name)
                                    Keys.onReturnPressed: {
                                        if (composeInput.text.length > 0) {
                                            Mirror.keyboardText(home.deviceId, composeInput.text)
                                            composeInput.text = ""
                                        } else {
                                            Mirror.keyboardPress(home.deviceId, "enter")
                                        }
                                    }
                                    Keys.onEnterPressed: {
                                        if (composeInput.text.length > 0) {
                                            Mirror.keyboardText(home.deviceId, composeInput.text)
                                            composeInput.text = ""
                                        } else {
                                            Mirror.keyboardPress(home.deviceId, "enter")
                                        }
                                    }
                                    Keys.onEscapePressed: Mirror.stopKeyboard()
                                    Txt {
                                        anchors.verticalCenter: parent.verticalCenter
                                        visible: composeInput.text.length === 0
                                        role: "bodySmall"
                                        muted: true
                                        elide: Text.ElideRight
                                        width: parent.width
                                        text: qsTr("Or type a phrase and press Enter…")
                                    }
                                }
                            }
                            Button {
                                id: sendComposeBtn
                                variant: "tonal"
                                size: "sm"
                                enabled: composeInput.text.length > 0
                                text: qsTr("Send")
                                onClicked: {
                                    if (composeInput.text.length > 0) {
                                        Mirror.keyboardText(home.deviceId, composeInput.text)
                                        composeInput.text = ""
                                    }
                                }
                            }
                            Button {
                                id: pasteBtn
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.clipboard
                                text: qsTr("Paste")
                                onClicked: Mirror.keyboardPaste(home.deviceId)
                            }
                            Button {
                                id: backspaceBtn
                                variant: "outline"
                                size: "sm"
                                iconPath: Icons.backspace
                                text: qsTr("Bksp")
                                onClicked: Mirror.keyboardPress(home.deviceId, "backspace")
                            }
                            Button {
                                id: enterKeyBtn
                                variant: "outline"
                                size: "sm"
                                text: qsTr("Enter")
                                onClicked: Mirror.keyboardPress(home.deviceId, "enter")
                            }
                        }
                    }

                    LockChip {
                        visible: !typeOnPhoneCard.available && label.length > 0
                        feature: typeOnPhoneCard.controlFeature
                    }
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
                function onHandoffFilePickerRequested() {
                    handoffFilePicker.open()
                }
            }
            FolderDialog {
                id: folderPicker
                title: qsTr("Send a folder to %1").arg(home.name)
                onAccepted: TransferList.send(home.deviceId, [selectedFolder.toString()])
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
                    ListView {
                        id: transferList
                        width: parent.width
                        height: contentHeight
                        interactive: false
                        model: TransferList
                        delegate: TransferItem { width: transferList.width }
                        add: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 200 }
                        }
                        displaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 220; easing.type: Easing.OutCubic }
                        }
                    }
                }
            }

            // ---- LocalSend nearby ----
            LocalSendCard {
                width: parent.width
                visible: AppController.localsendEnabled
            }

            // ---- Notifications ----
            Card {
                id: feed
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
                        IconButton {
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
                    // Ticks every minute so "5 min" ages without new events.
                    Timer {
                        id: feedClock
                        property int minutes: 0
                        interval: 60000
                        repeat: true
                        running: feed.count > 0
                        onTriggered: minutes++
                    }
                    // The page scrolls; this list only lays out and animates:
                    // new notifications fade and slide in, dismissed ones fade
                    // out, and the rest glide into place.
                    ListView {
                        id: list
                        width: parent.width
                        height: contentHeight
                        interactive: false
                        model: NotificationList
                        delegate: NotificationItem {
                            width: list.width
                            clock: feedClock.minutes
                            visible: deviceId === home.deviceId
                            height: visible ? implicitHeight : 0
                            canOpenApp: home.feature("mirroring.app_windows").state === "available" && app.length > 0
                            onOpenAppRequested: Mirror.startApp(deviceId, app, appName.length > 0 ? appName : app)
                            onOptionsRequested: appSheet.openFor(app, appName)
                        }
                        add: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: 220; easing.type: Easing.OutCubic }
                            NumberAnimation {
                                property: "y"
                                from: ViewTransition.destination.y - 10
                                to: ViewTransition.destination.y
                                duration: 260
                                easing.type: Easing.OutCubic
                            }
                        }
                        addDisplaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 240; easing.type: Easing.OutCubic }
                        }
                        remove: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "opacity"; to: 0; duration: 160 }
                        }
                        removeDisplaced: Transition {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { property: "y"; duration: 240; easing.type: Easing.OutCubic }
                        }
                        Behavior on height {
                            enabled: !Theme.reduceMotion
                            NumberAnimation { duration: 240; easing.type: Easing.OutCubic }
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
                              ? qsTr("Notifications from %1 show up here and as Windows notifications.").arg(home.name)
                              : home.lastSeen > 0 || home.online
                                ? qsTr("Notifications from %1 aren't available yet.").arg(home.name)
                                : qsTr("Notifications show up here once %1 has connected.").arg(home.name)
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
                                width: parent.width - 32 - turnOn.width - 24
                                role: "bodySmall"
                                wrapMode: Text.WordWrap
                                text: qsTr("Windows notifications are turned off, so these only show up here.")
                            }
                            Button {
                                id: turnOn
                                anchors.verticalCenter: parent.verticalCenter
                                variant: "tonal"
                                size: "sm"
                                text: qsTr("Turn on")
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

            // What plays on the phone (its first player).
            Repeater {
                model: MediaList
                delegate: NowPlaying {
                    required property int index
                    width: home.sideWidth
                    visible: deviceId === home.deviceId
                             && MediaList.revision >= 0 && index === MediaList.firstFor(home.deviceId)
                }
            }

            // Active webcam session on this phone.
            Card {
                id: webcamHomeCard
                width: parent.width
                readonly property var webcamFeature: home.feature("camera.webcam")
                readonly property bool isThisPhone: (Webcam.phase === "streaming" || Webcam.phase === "asking")
                                                    && Webcam.activeDevice === home.deviceId
                visible: isThisPhone
                Column {
                    width: parent.width
                    spacing: 10
                    Row {
                        spacing: 8
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            path: Icons.video
                            color: Theme.primary
                        }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Webcam")
                            role: "label"
                        }
                    }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: Webcam.statusText.length > 0
                              ? Webcam.statusText
                              : qsTr("Using %1's camera on this PC.").arg(home.name)
                    }
                    Button {
                        variant: "tonal"
                        size: "sm"
                        iconPath: Icons.close
                        text: qsTr("Stop webcam")
                        onClicked: Webcam.stop()
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
                property var hoveredLockedFeature: null
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
                        readonly property int buttonCount: phoneControls.hasFlashlight ? 4 : 3
                        readonly property real buttonWidth: (width - spacing * (buttonCount - 1)) / buttonCount

                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.moon
                            shortTitle: qsTr("DND")
                            label: qsTr("Do Not Disturb")
                            active: phoneControls.state ? Boolean(phoneControls.state.dnd) : false
                            feature: phoneControls.dndFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
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
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: AppController.setPhoneToggle(home.deviceId, "flashlight", String(!active))
                        }
                        QuickToggleButton {
                            width: toggleRow.buttonWidth
                            iconPath: Icons.wifi
                            shortTitle: qsTr("Wi‑Fi")
                            label: qsTr("Wi‑Fi")
                            active: phoneControls.state ? Boolean(phoneControls.state.wifi) : false
                            feature: phoneControls.wifiFeature
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
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
                            onHoveredChanged: if (hovered && !available) phoneControls.hoveredLockedFeature = feature
                            onClicked: AppController.setPhoneToggle(home.deviceId, "bluetooth", String(!active))
                        }
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
                                text: qsTr("Turning Wi‑Fi off on %1 will disconnect it if Wi‑Fi is how it reaches this PC.")
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

            // Recent timeline items across devices.
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
                        text: qsTr("Files, clips, links, saved photos, recordings and sessions shared between your devices appear here.")
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

            Card {
                width: parent.width
                Column {
                    width: parent.width
                    spacing: 10
                    Txt { text: qsTr("Connection"); role: "label"; muted: true }
                    Row {
                        spacing: 10
                        StatusDot { anchors.verticalCenter: parent.verticalCenter; online: home.online }
                        Txt {
                            anchors.verticalCenter: parent.verticalCenter
                            role: "body"
                            text: home.online
                                  ? (home.path === "relay" ? qsTr("Connected while away") : qsTr("Connected on this network"))
                                  : qsTr("Not connected")
                        }
                    }
                    Txt {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        role: "bodySmall"
                        muted: true
                        text: home.online
                              ? (home.rttMs >= 1 ? qsTr("Round trip %1 ms · End-to-end encrypted").arg(home.rttMs)
                                                 : qsTr("Round trip under 1 ms · End-to-end encrypted"))
                              : home.lastSeen > 0
                                ? qsTr("Last seen %1. Reconnects automatically on the same network.")
                                      .arg(page.relativeTime(home.lastSeen))
                                : qsTr("Open Nectarlink on %1 to connect on this network.")
                                      .arg(home.name)
                    }
                    Button {
                        visible: !home.online
                        variant: "tonal"
                        size: "sm"
                        text: qsTr("Check the connection")
                        onClicked: AppController.runDoctor()
                    }
                    Divider {
                        width: parent.width
                        visible: home.online && !webcamHomeCard.isThisPhone
                                 && (webcamHomeCard.webcamFeature.state === "available"
                                     || webcamHomeCard.webcamFeature.action === "enableAddon")
                    }
                    Item {
                        width: parent.width
                        height: 32
                        visible: home.online && !webcamHomeCard.isThisPhone
                                 && (webcamHomeCard.webcamFeature.state === "available"
                                     || webcamHomeCard.webcamFeature.action === "enableAddon")
                        Row {
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 16; height: 16
                                path: Icons.video
                                color: Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                role: "bodySmall"
                                text: qsTr("Phone as webcam")
                            }
                        }
                        Button {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            size: "sm"
                            text: qsTr("Start")
                            onClicked: Webcam.start(home.deviceId)
                        }
                    }
                    Divider { width: parent.width }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        text: qsTr("This PC (%1) · Paired %2")
                              .arg(AppController.deviceName)
                              .arg(new Date(home.pairedAt * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat))
                        wrapMode: Text.WordWrap
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
                      ? qsTr("Nothing from %1 shows on this PC. It still shows on your phone.").arg(appSheet.appName)
                      : appSheet.rule === "quiet"
                        ? qsTr("Notifications from %1 show here in Nectarlink, without Windows pop-ups or sounds.").arg(appSheet.appName)
                        : qsTr("Notifications from %1 show here and as Windows notifications.").arg(appSheet.appName)
            }
            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: qsTr("You can change this for every app in Settings.")
            }
            Row {
                anchors.right: parent.right
                Button { text: qsTr("Done"); onClicked: appSheet.close() }
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
                      ? qsTr("Notifications that went away on your phone, or that you dismissed, for a day.")
                      : qsTr("Nothing here yet. Notifications that go away on your phone are kept here for a day.")
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
                        model: NotificationHistory
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
                      ? qsTr("Your last 50 clips with your phone, kept encrypted on this PC.")
                      : qsTr("Nothing here yet. Text and images copied between this PC and your phone show up here.")
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
        // False until this PC implements the action (shown as coming soon).
        property bool ready: true
        // A second, smaller action in the corner (e.g. "Send a folder").
        property string sideIcon
        property string sideLabel
        property bool sideAlwaysAvailable: false
        signal clicked
        signal sideClicked

        readonly property bool available: feature.state === "available" && ready

        height: 108
        activeFocusOnTab: available
        radius: Theme.radiusLg
        color: tile.active ? Theme.primary : Theme.tileColor
        border.width: Theme.focusVisible(tile) ? 2 : (Theme.graphite ? 1 : 0)
        border.color: Theme.focusVisible(tile) ? Theme.primary : Theme.outlineVariant
        scale: tap.pressed && available ? Theme.pressScale : 1
        Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        transform: Translate {
            y: hover.hovered && tile.available && !tap.pressed ? -Theme.hoverLift : 0
            Behavior on y { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        }

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
        Column {
            anchors.fill: parent
            anchors.margins: 14
            spacing: 12
            Rectangle {
                width: 36; height: 36
                radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
                color: tile.active ? Qt.rgba(1, 1, 1, 0.18) : (Theme.graphite ? "transparent" : Theme.secondaryContainer)
                border.width: Theme.graphite ? 1 : 0
                border.color: Theme.outlineVariant
                opacity: tile.available ? 1 : 0.55
                Icon {
                    anchors.centerIn: parent
                    path: tile.iconPath
                    color: tile.active ? Theme.primaryContent : Theme.secondaryContainerContent
                }
            }
            Column {
                width: parent.width
                spacing: 3
                Txt { width: parent.width; text: tile.title; role: "title"; size: 14; color: tile.ink; opacity: tile.available ? 1 : 0.6 }
                Txt { width: parent.width; visible: tile.available; text: tile.subtitle; role: "bodySmall"; color: tile.ink; opacity: tile.active ? 0.82 : 1.0 }
                // Not built yet, or capabilities unknown until the device has
                // connected once.
                Txt {
                    width: parent.width
                    visible: !tile.available && !lock.visible
                    text: tile.ready ? qsTr("When connected") : qsTr("Coming soon")
                    role: "bodySmall"
                    muted: true
                }
                LockChip { id: lock; visible: tile.ready && !tile.available && label.length > 0; feature: tile.feature }
            }
        }
        HoverHandler { id: hover; cursorShape: tile.available ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { id: tap; enabled: tile.available; onTapped: tile.clicked() }
        IconButton {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 8
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

        height: 52
        activeFocusOnTab: available

        Accessible.role: Accessible.CheckBox
        Accessible.name: label
        Accessible.checked: active
        Accessible.onPressAction: if (qbtn.available) qbtn.clicked()
        Keys.onPressed: (event) => {
            if (qbtn.available && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                qbtn.clicked()
                event.accepted = true
            }
        }

        readonly property color fillColor: {
            if (active && available)
                return Theme.graphite ? Theme.surfaceContent : Theme.primary
            if (active && !available)
                return Theme.graphite ? Theme.surfaceContainerHigh : Theme.secondaryContainer
            return Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
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
            border.width: Theme.focusVisible(qbtn) ? 2 : (Theme.graphite || (qbtn.active && !qbtn.available) ? 1 : 0)
            border.color: Theme.focusVisible(qbtn) ? Theme.primary : Theme.outlineVariant
            opacity: qbtn.available || qbtn.active ? 1 : 0.55
            scale: qtap.pressed && qbtn.available ? Theme.pressScale : 1
            Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }

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

        HoverHandler { id: qhover; cursorShape: qbtn.available ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler { id: qtap; enabled: qbtn.available; onTapped: qbtn.clicked() }
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
                    width: parent.width * barScope.fraction
                    height: parent.height
                    radius: parent.radius
                    color: slider.available
                           ? (Theme.graphite ? Theme.surfaceContent : Theme.primary)
                           : Theme.surfaceContentVariant
                }
            }

            Rectangle {
                visible: slider.available
                x: Math.max(0, Math.min(barScope.width - width, barScope.width * barScope.fraction - width / 2))
                anchors.verticalCenter: parent.verticalCenter
                width: 14; height: 14; radius: 7
                color: Theme.graphite ? Theme.surfaceContent : Theme.primary
                border.width: Theme.focusVisible(barScope) ? 2 : 0
                border.color: Theme.surface
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
}
