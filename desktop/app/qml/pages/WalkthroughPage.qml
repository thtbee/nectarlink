// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Layouts
import app.nectarlink

// First-run setup walkthrough (7 steps): Welcome, Privacy, Install phone app,
// Pair, Choose what to share, Optional setup, and You are set.
FocusScope {
    id: walkthrough
    focus: true

    property bool active: true
    property string deviceId: ""
    property string deviceName: ""

    signal completed()
    signal openGuideRequested()
    signal doctorRequested()

    readonly property int totalSteps: 7
    // Matches the APK name on the GitHub release (see .github/workflows/release.yml).
    readonly property string adbCommand: "adb install -r Nectarlink-" + AppController.version + "-android.apk"
    property int step: Math.max(0, Math.min(totalSteps - 1, Preferences.walkthroughStep))
    property alias currentStep: walkthrough.step

    // Primary paired device (if any) for Step 5 per-device toggles.
    property string activeDeviceId: ""
    property string activeDeviceName: ""
    readonly property bool hasPairedDevice: AppController.hasDevices || activeDeviceId !== ""

    property real pairJoinProgress: hasPairedDevice ? 1.0 : 0.35
    Behavior on pairJoinProgress {
        enabled: !Theme.reduceMotion
        NumberAnimation { duration: 420; easing.type: Easing.OutCubic }
    }

    onStepChanged: {
        if (Preferences.walkthroughStep !== step)
            Preferences.walkthroughStep = step
        if (bodyFlick)
            bodyFlick.contentY = 0
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    onHasPairedDeviceChanged: {
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    onPairJoinProgressChanged: {
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    Connections {
        target: Preferences
        function onWalkthroughStepChanged() {
            let clamped = Math.max(0, Math.min(walkthrough.totalSteps - 1, Preferences.walkthroughStep))
            if (walkthrough.step !== clamped)
                walkthrough.step = clamped
        }
        function onStartWithWindowsChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    property var deviceTogglesMap: ({
        notifications: true,
        clipboard: true,
        files: true,
        sms: true,
        calls: true,
        media: true,
        storage: false,
        remote_input: false,
        pc_mirror: false
    })

    readonly property var defaultOnLabels: [
        "Notifications",
        "Clipboard",
        "Send files",
        "Now playing",
        "Messages and Calls",
        "Photos",
        "Find my phone"
    ]

    readonly property var optInLabels: [
        "Screen mirroring",
        "Remote input",
        "Share PC folders"
    ]

    readonly property var stepTitles: [
        "Welcome to NectarLink",
        "Your data stays on your devices",
        "Install NectarLink on your phone",
        "Pair your phone and PC",
        "Choose what to share",
        "Optional setup",
        "You are set"
    ]

    readonly property var stepSubtitles: [
        "Your Android phone and your PC, working together on your local network.",
        "No accounts, no cloud servers, and no telemetry.",
        "Get the Android app from GitHub Releases or install the APK over USB.",
        "Scan the QR code from the NectarLink Android app, or pick your phone from nearby devices.",
        "Safe features are on by default. Higher-trust access stays off until you turn it on.",
        "Three quick checks so notifications and discovery work reliably on your PC.",
        "Reopen it from Settings or the Command Palette."
    ]

    function refreshDeviceToggles() {
        let _rev = AppController.togglesRevision
        let count = DeviceList.count
        if (count <= 0) {
            activeDeviceId = ""
            activeDeviceName = ""
            return
        }
        let id = DeviceList.deviceIdAt(0) || ""
        let name = DeviceList.deviceNameAt(0) || "Your phone"
        activeDeviceId = id
        activeDeviceName = name
        if (id === "")
            return
        let list = AppController.deviceToggles(id)
        let next = {
            notifications: true,
            clipboard: true,
            files: true,
            sms: true,
            calls: true,
            media: true,
            storage: false,
            remote_input: false,
            pc_mirror: false
        }
        for (let i = 0; i < list.length; i++) {
            let entry = list[i]
            if (entry && entry.name !== undefined)
                next[entry.name] = Boolean(entry.on)
        }
        deviceTogglesMap = next
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    function isDeviceToggleOn(name) {
        return Boolean(deviceTogglesMap[name])
    }

    function setDeviceToggleValue(name, on) {
        let next = Object.assign({}, deviceTogglesMap)
        next[name] = on
        deviceTogglesMap = next
        if (activeDeviceId !== "")
            AppController.setDeviceToggle(activeDeviceId, name, on)
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    readonly property var parsedDoctorChecks: {
        let raw = AppController.doctorChecks
        if (!raw || raw === "")
            return []
        try {
            return JSON.parse(raw)
        } catch (e) {
            return []
        }
    }

    readonly property bool doctorAllPassed: {
        if (parsedDoctorChecks.length === 0)
            return false
        for (let i = 0; i < parsedDoctorChecks.length; i++) {
            if (parsedDoctorChecks[i].status !== "ok")
                return false
        }
        return true
    }

    readonly property string doctorSummaryLabel: {
        if (AppController.doctorBusy)
            return "Checking firewall and local network..."
        if (parsedDoctorChecks.length === 0)
            return "Run a quick check to verify Windows Firewall and local discovery."
        let problems = 0
        for (let i = 0; i < parsedDoctorChecks.length; i++) {
            if (parsedDoctorChecks[i].status !== "ok")
                problems++
        }
        return problems === 0
            ? "All network and firewall checks passed."
            : (problems + (problems === 1 ? " item needs attention." : " items need attention."))
    }

    function goNext() {
        if (step < totalSteps - 1) {
            step = step + 1
        } else {
            finishWalkthrough()
        }
    }

    function goBack() {
        if (step > 0)
            step = step - 1
    }

    function skipWalkthrough() {
        Preferences.walkthroughCompleted = true
        walkthrough.completed()
    }

    function finishWalkthrough() {
        Preferences.walkthroughCompleted = true
        walkthrough.completed()
    }

    function finishAndOpenGuide() {
        Preferences.walkthroughCompleted = true
        walkthrough.openGuideRequested()
    }

    Component.onCompleted: {
        refreshDeviceToggles()
        AppController.refreshToastsEnabled()
    }

    Connections {
        target: AppController
        function onHasDevicesChanged() { walkthrough.refreshDeviceToggles() }
        function onTogglesRevisionChanged() { walkthrough.refreshDeviceToggles() }
        function onToastsEnabledChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
        function onDoctorChecksChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    Connections {
        target: Theme
        function onDarkChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Escape) {
            skipWalkthrough()
            event.accepted = true
        } else if (event.key === Qt.Key_Right && (event.modifiers & Qt.AltModifier)) {
            goNext()
            event.accepted = true
        } else if (event.key === Qt.Key_Left && (event.modifiers & Qt.AltModifier)) {
            goBack()
            event.accepted = true
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 28
        anchors.rightMargin: 28
        anchors.topMargin: Theme.topBarHeight + 4
        anchors.bottomMargin: 20
        spacing: 16

        // Single top progress header: 7-segment progress bar + Skip setup action
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            RowLayout {
                id: progressTrack
                Layout.fillWidth: true
                spacing: 6
                Accessible.role: Accessible.ProgressBar
                Accessible.name: "Step " + (walkthrough.currentStep + 1) + " of " + walkthrough.totalSteps

                Repeater {
                    model: walkthrough.totalSteps
                    delegate: Rectangle {
                        required property int index
                        Layout.fillWidth: true
                        Layout.preferredHeight: 6
                        implicitWidth: 64
                        implicitHeight: 6
                        radius: 3
                        color: index < walkthrough.step
                            ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.55)
                            : (index === walkthrough.step ? Theme.primary : Theme.surfaceContainerHighest)

                        Behavior on color {
                            enabled: !Theme.reduceMotion
                            ColorAnimation { duration: Theme.fadeFast }
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: "Go to step " + (index + 1) + ": " + walkthrough.stepTitles[index]
                        MouseArea {
                            anchors.fill: parent
                            anchors.topMargin: -6
                            anchors.bottomMargin: -6
                            cursorShape: Qt.PointingHandCursor
                            onClicked: walkthrough.step = parent.index
                        }
                    }
                }
            }

            Button {
                text: qsTr("Skip setup")
                variant: "tonal"
                size: "sm"
                visible: walkthrough.step < walkthrough.totalSteps - 1
                Accessible.name: "Skip setup walkthrough"
                onClicked: walkthrough.skipWalkthrough()
            }
        }

        // Scrollable, vertically centered step content area
        Flickable {
            id: bodyFlick
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: width
            contentHeight: Math.max(height, contentColumn.implicitHeight + 16)
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: contentColumn
                width: Math.min(bodyFlick.width, 640)
                x: Math.floor((bodyFlick.width - width) / 2)
                y: Math.max(4, Math.floor((bodyFlick.height - implicitHeight) / 2))
                spacing: 12

                // Unified Honeycomb Cell Stage (fixed height across all 7 steps)
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 148
                    implicitWidth: 640
                    implicitHeight: 148
                    radius: Theme.radiusLg
                    color: Theme.surfaceContainerLow
                    border.width: 1
                    border.color: Theme.outlineVariant

                    Canvas {
                        id: honeycombCanvas
                        anchors.fill: parent
                        implicitWidth: 640
                        implicitHeight: 148

                        function drawHex(ctx, cx, cy, r) {
                            ctx.beginPath()
                            for (let i = 0; i < 6; i++) {
                                let angle = (Math.PI / 3) * i - Math.PI / 6
                                let px = cx + r * Math.cos(angle)
                                let py = cy + r * Math.sin(angle)
                                if (i === 0)
                                    ctx.moveTo(px, py)
                                else
                                    ctx.lineTo(px, py)
                            }
                            ctx.closePath()
                        }

                        function drawStyledHex(ctx, cx, cy, r, fillStyle, strokeStyle, lineWidth) {
                            drawHex(ctx, cx, cy, r)
                            if (fillStyle) {
                                ctx.fillStyle = fillStyle
                                ctx.fill()
                            }
                            if (strokeStyle) {
                                ctx.strokeStyle = strokeStyle
                                ctx.lineWidth = lineWidth || 1.5
                                ctx.stroke()
                            }
                        }

                        function drawPhoneGlyph(ctx, cx, cy, color) {
                            ctx.strokeStyle = color
                            ctx.lineWidth = 1.8
                            ctx.beginPath()
                            ctx.roundedRect(cx - 9, cy - 15, 18, 30, 3, 3)
                            ctx.stroke()
                            ctx.fillStyle = color
                            ctx.beginPath()
                            ctx.arc(cx, cy + 10, 1.4, 0, Math.PI * 2)
                            ctx.fill()
                        }

                        function drawPcGlyph(ctx, cx, cy, color) {
                            ctx.strokeStyle = color
                            ctx.lineWidth = 1.8
                            ctx.beginPath()
                            ctx.roundedRect(cx - 16, cy - 13, 32, 21, 3, 3)
                            ctx.stroke()
                            ctx.beginPath()
                            ctx.moveTo(cx - 7, cy + 13)
                            ctx.lineTo(cx + 7, cy + 13)
                            ctx.moveTo(cx, cy + 8)
                            ctx.lineTo(cx, cy + 13)
                            ctx.stroke()
                        }

                        function drawCheckGlyph(ctx, cx, cy, scale, color) {
                            ctx.strokeStyle = color
                            ctx.lineWidth = 2.2
                            ctx.lineCap = "round"
                            ctx.lineJoin = "round"
                            ctx.beginPath()
                            ctx.moveTo(cx - 6 * scale, cy + 0.5 * scale)
                            ctx.lineTo(cx - 1.5 * scale, cy + 5 * scale)
                            ctx.lineTo(cx + 6.5 * scale, cy - 4.5 * scale)
                            ctx.stroke()
                        }

                        onPaint: {
                            let ctx = getContext("2d")
                            ctx.reset()
                            let w = width
                            let h = height
                            let cy = h / 2
                            let clusterX = w * 0.24
                            let cx = w * 0.62

                            let primaryHex = Theme.primary.toString()
                            let primarySoft = Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.18).toString()
                            let primaryGlow = Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.32).toString()
                            let surfaceCell = Theme.surfaceContainerHigh.toString()
                            let outlineCell = Theme.outlineVariant.toString()
                            let mutedColor = Theme.surfaceContentVariant.toString()

                            // Left: 7-cell honeycomb cluster (6 outer ring + 1 center cell) filling as steps complete
                            let ringDist = 29
                            let cellR = 14
                            for (let s = 0; s < 7; s++) {
                                let hx = clusterX
                                let hy = cy
                                if (s < 6) {
                                    let ang = (Math.PI / 3) * s - Math.PI / 2
                                    hx = clusterX + ringDist * Math.cos(ang)
                                    hy = cy + ringDist * Math.sin(ang)
                                }
                                let isDone = s < walkthrough.step
                                let isCurr = s === walkthrough.step
                                drawStyledHex(
                                    ctx,
                                    hx,
                                    hy,
                                    isCurr ? cellR + 1.5 : cellR,
                                    isCurr ? primaryHex : (isDone ? primarySoft : surfaceCell),
                                    (isCurr || isDone) ? primaryHex : outlineCell,
                                    isCurr ? 2.0 : 1.4
                                )
                                if (isDone) {
                                    drawCheckGlyph(ctx, hx, hy, 0.55, primaryHex)
                                } else if (isCurr) {
                                    ctx.fillStyle = Theme.primaryContent.toString()
                                    ctx.beginPath()
                                    ctx.arc(hx, hy, 2.6, 0, Math.PI * 2)
                                    ctx.fill()
                                }
                            }

                            // Subtle divider line between 7-cell cluster and active step motif
                            ctx.strokeStyle = Qt.rgba(Theme.outlineVariant.r, Theme.outlineVariant.g, Theme.outlineVariant.b, 0.45).toString()
                            ctx.lineWidth = 1.0
                            ctx.beginPath()
                            ctx.moveTo(w * 0.40, cy - 38)
                            ctx.lineTo(w * 0.40, cy + 38)
                            ctx.stroke()

                            if (walkthrough.step === 0) {
                                // Step 0: Phone hex + glowing honeycomb bridge + PC hex
                                ctx.strokeStyle = primaryGlow
                                ctx.lineWidth = 2.0
                                ctx.beginPath()
                                ctx.moveTo(cx - 64, cy)
                                ctx.lineTo(cx + 64, cy)
                                ctx.stroke()

                                drawStyledHex(ctx, cx - 88, cy, 32, primarySoft, primaryHex, 2.0)
                                drawPhoneGlyph(ctx, cx - 88, cy, primaryHex)

                                drawStyledHex(ctx, cx, cy, 18, surfaceCell, primaryHex, 1.8)
                                drawStyledHex(ctx, cx, cy, 7, primaryHex, primaryHex, 1.0)

                                drawStyledHex(ctx, cx + 88, cy, 32, primarySoft, primaryHex, 2.0)
                                drawPcGlyph(ctx, cx + 88, cy, primaryHex)
                            } else if (walkthrough.step === 1) {
                                // Step 1: 3 interlocking security honeycomb cells (QUIC, DPAPI+Keystore, Local LAN)
                                let triad = [
                                    { x: cx - 54, y: cy + 6 },
                                    { x: cx, y: cy - 10 },
                                    { x: cx + 54, y: cy + 6 }
                                ]
                                ctx.strokeStyle = primaryGlow
                                ctx.lineWidth = 2.0
                                ctx.beginPath()
                                ctx.moveTo(triad[0].x, triad[0].y)
                                ctx.lineTo(triad[1].x, triad[1].y)
                                ctx.lineTo(triad[2].x, triad[2].y)
                                ctx.stroke()

                                for (let i = 0; i < triad.length; i++) {
                                    let isCenter = i === 1
                                    drawStyledHex(
                                        ctx,
                                        triad[i].x,
                                        triad[i].y,
                                        isCenter ? 31 : 26,
                                        isCenter ? primarySoft : surfaceCell,
                                        primaryHex,
                                        isCenter ? 2.2 : 1.6
                                    )
                                }
                                // Lock glyph in center cell
                                ctx.strokeStyle = primaryHex
                                ctx.lineWidth = 1.8
                                ctx.beginPath()
                                ctx.arc(cx, cy - 14, 6, Math.PI, 0, false)
                                ctx.stroke()
                                ctx.beginPath()
                                ctx.roundedRect(cx - 8, cy - 14, 16, 13, 2.5, 2.5)
                                ctx.stroke()
                                drawCheckGlyph(ctx, cx - 54, cy + 6, 0.85, primaryHex)
                                drawCheckGlyph(ctx, cx + 54, cy + 6, 0.85, primaryHex)
                            } else if (walkthrough.step === 2) {
                                // Step 2: APK package cell to Phone hex cell
                                ctx.strokeStyle = primaryGlow
                                ctx.lineWidth = 2.0
                                ctx.beginPath()
                                ctx.moveTo(cx - 52, cy)
                                ctx.lineTo(cx + 52, cy)
                                ctx.stroke()

                                drawStyledHex(ctx, cx - 68, cy, 28, surfaceCell, primaryHex, 1.8)
                                ctx.strokeStyle = primaryHex
                                ctx.lineWidth = 2.0
                                ctx.lineCap = "round"
                                ctx.beginPath()
                                ctx.moveTo(cx - 68, cy - 9)
                                ctx.lineTo(cx - 68, cy + 7)
                                ctx.moveTo(cx - 74, cy + 1)
                                ctx.lineTo(cx - 68, cy + 7)
                                ctx.lineTo(cx - 62, cy + 1)
                                ctx.stroke()

                                drawStyledHex(ctx, cx + 68, cy, 32, primarySoft, primaryHex, 2.2)
                                drawPhoneGlyph(ctx, cx + 68, cy, primaryHex)
                            } else if (walkthrough.step === 3) {
                                // Step 3: Pairing dual honeycomb cells joining together
                                let offset = 76 - (walkthrough.pairJoinProgress * 42)
                                ctx.strokeStyle = primaryGlow
                                ctx.lineWidth = 2.2
                                ctx.beginPath()
                                ctx.moveTo(cx - offset, cy)
                                ctx.lineTo(cx + offset, cy)
                                ctx.stroke()

                                drawStyledHex(ctx, cx - offset, cy, 32, primarySoft, primaryHex, 2.0)
                                drawPhoneGlyph(ctx, cx - offset, cy, primaryHex)

                                drawStyledHex(ctx, cx + offset, cy, 32, primarySoft, primaryHex, 2.0)
                                drawPcGlyph(ctx, cx + offset, cy, primaryHex)

                                if (walkthrough.hasPairedDevice) {
                                    drawStyledHex(ctx, cx, cy, 15, primaryHex, primaryHex, 1.5)
                                    drawCheckGlyph(ctx, cx, cy, 0.85, Theme.primaryContent.toString())
                                }
                            } else if (walkthrough.step === 4) {
                                // Step 4: 7 default-on amber hex cells + 3 opt-in outlined hex cells
                                let leftCluster = [
                                    [-102, -16], [-72, -16], [-42, -16],
                                    [-87, 13],   [-57, 13],  [-27, 13],
                                    [-117, 13]
                                ]
                                for (let i = 0; i < leftCluster.length; i++) {
                                    drawStyledHex(
                                        ctx,
                                        cx + leftCluster[i][0],
                                        cy + leftCluster[i][1],
                                        14,
                                        primarySoft,
                                        primaryHex,
                                        1.6
                                    )
                                }
                                let rightCluster = [
                                    [46, -13], [80, -13], [63, 15]
                                ]
                                let optKeys = ["pc_mirror", "remote_input", "storage"]
                                for (let j = 0; j < rightCluster.length; j++) {
                                    let activeOpt = walkthrough.hasPairedDevice && walkthrough.isDeviceToggleOn(optKeys[j])
                                    drawStyledHex(
                                        ctx,
                                        cx + rightCluster[j][0],
                                        cy + rightCluster[j][1],
                                        15,
                                        activeOpt ? primarySoft : surfaceCell,
                                        activeOpt ? primaryHex : outlineCell,
                                        1.6
                                    )
                                    ctx.fillStyle = activeOpt ? primaryHex : mutedColor
                                    ctx.beginPath()
                                    ctx.arc(cx + rightCluster[j][0], cy + rightCluster[j][1], 2.5, 0, Math.PI * 2)
                                    ctx.fill()
                                }
                            } else if (walkthrough.step === 5) {
                                // Step 5: 3 linked check hex cells (Toasts, Startup, Firewall)
                                let checks = [
                                    { x: cx - 72, y: cy, ok: AppController.toastsEnabled },
                                    { x: cx, y: cy, ok: Preferences.startWithWindows },
                                    { x: cx + 72, y: cy, ok: walkthrough.doctorAllPassed }
                                ]
                                ctx.strokeStyle = outlineCell
                                ctx.lineWidth = 1.8
                                ctx.beginPath()
                                ctx.moveTo(checks[0].x, checks[0].y)
                                ctx.lineTo(checks[2].x, checks[2].y)
                                ctx.stroke()

                                for (let k = 0; k < checks.length; k++) {
                                    let c = checks[k]
                                    drawStyledHex(
                                        ctx,
                                        c.x,
                                        c.y,
                                        27,
                                        c.ok ? primarySoft : surfaceCell,
                                        c.ok ? primaryHex : outlineCell,
                                        c.ok ? 2.0 : 1.5
                                    )
                                    drawCheckGlyph(ctx, c.x, c.y, 0.95, c.ok ? primaryHex : mutedColor)
                                }
                            } else {
                                // Step 6: Phone + PC linked into completed honeycomb check cell
                                ctx.strokeStyle = primaryHex
                                ctx.lineWidth = 2.2
                                ctx.beginPath()
                                ctx.moveTo(cx - 76, cy)
                                ctx.lineTo(cx + 76, cy)
                                ctx.stroke()

                                drawStyledHex(ctx, cx - 76, cy, 30, primarySoft, primaryHex, 2.0)
                                drawPhoneGlyph(ctx, cx - 76, cy, primaryHex)

                                drawStyledHex(ctx, cx, cy, 24, primaryHex, primaryHex, 2.0)
                                drawCheckGlyph(ctx, cx, cy, 1.05, Theme.primaryContent.toString())

                                drawStyledHex(ctx, cx + 76, cy, 30, primarySoft, primaryHex, 2.0)
                                drawPcGlyph(ctx, cx + 76, cy, primaryHex)
                            }
                        }
                    }
                }

                // Consistent Step Title and Subtitle block
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4

                    Txt {
                        Layout.fillWidth: true
                        text: walkthrough.stepTitles[walkthrough.step] || ""
                        role: "headline"
                        wrapMode: Text.WordWrap
                    }

                    Txt {
                        Layout.fillWidth: true
                        text: walkthrough.stepSubtitles[walkthrough.step] || ""
                        role: "body"
                        color: Theme.surfaceContentVariant
                        wrapMode: Text.WordWrap
                    }
                }

                // Single Unified Card per step
                Card {
                    Layout.fillWidth: true
                    padding: 18
                    implicitHeight: stepCardBody.implicitHeight + padding * 2

                    ColumnLayout {
                        id: stepCardBody
                        width: parent.width
                        height: implicitHeight
                        spacing: 14

                        // STEP 0: Welcome
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 0

                            GridLayout {
                                Layout.fillWidth: true
                                columns: width >= 480 ? 2 : 1
                                columnSpacing: 12
                                rowSpacing: 12

                                Repeater {
                                    model: [
                                        {
                                            icon: Icons.bell,
                                            title: qsTr("Notifications and Calls"),
                                            body: "Reply to notifications, read SMS threads, and answer calls from your PC."
                                        },
                                        {
                                            icon: Icons.send,
                                            title: qsTr("Clipboard and Send files"),
                                            body: "Copy text or images once, or drop files and folders at full Wi-Fi speed."
                                        },
                                        {
                                            icon: Icons.mirror,
                                            title: qsTr("Mirroring, Camera, and Audio"),
                                            body: "Stream your phone screen, use your phone as a webcam, or route PC audio."
                                        },
                                        {
                                            icon: Icons.lock,
                                            title: qsTr("Direct local QUIC link"),
                                            body: "Everything runs directly between your devices with zero cloud accounts."
                                        }
                                    ]

                                    delegate: Rectangle {
                                        required property var modelData
                                        Layout.fillWidth: true
                                        implicitWidth: 260
                                        implicitHeight: tileCol.implicitHeight + 24
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        RowLayout {
                                            anchors.fill: parent
                                            anchors.margins: 12
                                            spacing: 12

                                            Rectangle {
                                                Layout.preferredWidth: 34
                                                Layout.preferredHeight: 34
                                                implicitWidth: 34
                                                implicitHeight: 34
                                                Layout.alignment: Qt.AlignTop
                                                radius: 10
                                                color: Theme.primaryContainer

                                                Icon {
                                                    anchors.centerIn: parent
                                                    path: modelData.icon
                                                    width: 18
                                                    height: 18
                                                    color: Theme.primaryContainerContent
                                                }
                                            }

                                            ColumnLayout {
                                                id: tileCol
                                                Layout.fillWidth: true
                                                spacing: 3

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: modelData.title
                                                    role: "title"
                                                    wrapMode: Text.WordWrap
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: modelData.body
                                                    role: "caption"
                                                    color: Theme.surfaceContentVariant
                                                    wrapMode: Text.WordWrap
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: 40
                                radius: Theme.radiusSm
                                color: walkthrough.hasPairedDevice
                                    ? Qt.rgba(0.18, 0.72, 0.42, 0.12)
                                    : Theme.surfaceContainerLow
                                border.width: 1
                                border.color: walkthrough.hasPairedDevice
                                    ? Qt.rgba(0.18, 0.72, 0.42, 0.35)
                                    : Theme.outlineVariant

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 12
                                    anchors.rightMargin: 12
                                    spacing: 10

                                    Icon {
                                        path: walkthrough.hasPairedDevice ? Icons.check : Icons.phone
                                        width: 16
                                        height: 16
                                        color: walkthrough.hasPairedDevice ? Qt.rgba(0.22, 0.80, 0.48, 1.0) : Theme.primary
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: walkthrough.hasPairedDevice
                                            ? ("Paired with " + (walkthrough.activeDeviceName || "your phone") + ".")
                                            : "No phone paired yet. You can pair in Step 4 or skip setup anytime."
                                        role: "caption"
                                        color: Theme.surfaceContent
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }

                        // STEP 1: Privacy promise
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 1

                            Repeater {
                                model: [
                                    {
                                        icon: Icons.lock,
                                        title: qsTr("End-to-end encrypted over QUIC"),
                                        body: "All traffic between your phone and PC uses TLS 1.3 with device certificates pinned during pairing."
                                    },
                                    {
                                        icon: Icons.shield,
                                        title: qsTr("Hardware-backed device keys"),
                                        body: "Device keys are protected by Windows DPAPI and the Android Keystore."
                                    },
                                    {
                                        icon: Icons.wifi,
                                        title: qsTr("Local network by default"),
                                        body: "Pairing and transfers stay on your local network unless you turn on \"Connect over mobile data\", and automatic update checks can be turned off in Settings."
                                    }
                                ]

                                delegate: Rectangle {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    implicitWidth: 520
                                    implicitHeight: privacyRow.implicitHeight + 24
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant

                                    RowLayout {
                                        id: privacyRow
                                        anchors.fill: parent
                                        anchors.margins: 12
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            Layout.alignment: Qt.AlignTop
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: modelData.icon
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 3

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.title
                                                role: "title"
                                                wrapMode: Text.WordWrap
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.body
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 2: Install NectarLink on your phone
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 2

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: releaseCol.implicitHeight + 28
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: releaseCol
                                    anchors.fill: parent
                                    anchors.margins: 14
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 10

                                        Icon {
                                            path: Icons.download
                                            width: 18
                                            height: 18
                                            color: Theme.primary
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Option 1: Download the Android APK (Android 8.0+)")
                                            role: "title"
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: qsTr("Open the latest GitHub Release on your phone or copy the release link.")
                                        role: "caption"
                                        color: Theme.surfaceContentVariant
                                        wrapMode: Text.WordWrap
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 8

                                        Button {
                                            text: qsTr("Open release page")
                                            iconPath: Icons.openExternal
                                            variant: "tonal"
                                            size: "sm"
                                            Accessible.name: "Open Android release page in browser"
                                            onClicked: Qt.openUrlExternally("https://github.com/thtbee/nectarlink/releases/latest")
                                        }

                                        Button {
                                            id: copyUrlBtn
                                            property bool copied: false
                                            text: copied ? "Copied link" : "Copy link"
                                            iconPath: copied ? Icons.check : Icons.copy
                                            variant: "outline"
                                            size: "sm"
                                            Accessible.name: "Copy Android release URL"
                                            onClicked: {
                                                AppController.copyText("https://github.com/thtbee/nectarlink/releases/latest")
                                                copied = true
                                                resetCopyUrlTimer.restart()
                                            }
                                            Timer {
                                                id: resetCopyUrlTimer
                                                interval: 2000
                                                onTriggered: copyUrlBtn.copied = false
                                            }
                                        }

                                        Item { Layout.fillWidth: true }
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: adbCol.implicitHeight + 28
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: adbCol
                                    anchors.fill: parent
                                    anchors.margins: 14
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 10

                                        Icon {
                                            path: Icons.terminal
                                            width: 18
                                            height: 18
                                            color: Theme.primary
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Option 2: Install over USB with ADB")
                                            role: "title"
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 8

                                        Rectangle {
                                            Layout.fillWidth: true
                                            implicitWidth: 320
                                            implicitHeight: 34
                                            radius: Theme.radiusSm
                                            color: Theme.surfaceContainer
                                            border.width: 1
                                            border.color: Theme.outlineVariant

                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                anchors.left: parent.left
                                                anchors.right: parent.right
                                                anchors.leftMargin: 10
                                                anchors.rightMargin: 10
                                                text: walkthrough.adbCommand
                                                role: "code"
                                                size: 12
                                                elide: Text.ElideRight
                                            }
                                        }

                                        Button {
                                            id: copyAdbBtn
                                            property bool copied: false
                                            text: copied ? "Copied" : "Copy command"
                                            iconPath: copied ? Icons.check : Icons.copy
                                            variant: "tonal"
                                            size: "sm"
                                            Accessible.name: "Copy ADB install command"
                                            onClicked: {
                                                AppController.copyText(walkthrough.adbCommand)
                                                copied = true
                                                resetCopyAdbTimer.restart()
                                            }
                                            Timer {
                                                id: resetCopyAdbTimer
                                                interval: 2000
                                                onTriggered: copyAdbBtn.copied = false
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 3: Pair your phone and PC
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 3

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: 42
                                radius: Theme.radiusSm
                                visible: walkthrough.hasPairedDevice
                                color: Qt.rgba(0.18, 0.72, 0.42, 0.14)
                                border.width: 1
                                border.color: Qt.rgba(0.18, 0.72, 0.42, 0.40)

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 12
                                    anchors.rightMargin: 12
                                    spacing: 10

                                    Icon {
                                        path: Icons.check
                                        width: 16
                                        height: 16
                                        color: Qt.rgba(0.22, 0.80, 0.48, 1.0)
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: qsTr("Paired with %1. You can continue to the next step.")
                                            .arg(walkthrough.activeDeviceName || qsTr("your phone"))
                                        role: "caption"
                                        color: Theme.surfaceContent
                                        elide: Text.ElideRight
                                    }
                                }
                            }

                            PairingPanel {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                cancellable: false
                                compact: true
                                running: walkthrough.visible && walkthrough.step === 3
                            }
                        }

                        // STEP 4: Choose what to share
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 4

                            // B.4: Read-only two-column summary when no phone is paired yet
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 14
                                visible: !walkthrough.hasPairedDevice

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 480 ? 2 : 1
                                    columnSpacing: 16
                                    rowSpacing: 16

                                    // Left column: On by default
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: onDefaultCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: onDefaultCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: 8

                                                Rectangle {
                                                    Layout.preferredWidth: 8
                                                    Layout.preferredHeight: 8
                                                    implicitWidth: 8
                                                    implicitHeight: 8
                                                    radius: 4
                                                    color: Theme.primary
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: qsTr("On by default")
                                                    role: "title"
                                                }
                                            }

                                            Repeater {
                                                model: walkthrough.defaultOnLabels
                                                delegate: RowLayout {
                                                    required property string modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Icon {
                                                        path: Icons.check
                                                        width: 14
                                                        height: 14
                                                        color: Theme.primary
                                                    }

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData
                                                        role: "body"
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Right column: Off until you turn them on
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: optInCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: optInCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: 8

                                                Rectangle {
                                                    Layout.preferredWidth: 8
                                                    Layout.preferredHeight: 8
                                                    implicitWidth: 8
                                                    implicitHeight: 8
                                                    radius: 4
                                                    color: "transparent"
                                                    border.width: 1.5
                                                    border.color: Theme.surfaceContentVariant
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: qsTr("Off until you turn them on")
                                                    role: "title"
                                                }
                                            }

                                            Repeater {
                                                model: walkthrough.optInLabels
                                                delegate: RowLayout {
                                                    required property string modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Icon {
                                                        path: Icons.lock
                                                        width: 14
                                                        height: 14
                                                        color: Theme.surfaceContentVariant
                                                    }

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData
                                                        role: "body"
                                                        color: Theme.surfaceContentVariant
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                Txt {
                                    Layout.fillWidth: true
                                    text: qsTr("Change these per phone on Home after pairing.")
                                    role: "caption"
                                    color: Theme.surfaceContentVariant
                                }
                            }

                            // Interactive toggles when a phone IS paired
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 14
                                visible: walkthrough.hasPairedDevice

                                Txt {
                                    Layout.fillWidth: true
                                    text: qsTr("Sharing settings for %1").arg(walkthrough.activeDeviceName || qsTr("your phone"))
                                    role: "label"
                                    color: Theme.primary
                                }

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 500 ? 2 : 1
                                    columnSpacing: 16
                                    rowSpacing: 16

                                    // Left column: On by default switches
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: pairedDefaultCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: pairedDefaultCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("On by default")
                                                role: "title"
                                            }

                                            Repeater {
                                                model: [
                                                    { key: "notifications", label: qsTr("Notifications") },
                                                    { key: "clipboard", label: qsTr("Clipboard") },
                                                    { key: "files", label: qsTr("Send files") },
                                                    { key: "media", label: qsTr("Now playing") },
                                                    { key: "sms", label: qsTr("Messages") },
                                                    { key: "calls", label: qsTr("Calls") }
                                                ]

                                                delegate: RowLayout {
                                                    required property var modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData.label
                                                        role: "body"
                                                    }

                                                    Toggle {
                                                        label: modelData.label
                                                        checked: walkthrough.isDeviceToggleOn(modelData.key)
                                                        onToggled: next => walkthrough.setDeviceToggleValue(modelData.key, next)
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Right column: Opt-in switches
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: pairedOptInCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: pairedOptInCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("Off until you turn them on")
                                                role: "title"
                                            }

                                            Repeater {
                                                model: [
                                                    { key: "pc_mirror", label: qsTr("Screen mirroring") },
                                                    { key: "remote_input", label: qsTr("Remote input") },
                                                    { key: "storage", label: qsTr("Share PC folders") }
                                                ]

                                                delegate: RowLayout {
                                                    required property var modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData.label
                                                        role: "body"
                                                    }

                                                    Toggle {
                                                        label: modelData.label
                                                        checked: walkthrough.isDeviceToggleOn(modelData.key)
                                                        onToggled: next => walkthrough.setDeviceToggleValue(modelData.key, next)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 5: Optional setup
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 5

                            // 1. Windows notifications
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: toastRow.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                RowLayout {
                                    id: toastRow
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 12

                                    Rectangle {
                                        Layout.preferredWidth: 34
                                        Layout.preferredHeight: 34
                                        implicitWidth: 34
                                        implicitHeight: 34
                                        radius: 10
                                        color: Theme.primaryContainer

                                        Icon {
                                            anchors.centerIn: parent
                                            path: Icons.bell
                                            width: 18
                                            height: 18
                                            color: Theme.primaryContainerContent
                                        }
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 2

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Windows notifications")
                                            role: "title"
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: AppController.toastsEnabled
                                                ? "Enabled in Windows Settings."
                                                : "Notifications are off for NectarLink in Windows Settings."
                                            role: "caption"
                                            color: Theme.surfaceContentVariant
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    RowLayout {
                                        spacing: 8

                                        Button {
                                            text: qsTr("Windows Settings")
                                            variant: "tonal"
                                            size: "sm"
                                            visible: !AppController.toastsEnabled
                                            Accessible.name: "Open Windows notification settings"
                                            onClicked: Qt.openUrlExternally("ms-settings:notifications")
                                        }

                                        Button {
                                            text: qsTr("Recheck")
                                            iconPath: Icons.refresh
                                            variant: "text"
                                            size: "sm"
                                            Accessible.name: "Recheck Windows notification status"
                                            onClicked: AppController.refreshToastsEnabled()
                                        }
                                    }
                                }
                            }

                            // 2. Start with Windows
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: startupRow.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                RowLayout {
                                    id: startupRow
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 12

                                    Rectangle {
                                        Layout.preferredWidth: 34
                                        Layout.preferredHeight: 34
                                        implicitWidth: 34
                                        implicitHeight: 34
                                        radius: 10
                                        color: Theme.primaryContainer

                                        Icon {
                                            anchors.centerIn: parent
                                            path: Icons.power
                                            width: 18
                                            height: 18
                                            color: Theme.primaryContainerContent
                                        }
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 2

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Start with Windows")
                                            role: "title"
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Keep NectarLink ready in the system tray when you sign in.")
                                            role: "caption"
                                            color: Theme.surfaceContentVariant
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    Toggle {
                                        label: qsTr("Start with Windows")
                                        checked: Preferences.startWithWindows
                                        onToggled: next => Preferences.startWithWindows = next
                                    }
                                }
                            }

                            // 3. Firewall and network check
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: firewallCol.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: firewallCol
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: Icons.bolt
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("Firewall and network check")
                                                role: "title"
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: walkthrough.doctorSummaryLabel
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }

                                        Button {
                                            text: AppController.doctorBusy ? "Checking..." : "Connection Doctor"
                                            iconPath: Icons.bolt
                                            variant: "tonal"
                                            size: "sm"
                                            enabled: !AppController.doctorBusy
                                            Accessible.name: "Run firewall and network check"
                                            onClicked: {
                                                AppController.runDoctor()
                                                walkthrough.doctorRequested()
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 6: You are set (no duplicate action buttons inside card)
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 6

                            Repeater {
                                model: [
                                    {
                                        icon: Icons.phone,
                                        title: walkthrough.hasPairedDevice
                                            ? ("Paired with " + (walkthrough.activeDeviceName || "your phone"))
                                            : "Pair a phone anytime from Home",
                                        body: walkthrough.hasPairedDevice
                                            ? "Your phone is linked and ready on your local network."
                                            : "Open the NectarLink Android app whenever you are ready to scan the QR code."
                                    },
                                    {
                                        icon: Icons.search,
                                        title: qsTr("Command Palette (Ctrl+K)"),
                                        body: "Press Ctrl+K from anywhere in the app to jump to features, send files, or run actions."
                                    },
                                    {
                                        icon: Icons.book,
                                        title: qsTr("Features guide"),
                                        body: "Browse every feature in NectarLink, see what permissions it needs, and jump straight to it."
                                    }
                                ]

                                delegate: Rectangle {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    implicitWidth: 520
                                    implicitHeight: doneRow.implicitHeight + 24
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant

                                    RowLayout {
                                        id: doneRow
                                        anchors.fill: parent
                                        anchors.margins: 12
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            Layout.alignment: Qt.AlignTop
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: modelData.icon
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.title
                                                role: "title"
                                                wrapMode: Text.WordWrap
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.body
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Bottom navigation footer: Back on left, Next or Open features guide + Done on right
        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            Button {
                text: qsTr("Back")
                iconPath: Icons.back
                variant: "tonal"
                size: "md"
                visible: walkthrough.step > 0
                Accessible.name: "Back to previous step"
                onClicked: walkthrough.goBack()
            }

            Item { Layout.fillWidth: true }

            Button {
                text: qsTr("Open features guide")
                iconPath: Icons.book
                variant: "tonal"
                size: "md"
                visible: walkthrough.step === walkthrough.totalSteps - 1
                Accessible.name: "Finish setup and open Features guide"
                onClicked: walkthrough.finishAndOpenGuide()
            }

            Button {
                text: walkthrough.step === walkthrough.totalSteps - 1
                    ? "Done"
                    : (walkthrough.step === 0 ? "Get started" : "Next")
                iconPath: walkthrough.step === walkthrough.totalSteps - 1 ? Icons.check : Icons.chevronRight
                variant: "fill"
                size: "md"
                Accessible.name: walkthrough.step === walkthrough.totalSteps - 1 ? "Finish walkthrough" : "Next step"
                onClicked: walkthrough.goNext()
            }
        }
    }
}
