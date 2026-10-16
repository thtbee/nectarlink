// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Effects
import app.nectarlink

// The main window: navigation rail, title bar and pages. Before any device
// is paired it shows the welcome (pairing) screen instead.
NativeWindow {
    id: window

    readonly property bool showWalkthrough: AppController.status === "ready" && !Preferences.walkthroughCompleted
    readonly property bool welcome: AppController.status !== "ready" || showWalkthrough
    property string page: AppController.currentPage.length > 0 ? AppController.currentPage : "home"
    property string pendingSettingsSection: ""
    onPageChanged: if (AppController.currentPage !== page) AppController.currentPage = page
    Connections {
        target: AppController
        function onCurrentPageChanged() {
            if (AppController.currentPage.length > 0 && window.page !== AppController.currentPage)
                window.page = AppController.currentPage
        }
        function onHighlightTargetChanged() {
            const targetId = AppController.highlightTarget
            if (!targetId || targetId.length === 0)
                return
            if (targetId.indexOf("__size__:") === 0) {
                const dims = targetId.substring("__size__:".length).split("x")
                if (dims.length === 2) {
                    window.width = parseInt(dims[0], 10)
                    window.height = parseInt(dims[1], 10)
                }
                AppController.clearHighlight()
                return
            }
            if (targetId.indexOf("__grab__:") === 0) {
                const outPath = targetId.substring("__grab__:".length)
                AppController.clearHighlight()
                if (window.contentItem && outPath.length > 0) {
                    window.contentItem.grabToImage(function(result) {
                        result.saveToFile(outPath)
                    })
                }
                return
            }
            window.applyHighlightNavigation(targetId)
        }
    }
    Timer {
        id: highlightClearTimer
        interval: 2000
        onTriggered: AppController.clearHighlight()
    }
    function applyHighlightNavigation(targetId) {
        if (!targetId || targetId.length === 0)
            return
        highlightClearTimer.restart()
        if (targetId === "doctor:open") {
            AppController.runDoctor()
            return
        }
        if (targetId.indexOf("settings:") === 0) {
            const parts = targetId.split(":")
            const section = parts.length > 1 ? parts[1] : "connection"
            window.pendingSettingsSection = section
            window.page = "settings"
            if (pageLoader.item && "currentSection" in pageLoader.item) {
                pageLoader.item.currentSection = section
                window.pendingSettingsSection = ""
            }
            return
        }
        if (targetId.indexOf("messages:") === 0) {
            window.page = "messages"
            return
        }
        if (targetId.indexOf("calls:") === 0) {
            window.page = "calls"
            return
        }
        if (targetId.indexOf("photos:") === 0) {
            window.page = "photos"
            return
        }
        if (targetId.indexOf("deck:") === 0) {
            window.page = "deck"
            return
        }
        if (targetId.indexOf("apps:") === 0) {
            window.page = "apps"
            return
        }
        if (targetId.indexOf("timeline:") === 0) {
            window.page = "timeline"
            return
        }
        window.page = "home"
    }
    function navigateAndHighlight(targetId) {
        if (!targetId || targetId.length === 0)
            return
        AppController.triggerHighlight(targetId)
        window.applyHighlightNavigation(targetId)
    }
    readonly property int currentDeviceIndex: DeviceList.count > 0
        ? Math.min(Math.max(0, AppController.currentDevice), DeviceList.count - 1) : 0
    readonly property string currentDeviceId: DeviceList.count > 0
        ? DeviceList.deviceIdAt(currentDeviceIndex) : ""
    readonly property string currentDeviceName: DeviceList.count > 0
        ? DeviceList.deviceNameAt(currentDeviceIndex) : ""

    width: 1120
    height: 740
    minimumWidth: Theme.layout.minWindow[0]
    minimumHeight: Theme.layout.minWindow[1]
    title: "NectarLink"

    property bool resizing: resizeSettleTimer.running
    Timer {
        id: resizeSettleTimer
        interval: 140
    }
    onWidthChanged: if (visible) resizeSettleTimer.restart()
    onHeightChanged: if (visible) resizeSettleTimer.restart()

    // Mica/Acrylic is applied by Windows a moment after it's requested. Showing
    // through before it's there reveals the bare desktop (a flash), so the
    // window turns translucent only once the backdrop is in place, and opaque again
    // before it is removed.
    // Hive keeps its own fixed colors, so it never shows the wallpaper through.
    readonly property bool wantBackdrop: Qt.platform.os === "windows" && (Preferences.backdrop || Theme.glass) && !Theme.amoled && !Theme.hive
    property bool micaApplied: false
    property bool micaVisible: false
    backdrop: micaApplied
    acrylic: Theme.glass && !Theme.amoled
    Timer {
        id: micaSettle
        interval: 220
        onTriggered: window.micaVisible = window.wantBackdrop
    }
    Timer {
        id: micaRelease
        interval: Theme.fadeNormal + 60
        onTriggered: window.micaApplied = window.wantBackdrop
    }
    function followBackdrop() {
        if (window.wantBackdrop) {
            micaRelease.stop()
            micaApplied = true
            micaSettle.restart()
        } else {
            micaSettle.stop()
            micaVisible = false
            micaRelease.restart()
        }
    }
    onWantBackdropChanged: followBackdrop()
    Connections {
        target: Preferences
        function onBackdropChanged() { window.followBackdrop() }
    }
    Connections {
        target: window
        function onActiveChanged() {
            if (window.active) {
                AppController.refreshToastsEnabled()
                if (!window.activeFocusItem && window.contentItem)
                    window.contentItem.forceActiveFocus()
            }
        }
        function onActiveFocusItemChanged() {
            if (!window.activeFocusItem && window.contentItem)
                Qt.callLater(() => {
                    if (!window.activeFocusItem && window.contentItem)
                        window.contentItem.forceActiveFocus()
                })
        }
    }
    Component.onCompleted: {
        followBackdrop()
        if (!window.activeFocusItem && window.contentItem)
            window.contentItem.forceActiveFocus()
    }
    darkFrame: Theme.dark
    captionHeight: Theme.topBarHeight

    function openPairing() { pairingSheet.open() }

    // Window background: a tinted veil of the theme over Mica/Acrylic (so the
    // theme's seed color is rich and clear), opaque otherwise.
    readonly property color windowSurface: Theme.graphite || Theme.amoled || Theme.hive
        ? Theme.surface
        : Qt.tint(
            Theme.surface,
            Qt.rgba(
                Theme.primaryContainer.r,
                Theme.primaryContainer.g,
                Theme.primaryContainer.b,
                Theme.isPhoneSeed ? (Theme.dark ? 0.22 : 0.30) : (Theme.dark ? 0.12 : 0.18)
            )
        )
    Rectangle {
        anchors.fill: parent
        color: window.micaVisible && !Theme.amoled
            ? Qt.rgba(
                window.windowSurface.r,
                window.windowSurface.g,
                window.windowSurface.b,
                Theme.glass ? (Theme.dark ? 0.38 : 0.46) : (Theme.dark ? 0.76 : 0.84)
            )
            : window.windowSurface
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
    }

    // Hive: the living comb behind everything, growing from the logo. It
    // holds its frame while the window isn't in front or is minimized.
    HiveBackdrop {
        id: hiveComb
        anchors.fill: parent
        visible: opacity > 0
        opacity: Theme.hive ? 1 : 0
        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 420; easing.type: Easing.OutCubic }
        }
        animate: Theme.combMotion
        ownClock: false
        time: Theme.combTime
        cell: Theme.comb.cell
        frameMs: Theme.comb.frameMs
        breathSeconds: Theme.comb.breathSeconds
        pulseSeconds: Theme.comb.pulseSeconds
        lineColor: Theme.combLine
        glowColor: Theme.combGlow
        // The center of the logo mark in the rail; the comb and its pulses
        // carry across the whole window.
        origin: Qt.point(Theme.railWidth / 2, Theme.topBarHeight / 2)
        reach: 2.4
        pulseReach: 1.05
    }

    // The comb's clock runs while this window is in front and showing Hive.
    Binding {
        target: Theme
        property: "combClockRunning"
        value: window.active && window.visible
               && window.visibility !== Window.Minimized && window.visibility !== Window.Hidden
    }

    // Dynamic native GPU-blurred ambient layer when "Glass" theme mode is active.
    Item {
        id: artworkAmbient
        anchors.fill: parent
        clip: true
        opacity: Theme.ambientBlurActive ? 1.0 : 0.0
        visible: opacity > 0
        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 420; easing.type: Easing.OutCubic }
        }

        // Fixed-resolution 256x256 ambient mesh + blur so window resizing never
        // reallocates full-window GPU blur FBOs on every pixel.
        Item {
            id: glassColorMesh
            width: 256
            height: 256
            visible: false
            layer.enabled: glassMeshBlur.visible && window.visible && window.visibility !== Window.Minimized
            layer.textureSize: Qt.size(256, 256)
            layer.smooth: true

            Rectangle {
                width: 174; height: 174; radius: 87
                x: -31; y: -42
                color: Theme.primary
                opacity: Theme.dark ? 0.56 : 0.44
                Behavior on color { ColorAnimation { duration: 480 } }
            }
            Rectangle {
                width: 154; height: 154; radius: 77
                x: 123; y: 56
                color: Theme.primaryContainer
                opacity: Theme.dark ? 0.62 : 0.52
                Behavior on color { ColorAnimation { duration: 480 } }
            }
            Rectangle {
                width: 138; height: 138; radius: 69
                x: 31; y: 128
                color: Theme.secondaryContainer
                opacity: Theme.dark ? 0.54 : 0.46
                Behavior on color { ColorAnimation { duration: 480 } }
            }
            Rectangle {
                width: 108; height: 108; radius: 54
                x: 159; y: -15
                color: Qt.lighter(Theme.primary, 1.35)
                opacity: Theme.dark ? 0.34 : 0.28
                Behavior on color { ColorAnimation { duration: 480 } }
            }
        }

        MultiEffect {
            id: glassMeshBlur
            width: 256
            height: 256
            source: glassColorMesh
            visible: Theme.glass
            autoPaddingEnabled: false
            blurEnabled: true
            blurMax: 48
            blur: 1.0
            saturation: Theme.dark ? 0.42 : 0.30
            transform: Scale {
                xScale: artworkAmbient.width / 256
                yScale: artworkAmbient.height / 256
            }
        }

        // Translucent surface veil so UI cards and typography stay crisp over the blurred backdrop.
        Rectangle {
            anchors.fill: parent
            color: Qt.rgba(
                Theme.surface.r,
                Theme.surface.g,
                Theme.surface.b,
                Theme.glass ? (Theme.amoled ? 0.62 : (Theme.dark ? 0.22 : 0.28)) : (Theme.dark ? 0.60 : 0.66)
            )
            Behavior on color {
                enabled: !Theme.reduceMotion
                ColorAnimation { duration: 360; easing.type: Easing.OutCubic }
            }
        }
    }

    // ---- Navigation rail ----
    Rectangle {
        id: rail
        visible: !window.welcome
        width: Theme.railWidth
        height: parent.height
        color: "transparent"
        property string hoveredNav: ""

        Rectangle {
            id: mark
            anchors.horizontalCenter: parent.horizontalCenter
            // Centered on the top bar, level with the page title and the window buttons.
            y: (Theme.topBarHeight - height) / 2
            width: 38; height: 38
            radius: Theme.radiusSm
            color: Theme.graphite ? Theme.surfaceContent : Theme.primary
            Icon {
                anchors.centerIn: parent
                path: Icons.mark
                stroke: 2.4
                color: Theme.graphite ? Theme.surface : Theme.primaryContent
            }
        }
        Column {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: mark.bottom
            anchors.topMargin: 31
            spacing: 6
            NavItem {
                iconPath: Icons.home
                text: qsTr("Home")
                selected: window.page === "home" || window.page === "timeline"
                activeHover: rail.hoveredNav === "home"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "home"
                    else if (rail.hoveredNav === "home") rail.hoveredNav = ""
                }
                onClicked: window.page = "home"
            }
            NavItem {
                iconPath: Icons.messages
                text: qsTr("Messages")
                selected: window.page === "messages"
                activeHover: rail.hoveredNav === "messages"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "messages"
                    else if (rail.hoveredNav === "messages") rail.hoveredNav = ""
                }
                onClicked: window.page = "messages"
            }
            NavItem {
                iconPath: Icons.call
                text: qsTr("Calls")
                selected: window.page === "calls"
                activeHover: rail.hoveredNav === "calls"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "calls"
                    else if (rail.hoveredNav === "calls") rail.hoveredNav = ""
                }
                onClicked: window.page = "calls"
            }
            NavItem {
                iconPath: Icons.photo
                text: qsTr("Photos")
                selected: window.page === "photos"
                activeHover: rail.hoveredNav === "photos"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "photos"
                    else if (rail.hoveredNav === "photos") rail.hoveredNav = ""
                }
                onClicked: window.page = "photos"
            }
            NavItem {
                iconPath: Icons.apps
                text: qsTr("Apps")
                selected: window.page === "apps"
                activeHover: rail.hoveredNav === "apps"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "apps"
                    else if (rail.hoveredNav === "apps") rail.hoveredNav = ""
                }
                onClicked: window.page = "apps"
            }
            NavItem {
                iconPath: Icons.deck
                text: qsTr("Deck")
                selected: window.page === "deck"
                activeHover: rail.hoveredNav === "deck"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "deck"
                    else if (rail.hoveredNav === "deck") rail.hoveredNav = ""
                }
                onClicked: window.page = "deck"
            }
            NavItem {
                iconPath: Icons.book
                text: qsTr("Guide")
                selected: window.page === "guide"
                activeHover: rail.hoveredNav === "guide"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "guide"
                    else if (rail.hoveredNav === "guide") rail.hoveredNav = ""
                }
                onClicked: window.page = "guide"
            }
            NavItem {
                iconPath: Icons.settings
                text: qsTr("Settings")
                selected: window.page === "settings"
                activeHover: rail.hoveredNav === "settings"
                onHoveredChanged: {
                    if (hovered) rail.hoveredNav = "settings"
                    else if (rail.hoveredNav === "settings") rail.hoveredNav = ""
                }
                onClicked: window.page = "settings"
            }
        }
        IconButton {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 18
            iconPath: Icons.plus
            label: qsTr("Pair a new device")
            tonal: true
            onClicked: window.openPairing()
        }
    }

    // ---- Title bar and pages ----
    Item {
        id: main
        visible: !window.welcome
        anchors.left: rail.right
        anchors.right: parent.right
        height: parent.height

        Item {
            id: topBar
            width: parent.width - captionButtons.width
            height: Theme.topBarHeight
            IconButton {
                id: backToHome
                anchors.left: parent.left
                anchors.leftMargin: Theme.contentPadding - 6
                anchors.verticalCenter: parent.verticalCenter
                visible: window.page === "timeline"
                iconPath: Icons.back
                label: qsTr("Back to Home")
                onClicked: window.page = "home"
                Component.onCompleted: window.addCaptionHole(backToHome)
            }
            Txt {
                id: pageTitle
                anchors.left: backToHome.visible ? backToHome.right : parent.left
                anchors.leftMargin: backToHome.visible ? 6 : Theme.contentPadding
                anchors.verticalCenter: parent.verticalCenter
                text: window.page === "home" ? qsTr("Home")
                    : window.page === "timeline" ? qsTr("Timeline")
                    : window.page === "messages" ? qsTr("Messages")
                    : window.page === "calls" ? qsTr("Calls")
                    : window.page === "photos" ? qsTr("Photos")
                    : window.page === "apps" ? qsTr("Apps")
                    : window.page === "deck" ? qsTr("Deck")
                    : window.page === "guide" ? qsTr("Features guide")
                    : qsTr("Settings")
                role: "headline"
                size: 20
            }
            // Screen sharing: a live pill in the title bar, so the page never shifts. It sits
            // centered, and slides aside rather than overlap the title or the actions.
            Rectangle {
                id: sharingPill
                readonly property bool shown: AppController.pcMirrorActive
                readonly property real room: topBarActions.x - (pageTitle.x + pageTitle.width) - 32
                readonly property string fullText: AppController.pcMirrorDeviceName.length > 0
                    ? qsTr("Sharing screen with %1").arg(AppController.pcMirrorDeviceName)
                    : qsTr("Sharing screen")

                height: 34
                width: Math.max(0, Math.min(14 + 10 + 8 + sharingLabel.implicitWidth + 12 + stopSharing.width + 4, room))
                radius: height / 2
                x: Math.max(pageTitle.x + pageTitle.width + 16,
                            Math.min((parent.width - width) / 2, topBarActions.x - width - 16))
                anchors.verticalCenter: parent.verticalCenter
                visible: (shown || opacity > 0) && room > 120
                opacity: shown ? 1 : 0
                scale: shown ? 1 : 0.94
                color: Theme.errorContainer || Qt.rgba(Theme.error.r, Theme.error.g, Theme.error.b, 0.2)
                border.width: 1
                border.color: Qt.rgba(Theme.error.r, Theme.error.g, Theme.error.b, Theme.dark ? 0.45 : 0.30)
                Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
                Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 240; easing.type: Easing.OutBack } }

                Accessible.role: Accessible.StatusBar
                Accessible.name: fullText

                // Live dot with a slow halo, like a recording light.
                Item {
                    id: liveDot
                    width: 10; height: 10
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    Rectangle {
                        id: halo
                        anchors.centerIn: parent
                        width: 10; height: 10; radius: 5
                        color: Theme.error
                        opacity: 0
                        SequentialAnimation on opacity {
                            running: sharingPill.shown && !Theme.reduceMotion && Qt.application.state === Qt.ApplicationActive
                            loops: Animation.Infinite
                            NumberAnimation { from: 0.45; to: 0; duration: 1400; easing.type: Easing.OutCubic }
                        }
                        SequentialAnimation on scale {
                            running: sharingPill.shown && !Theme.reduceMotion && Qt.application.state === Qt.ApplicationActive
                            loops: Animation.Infinite
                            NumberAnimation { from: 1; to: 2.4; duration: 1400; easing.type: Easing.OutCubic }
                        }
                    }
                    Rectangle {
                        anchors.centerIn: parent
                        width: 8; height: 8; radius: 4
                        color: Theme.error
                    }
                }
                Txt {
                    id: sharingLabel
                    anchors.left: liveDot.right
                    anchors.leftMargin: 8
                    anchors.right: stopSharing.left
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: sharingPill.fullText
                    role: "label"
                    color: Theme.errorContainerContent || Theme.error
                    elide: Text.ElideRight
                }
                Button {
                    id: stopSharing
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    size: "sm"
                    variant: "tonal"
                    text: qsTr("Stop")
                    Accessible.name: qsTr("Stop sharing this PC's screen")
                    onClicked: AppController.stopPcMirror()
                    Component.onCompleted: window.addCaptionHole(stopSharing)
                }
            }
            Row {
                id: topBarActions
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 4

                IconButton {
                    id: paletteTopBtn
                    anchors.verticalCenter: parent.verticalCenter
                    iconPath: Icons.search
                    iconSize: 16
                    text: qsTr("Commands")
                    label: Preferences.commandPaletteHotkey.length > 0
                        ? qsTr("Command palette (%1)").arg(Preferences.commandPaletteHotkey)
                        : qsTr("Command palette (Ctrl+K)")
                    onClicked: AppController.openCommandPalette()
                    Component.onCompleted: window.addCaptionHole(paletteTopBtn)
                }
                IconButton {
                    id: timelineTopBtn
                    anchors.verticalCenter: parent.verticalCenter
                    visible: (window.page === "home" || window.page === "timeline") && AppController.hasDevices
                    iconPath: Icons.history
                    iconSize: 16
                    text: qsTr("Timeline")
                    tonal: window.page === "timeline"
                    label: qsTr("Files, clips, links, photos, recordings and sessions")
                    onClicked: window.page = window.page === "timeline" ? "home" : "timeline"
                    Component.onCompleted: window.addCaptionHole(timelineTopBtn)
                }
                // Reconnects and syncs with the phones (the title bar lets
                // clicks through here).
                IconButton {
                    id: refresh
                    anchors.verticalCenter: parent.verticalCenter
                    visible: window.page === "home" && AppController.hasDevices
                    iconPath: Icons.refresh
                    iconSize: 16
                    label: qsTr("Refresh")
                    onClicked: {
                        AppController.syncNow()
                        if (!Theme.reduceMotion) spin.restart()
                    }
                    RotationAnimation on rotation {
                        id: spin
                        running: false
                        from: 0; to: 360
                        duration: 700
                        easing.type: Easing.OutCubic
                    }
                    Component.onCompleted: window.addCaptionHole(refresh)
                }
            }
        }

        Rectangle {
            id: pages
            anchors.top: topBar.bottom
            anchors.bottom: parent.bottom
            width: parent.width
            clip: true
            Component.onCompleted: if ("topLeftRadius" in this) topLeftRadius = 18
            // Hive: lets the comb show faintly through, under opaque cards.
            color: Theme.hive
                ? Qt.rgba(Theme.surfaceContainerLow.r, Theme.surfaceContainerLow.g, Theme.surfaceContainerLow.b, Theme.dark ? 0.50 : 0.58)
                : Theme.amoled
                ? "#07080C"
                : (Theme.glass
                    ? Qt.rgba(Theme.surfaceContainerLow.r, Theme.surfaceContainerLow.g, Theme.surfaceContainerLow.b, Theme.dark ? 0.34 : 0.46)
                    : (Theme.graphite
                        ? Theme.surfaceContainerLow
                        : Qt.rgba(Theme.surfaceContainerLow.r, Theme.surfaceContainerLow.g, Theme.surfaceContainerLow.b, Theme.dark ? 0.56 : 0.68)))
            Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
            border.width: 0

            Loader {
                id: pageLoader
                anchors.fill: parent
                active: !window.welcome && AppController.status === "ready"
                property bool firstLoadDone: false
                transform: [
                    Translate { id: pageSlide; y: 0 },
                    Scale {
                        id: pageScale
                        origin.x: pageLoader.width / 2
                        origin.y: pageLoader.height / 3
                        xScale: 1.0
                        yScale: 1.0
                    }
                ]
                ParallelAnimation {
                    id: pageEnterAnim
                    NumberAnimation {
                        target: pageLoader
                        property: "opacity"
                        from: 0.35
                        to: 1.0
                        duration: 180
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        target: pageSlide
                        property: "y"
                        from: 10
                        to: 0
                        duration: 210
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        target: pageScale
                        properties: "xScale,yScale"
                        from: 0.985
                        to: 1.0
                        duration: 210
                        easing.type: Easing.OutCubic
                    }
                }
                source: {
                    if (!active) return ""
                    switch (window.page) {
                    case "timeline": return "qrc:/qt/qml/app/nectarlink/qml/pages/TimelinePage.qml"
                    case "messages": return "qrc:/qt/qml/app/nectarlink/qml/pages/MessagesPage.qml"
                    case "calls": return "qrc:/qt/qml/app/nectarlink/qml/pages/CallsPage.qml"
                    case "photos": return "qrc:/qt/qml/app/nectarlink/qml/pages/PhotosPage.qml"
                    case "apps": return "qrc:/qt/qml/app/nectarlink/qml/pages/AppsPage.qml"
                    case "deck": return "qrc:/qt/qml/app/nectarlink/qml/pages/DeckPage.qml"
                    case "guide": return "qrc:/qt/qml/app/nectarlink/qml/pages/FeaturesGuidePage.qml"
                    case "settings": return "qrc:/qt/qml/app/nectarlink/qml/pages/SettingsPage.qml"
                    default: return "qrc:/qt/qml/app/nectarlink/qml/pages/HomePage.qml"
                    }
                }
                onLoaded: {
                    if ("deviceId" in item)
                        item.deviceId = Qt.binding(() => window.currentDeviceId)
                    if ("deviceName" in item)
                        item.deviceName = Qt.binding(() => window.currentDeviceName)
                    item.active = true
                    if (window.pendingSettingsSection.length > 0 && "currentSection" in item) {
                        item.currentSection = window.pendingSettingsSection
                        window.pendingSettingsSection = ""
                    }
                    if (item.pairRequested)
                        item.pairRequested.connect(window.openPairing)
                    if (item.showMeRequested)
                        item.showMeRequested.connect(window.navigateAndHighlight)
                    if (item.restartWalkthroughRequested)
                        item.restartWalkthroughRequested.connect(() => Preferences.restartWalkthrough())
                    if (item.textRequested) {
                        item.textRequested.connect((number, name) => {
                            Messages.startChat(window.currentDeviceId, number, name)
                            window.page = "messages"
                        })
                    }
                    if (!pageLoader.firstLoadDone || Theme.reduceMotion) {
                        pageLoader.firstLoadDone = true
                        pageLoader.opacity = 1.0
                        pageSlide.y = 0
                        pageScale.xScale = 1.0
                        pageScale.yScale = 1.0
                    } else {
                        pageLoader.opacity = 0.35
                        pageSlide.y = 10
                        pageScale.xScale = 0.985
                        pageScale.yScale = 0.985
                        Qt.callLater(() => pageEnterAnim.restart())
                    }
                }
            }
        }
    }

    // ---- First run ----
    Loader {
        anchors.fill: parent
        active: AppController.status !== "ready"
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/pages/WelcomePage.qml" : ""
    }

    Loader {
        id: walkthroughLoader
        anchors.fill: parent
        active: window.showWalkthrough
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/pages/WalkthroughPage.qml" : ""
        onLoaded: {
            if ("deviceId" in item)
                item.deviceId = Qt.binding(() => window.currentDeviceId)
            if ("deviceName" in item)
                item.deviceName = Qt.binding(() => window.currentDeviceName)
            item.completed.connect(() => { window.page = "home" })
            item.openGuideRequested.connect(() => { window.page = "guide" })
            item.doctorRequested.connect(() => { AppController.runDoctor() })
        }
    }

    // ---- Overlays ----
    Connections {
        target: Pairing
        function onStateChanged() {
            if (Pairing.state === "comparing" && !window.welcome && !pairingSheet.opened) {
                pairingSheet.open()
            }
        }
    }
    Sheet {
        id: pairingSheet
        cardWidth: 480
        onOpenedChanged: if (!opened) Pairing.cancel()
        Loader {
            id: pairingLoader
            width: parent.width
            active: pairingSheet.opened
            source: active ? "qrc:/qt/qml/app/nectarlink/qml/pages/PairingPanel.qml" : ""
            onLoaded: {
                item.width = Qt.binding(() => pairingLoader.width)
                item.running = Qt.binding(() => pairingSheet.opened)
                item.finished.connect(() => pairingSheet.close())
                item.cancelled.connect(() => pairingSheet.close())
            }
        }
    }

    Sheet {
        id: ringingSheet
        opened: AppController.ringingFrom.length > 0
        dismissable: false
        cardWidth: 380
        Column {
            width: parent.width
            spacing: 16
            Avatar { anchors.horizontalCenter: parent.horizontalCenter; width: 56; height: 56; iconPath: Icons.ring; emphasized: true }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("%1 is ringing this PC").arg(AppController.ringingFrom)
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Stop ringing")
                onClicked: AppController.stopRinging()
            }
        }
    }

    Sheet {
        id: remotePromptSheet
        opened: AppController.remotePromptDeviceId.length > 0
        cardWidth: 420
        onOpenedChanged: {
            if (!opened && AppController.remotePromptDeviceId.length > 0)
                AppController.dismissRemotePrompt()
        }
        Column {
            width: parent.width
            spacing: 16
            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56; height: 56
                iconPath: Icons.phone
                emphasized: true
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Allow %1 to control this PC?").arg(AppController.remotePromptDeviceName)
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("%1 wants to use this PC's mouse and keyboard. You can change this in Settings.").arg(AppController.remotePromptDeviceName)
                role: "body"
                muted: true
                wrapMode: Text.WordWrap
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Not now")
                    onClicked: AppController.dismissRemotePrompt()
                }
                Button {
                    text: qsTr("Allow")
                    onClicked: AppController.allowRemoteInput(AppController.remotePromptDeviceId)
                }
            }
        }
    }

    Sheet {
        id: pcMirrorPromptSheet
        opened: AppController.pcMirrorPromptDeviceId.length > 0
        cardWidth: 460
        onOpenedChanged: {
            if (!opened && AppController.pcMirrorPromptDeviceId.length > 0)
                AppController.dismissPcMirrorPrompt()
        }
        Column {
            width: parent.width
            spacing: 16
            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56; height: 56
                iconPath: Icons.mirror
                emphasized: true
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Allow %1 to view this PC's screen?").arg(AppController.pcMirrorPromptDeviceName)
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("%1 wants to view this PC's screen. Control needs Remote mouse, keyboard & Deck in Settings.").arg(AppController.pcMirrorPromptDeviceName)
                role: "body"
                muted: true
                wrapMode: Text.WordWrap
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Not now")
                    onClicked: AppController.dismissPcMirrorPrompt()
                }
                Button {
                    variant: "tonal"
                    text: qsTr("Allow")
                    onClicked: AppController.allowPcMirrorOnce(AppController.pcMirrorPromptDeviceId)
                }
                Button {
                    text: qsTr("Allow for this device")
                    onClicked: AppController.allowPcMirrorDevice(AppController.pcMirrorPromptDeviceId)
                }
            }
        }
    }

    Sheet {
        id: continuityPasteSheet
        opened: AppController.continuityPastePromptVisible
        cardWidth: 440
        onOpenedChanged: {
            if (!opened && AppController.continuityPastePromptVisible)
                AppController.dismissContinuityPaste()
        }
        Column {
            width: parent.width
            spacing: 16
            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56; height: 56
                iconPath: Icons.camera
                emphasized: true
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: AppController.continuityPastePromptTitle.length > 0
                    ? AppController.continuityPastePromptTitle
                    : qsTr("Photo ready on clipboard")
                role: "headline"
                wrapMode: Text.WordWrap
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: AppController.continuityPastePromptBody
                role: "body"
                muted: true
                wrapMode: Text.WordWrap
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Keep on clipboard")
                    onClicked: AppController.dismissContinuityPaste()
                }
                Button {
                    text: qsTr("Paste into active window")
                    onClicked: AppController.confirmContinuityPaste()
                }
            }
        }
    }

    CaptionButtons {
        id: captionButtons
        window: window
        anchors.top: parent.top
        anchors.right: parent.right
    }

    Loader {
        id: doctorLoader
        anchors.fill: parent
        active: false
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/components/DoctorSheet.qml" : ""
        onLoaded: item.open()
    }
    Connections {
        target: AppController
        function onDoctorRequested() {
            if (doctorLoader.active && doctorLoader.item)
                doctorLoader.item.open()
            else
                doctorLoader.active = true
        }
    }

    Shortcut {
        sequences: ["Ctrl+K", "Ctrl+P"]
        onActivated: {
            if (AppController.commandPaletteOpen)
                AppController.closeCommandPalette()
            else
                AppController.openCommandPalette()
        }
    }

    Loader {
        id: commandPaletteLoader
        anchors.fill: parent
        active: AppController.commandPaletteOpen
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/components/CommandPalette.qml" : ""
    }

    Toast {
        id: toast
        onActionTriggered: AppController.runClipSuggestion()
    }
    Connections {
        target: AppController
        function onToast(message) { toast.show(message) }
        function onToastWithAction(message, actionLabel) { toast.showWithAction(message, actionLabel) }
        function onPendingDialChanged() {
            if (AppController.pendingDial.length > 0 && window.page !== "calls")
                window.page = "calls"
        }
    }
}
