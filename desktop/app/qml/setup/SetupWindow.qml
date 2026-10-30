// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// The installer's and the uninstaller's window (src/setup.rs). One window,
// three moments: welcome, working, done. Its own fixed colors (honey on warm
// paper, or on charcoal), following the system's light or dark mode.
NativeWindow {
    id: window

    readonly property bool dark: Setup.dark
    readonly property bool removing: Setup.mode === "remove"
    readonly property bool animate: !Setup.reduceMotion
    readonly property string phase: Setup.phase

    readonly property color bg: dark ? "#121214" : "#FFF8F3"
    readonly property color ink: dark ? "#EDE6DD" : "#1F1B16"
    readonly property color muted: dark ? "#A39A90" : "#6B5E52"
    readonly property color honey: dark ? "#FFB95F" : "#8A5100"
    readonly property color onHoney: dark ? "#2C1600" : "#FFFFFF"
    readonly property color track: dark ? Qt.rgba(1, 1, 1, 0.08) : Qt.rgba(0.12, 0.10, 0.08, 0.08)
    readonly property color danger: dark ? "#F0A39C" : "#9E2B25"
    readonly property color panel: dark ? "#1C1B1D" : "#FFFFFF"
    // Removing: the comb goes grey and dim.
    readonly property color comb: removing
        ? (dark ? Qt.rgba(1, 1, 1, 0.09) : Qt.rgba(0.30, 0.26, 0.22, 0.06))
        : (dark ? Qt.rgba(1.0, 0.73, 0.37, 0.22) : Qt.rgba(0.54, 0.32, 0.0, 0.085))
    readonly property color combGlow: removing
        ? (dark ? Qt.rgba(0.55, 0.52, 0.48, 0.55) : Qt.rgba(0.72, 0.68, 0.62, 0.55))
        : (dark ? Qt.rgba(1.0, 0.62, 0.18, 0.75) : Qt.rgba(1.0, 0.72, 0.37, 0.80))

    // Not Theme's: Theme reads the app's settings and data, which setup and
    // the uninstaller (which deletes them) must leave alone.
    readonly property string captionIconFont: Qt.fontFamilies().indexOf("Segoe Fluent Icons") >= 0
        ? "Segoe Fluent Icons" : "Segoe MDL2 Assets"

    readonly property string action: removing ? "remove"
        : Setup.kind === "update" ? "update"
        : Setup.kind === "reinstall" ? "reinstall"
        : "install"

    width: 600
    height: 480
    minimumWidth: width
    maximumWidth: width
    minimumHeight: height
    maximumHeight: height
    visible: true
    title: removing ? qsTr("Uninstall NectarLink") : qsTr("NectarLink Setup")
    color: bg
    backdrop: false
    darkFrame: dark
    captionHeight: 48

    // Closing is ignored while setup works; otherwise it ends the program.
    onCloseRequested: {
        if (window.phase === "working")
            return
        Setup.finish()
        Qt.quit()
    }

    function primary() {
        if (window.phase === "welcome" || window.phase === "failed")
            Setup.start()
        else if (window.phase === "done") {
            if (!window.removing)
                Setup.openApp()
            window.close()
        }
    }

    Item {
        anchors.fill: parent
        focus: true
        Keys.onEscapePressed: {
            if (disclosure.open)
                disclosure.open = false
            else if (window.phase !== "working")
                window.close()
        }
        Keys.onReturnPressed: if (!disclosure.open && window.phase !== "working") window.primary()
        Keys.onEnterPressed: if (!disclosure.open && window.phase !== "working") window.primary()

        HiveBackdrop {
            anchors.fill: parent
            animate: window.animate
            lineColor: window.comb
            glowColor: window.combGlow
            strength: window.removing ? 0.7 : 1
            origin: Qt.point(width / 2, window.phase === "done" && !window.removing ? 40 : 112)
        }

        // There is no title bar, so the window moves from anywhere that
        // isn't a button or a link.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            onPressed: window.startSystemMove()
        }

        // Close, in the corner.
        Item {
            id: closeButton
            anchors.top: parent.top
            anchors.right: parent.right
            width: 46
            height: 34
            visible: window.phase !== "working"
            Component.onCompleted: window.addCaptionHole(closeButton)
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("Close")
            Rectangle {
                anchors.fill: parent
                color: "#C42B1C"
                opacity: closeArea.containsMouse ? 1 : 0
            }
            Text {
                anchors.centerIn: parent
                visible: Qt.platform.os === "windows"
                text: "\ue8bb"
                font.family: window.captionIconFont
                font.pixelSize: 10
                color: closeArea.containsMouse ? "#FFFFFF" : window.ink
            }
            Shape {
                anchors.centerIn: parent
                visible: Qt.platform.os !== "windows"
                width: 10
                height: 10
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: closeArea.containsMouse ? "#FFFFFF" : window.ink
                    strokeWidth: 1.25
                    fillColor: "transparent"
                    capStyle: ShapePath.RoundCap
                    joinStyle: ShapePath.RoundJoin
                    PathSvg { path: "M 1 1 L 9 9 M 9 1 L 1 9" }
                }
            }
            MouseArea {
                id: closeArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: window.close()
            }
        }

        // The logo (installing) or the bee (removing), breathing while busy.
        Item {
            id: emblem
            width: 104
            height: 104
            anchors.horizontalCenter: parent.horizontalCenter
            y: 60
            opacity: window.phase === "done" && !window.removing ? 0 : 1
            visible: opacity > 0
            Behavior on opacity { NumberAnimation { duration: 260 } }

            property real breath: 0
            SequentialAnimation on breath {
                running: window.animate && window.phase === "working" && !window.removing
                loops: Animation.Infinite
                NumberAnimation { from: 0; to: 1; duration: 1500; easing.type: Easing.InOutSine }
                NumberAnimation { from: 1; to: 0; duration: 1500; easing.type: Easing.InOutSine }
            }

            Image {
                anchors.centerIn: parent
                width: 96
                height: 96
                visible: !window.removing
                source: Setup.iconUrl
                sourceSize: Qt.size(192, 192)
                smooth: true
                mipmap: true
                scale: 1 + emblem.breath * 0.04
                Accessible.role: Accessible.Graphic
                Accessible.name: qsTr("NectarLink")
            }
            SadBee {
                anchors.centerIn: parent
                width: 150
                height: 128
                visible: window.removing
                animate: window.animate
                dark: window.dark
            }
        }

        // ---- Welcome, working and failed ----
        Column {
            id: main
            anchors.horizontalCenter: parent.horizontalCenter
            y: 190
            width: 440
            spacing: 0
            visible: window.phase !== "done"

            Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                font.family: "Figtree"
                font.pixelSize: 28
                font.weight: Font.DemiBold
                color: window.ink
                text: window.phase === "failed"
                    ? (window.removing ? qsTr("Couldn't uninstall NectarLink") : qsTr("Setup couldn't finish"))
                    : window.phase === "working"
                    ? (window.action === "remove" ? qsTr("Uninstalling NectarLink")
                       : window.action === "update" ? qsTr("Updating NectarLink")
                       : qsTr("Installing NectarLink"))
                    : window.removing ? qsTr("Uninstall NectarLink?")
                    : "NectarLink"
            }
            Item { width: 1; height: 10 }
            Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                lineHeight: 1.25
                font.family: "Figtree"
                font.pixelSize: 15
                color: window.phase === "failed" ? window.danger : window.muted
                text: window.phase === "failed" ? Setup.message
                    : window.phase === "working" ? Setup.step
                    : window.removing
                    ? qsTr("Sorry to see you go. This uninstalls NectarLink and deletes its settings and pairings on this PC. Files you received stay in %1.").arg("Downloads\\Nectarlink")
                    : Setup.kind === "update"
                    ? qsTr("Update from %1 to %2. Your settings and pairings stay as they are.").arg(Setup.installedVersion).arg(Setup.version)
                    : Setup.kind === "reinstall"
                    ? qsTr("Version %1 is already installed. Install it again to repair it.").arg(Setup.version)
                    : Setup.kind === "older"
                    ? qsTr("This PC has a newer version (%1). Installing %2 replaces it.").arg(Setup.installedVersion).arg(Setup.version)
                    : qsTr("Your Android phone and your PC, working as one.")
            }

            // Progress.
            Item { width: 1; height: window.phase === "working" ? 26 : 0 }
            Rectangle {
                visible: window.phase === "working"
                anchors.horizontalCenter: parent.horizontalCenter
                width: 300
                height: 6
                radius: 3
                color: window.track
                Rectangle {
                    width: parent.width * Setup.progress
                    height: parent.height
                    radius: parent.radius
                    color: window.removing ? window.muted : window.honey
                    Behavior on width { NumberAnimation { duration: 700; easing.type: Easing.OutCubic } }
                }
                Accessible.role: Accessible.ProgressBar
                Accessible.name: Setup.step
            }

            // Why it went back to the start (permission declined).
            Item { width: 1; height: window.phase === "welcome" && Setup.message.length > 0 ? 14 : 0 }
            Text {
                visible: window.phase === "welcome" && Setup.message.length > 0
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                font.family: "Figtree"
                font.pixelSize: 13
                color: window.danger
                text: Setup.message
            }

            Item { width: 1; height: window.phase === "working" ? 0 : 30 }
            Row {
                visible: window.phase !== "working"
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 12
                SetupButton {
                    text: window.phase === "failed" ? qsTr("Try again")
                        : window.action === "remove" ? qsTr("Uninstall")
                        : window.action === "update" ? qsTr("Update")
                        : window.action === "reinstall" ? qsTr("Reinstall")
                        : qsTr("Install")
                    fill: window.removing ? window.ink : window.honey
                    ink: window.removing ? window.bg : window.onHoney
                    focusColor: window.honey
                    onClicked: window.primary()
                }
                SetupButton {
                    visible: window.removing || window.phase === "failed"
                    filled: false
                    text: window.removing && window.phase !== "failed" ? qsTr("Keep NectarLink") : qsTr("Close")
                    ink: window.ink
                    focusColor: window.honey
                    onClicked: window.close()
                }
            }

            // Whether setup adds a desktop shortcut (Windows; #102).
            Item { width: 1; height: shortcutChoice.visible ? 18 : 0 }
            Item {
                id: shortcutChoice
                visible: !window.removing && window.phase !== "working" && Qt.platform.os === "windows"
                anchors.horizontalCenter: parent.horizontalCenter
                width: shortcutRow.implicitWidth
                height: visible ? 24 : 0
                activeFocusOnTab: visible
                Accessible.role: Accessible.CheckBox
                Accessible.name: shortcutLabel.text
                Accessible.checkable: true
                Accessible.checked: Setup.desktopShortcut
                Accessible.onToggleAction: Setup.desktopShortcut = !Setup.desktopShortcut
                Accessible.onPressAction: Setup.desktopShortcut = !Setup.desktopShortcut
                Keys.onSpacePressed: Setup.desktopShortcut = !Setup.desktopShortcut

                Row {
                    id: shortcutRow
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 10
                    Rectangle {
                        width: 18
                        height: 18
                        radius: 5
                        anchors.verticalCenter: parent.verticalCenter
                        color: Setup.desktopShortcut ? window.honey : "transparent"
                        border.width: Setup.desktopShortcut ? 0 : 1.5
                        border.color: window.muted
                        Shape {
                            anchors.centerIn: parent
                            width: 10
                            height: 8
                            visible: Setup.desktopShortcut
                            preferredRendererType: Shape.CurveRenderer
                            ShapePath {
                                strokeColor: window.onHoney
                                strokeWidth: 2
                                fillColor: "transparent"
                                capStyle: ShapePath.RoundCap
                                joinStyle: ShapePath.RoundJoin
                                PathSvg { path: "M 1 4 L 4 7 L 9 1" }
                            }
                        }
                    }
                    Text {
                        id: shortcutLabel
                        anchors.verticalCenter: parent.verticalCenter
                        font.family: "Figtree"
                        font.pixelSize: 13
                        color: window.ink
                        text: qsTr("Add a desktop shortcut")
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -5
                    radius: 8
                    color: "transparent"
                    border.width: 2
                    border.color: window.honey
                    visible: shortcutChoice.activeFocus
                }
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -6
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Setup.desktopShortcut = !Setup.desktopShortcut
                }
            }

            Item { width: 1; height: shortcutChoice.visible ? 14 : 18 }
            Text {
                id: details
                visible: window.phase === "welcome" && !window.removing
                anchors.horizontalCenter: parent.horizontalCenter
                font.family: "Figtree"
                font.pixelSize: 13
                font.underline: detailsArea.containsMouse
                color: window.muted
                text: qsTr("What this installs")
                Accessible.role: Accessible.Link
                Accessible.name: text
                Accessible.onPressAction: disclosure.open = true
                MouseArea {
                    id: detailsArea
                    anchors.fill: parent
                    anchors.margins: -6
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: disclosure.open = true
                }
            }
        }

        // ---- Done ----
        Column {
            id: done
            anchors.horizontalCenter: parent.horizontalCenter
            y: window.removing ? 190 : 56
            width: 440
            spacing: 0
            visible: window.phase === "done"
            opacity: visible ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 320 } }

            Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                font.family: "Figtree"
                font.pixelSize: 28
                font.weight: Font.DemiBold
                color: window.ink
                text: window.removing ? qsTr("NectarLink is uninstalled") : qsTr("NectarLink is ready")
            }
            Item { width: 1; height: 8 }
            Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                lineHeight: 1.25
                font.family: "Figtree"
                font.pixelSize: 15
                color: window.muted
                text: window.removing
                    ? qsTr("Your received files are still in %1. Thanks for giving it a try.").arg("Downloads\\Nectarlink")
                    : qsTr("Next, get the Android app. Scan this code with your phone's camera to open its download page.")
            }

            Item { width: 1; height: window.removing ? 0 : 20 }
            Rectangle {
                visible: !window.removing
                anchors.horizontalCenter: parent.horizontalCenter
                width: 148
                height: 148
                radius: 18
                color: "#FFFFFF"
                border.width: window.dark ? 0 : 1
                border.color: Qt.rgba(0.12, 0.10, 0.08, 0.10)
                Image {
                    anchors.centerIn: parent
                    width: 124
                    height: 124
                    source: Setup.qrUrl
                    sourceSize: Qt.size(248, 248)
                    smooth: false
                    Accessible.role: Accessible.Graphic
                    Accessible.name: qsTr("QR code for the Android app download page")
                }
            }
            Item { width: 1; height: window.removing ? 0 : 12 }
            Text {
                visible: !window.removing
                anchors.horizontalCenter: parent.horizontalCenter
                font.family: "Figtree"
                font.pixelSize: 13
                font.underline: true
                color: window.honey
                text: "github.com/alejandrokundrah/nectarlink/releases/latest"
                Accessible.role: Accessible.Link
                Accessible.name: qsTr("Open the Android app download page")
                Accessible.onPressAction: Setup.openAndroidPage()
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -6
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Setup.openAndroidPage()
                }
            }

            Item { width: 1; height: 26 }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 12
                SetupButton {
                    text: window.removing ? qsTr("Close") : qsTr("Open NectarLink")
                    fill: window.removing ? window.ink : window.honey
                    ink: window.removing ? window.bg : window.onHoney
                    focusColor: window.honey
                    onClicked: window.primary()
                }
                SetupButton {
                    visible: !window.removing
                    filled: false
                    text: qsTr("Close")
                    ink: window.ink
                    focusColor: window.honey
                    onClicked: window.close()
                }
            }
        }

        // ---- What this installs ----
        Rectangle {
            id: disclosure
            property bool open: false
            anchors.fill: parent
            color: Qt.rgba(window.bg.r, window.bg.g, window.bg.b, 0.72)
            opacity: open ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { NumberAnimation { duration: 200 } }
            MouseArea { anchors.fill: parent; onClicked: disclosure.open = false }

            Rectangle {
                anchors.centerIn: parent
                width: 500
                height: notes.implicitHeight + 64
                radius: 20
                color: window.panel
                border.width: 1
                border.color: Qt.rgba(window.ink.r, window.ink.g, window.ink.b, 0.10)
                MouseArea { anchors.fill: parent }

                Column {
                    id: notes
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.margins: 32
                    spacing: 12
                    Text {
                        font.family: "Figtree"
                        font.pixelSize: 18
                        font.weight: Font.DemiBold
                        color: window.ink
                        text: qsTr("What this installs")
                    }
                    Text {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        lineHeight: 1.3
                        font.family: "Figtree"
                        font.pixelSize: 14
                        color: window.muted
                        textFormat: Text.StyledText
                        text: qsTr("NectarLink goes into Program Files for everyone on this PC. Setup also:")
                            + "<br>&#8226; " + qsTr("allows it through Windows Firewall on private networks, so your phone can connect")
                            + "<br>&#8226; " + qsTr("adds a task that installs updates without an administrator prompt")
                            + "<br>&#8226; " + qsTr("adds a Start menu shortcut, and a desktop one if you leave that ticked, and starts it with Windows (turn this off in Settings)")
                            + "<br>&#8226; " + qsTr("adds it to the Windows Share sheet")
                            + "<br><br>" + qsTr("It checks GitHub for updates once a day, which you can turn off in Settings. Otherwise your phone and PC only talk to each other.")
                    }
                    Text {
                        font.family: "Figtree"
                        font.pixelSize: 14
                        font.underline: true
                        color: window.honey
                        text: qsTr("Privacy details")
                        Accessible.role: Accessible.Link
                        Accessible.name: text
                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -6
                            cursorShape: Qt.PointingHandCursor
                            onClicked: Qt.openUrlExternally("https://github.com/alejandrokundrah/nectarlink/blob/main/docs/guide/privacy-and-security.md")
                        }
                    }
                }
            }
        }
    }
}
