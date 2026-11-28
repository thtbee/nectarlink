// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import app.nectarlink

// Sleek frosted pill that slides out from the configured screen edge only when
// the user drags local file(s) (`CF_HDROP`) to that edge while a phone is
// connected. Never takes keyboard focus and stays click-through (`WindowTransparentForInput`)
// while the native Win32 IDropTarget beneath handles the drag-and-drop session.
Window {
    id: edgeDropWin

    title: qsTr("NectarLink Drop")
    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
        | Qt.WindowDoesNotAcceptFocus | Qt.WindowTransparentForInput
    color: "transparent"
    visible: true

    readonly property int availW: Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920
    readonly property int availH: Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : 1080
    property var workArea: AppController.workArea()
    readonly property int workAreaKey: Screen.desktopAvailableWidth * 100000 + Screen.desktopAvailableHeight
    onWorkAreaKeyChanged: workArea = AppController.workArea()
    readonly property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1
    readonly property real areaX: workArea && workArea.width ? workArea.x / dpr : Screen.virtualX
    readonly property real areaY: workArea && workArea.height ? workArea.y / dpr : Screen.virtualY
    readonly property real areaW: workArea && workArea.width ? workArea.width / dpr : availW
    readonly property real areaH: workArea && workArea.height ? workArea.height / dpr : availH

    readonly property string side: Preferences.screenEdgeDropSide === "left" ? "left" : "right"
    readonly property string phoneName: AppController.edgeDropPhoneName.length > 0
        ? AppController.edgeDropPhoneName
        : qsTr("phone")

    width: 296
    height: 64
    x: side === "left" ? areaX + 4 : areaX + areaW - width - 4
    y: areaY + Math.round((areaH - height) / 2)

    property bool positioned: false
    property bool dismissing: false
    readonly property bool shown: positioned && !dismissing && (AppController.edgeDropHovering || AppController.edgeDropSent)

    readonly property string pillLabel: {
        if (AppController.edgeDropSent) {
            const n = AppController.edgeDropFileCount
            if (n > 1) {
                return AppController.edgeDropOnline
                    ? qsTr("Sent %1 files to %2").arg(n).arg(phoneName)
                    : qsTr("Queued %1 files for %2").arg(n).arg(phoneName)
            }
            return AppController.edgeDropOnline
                ? qsTr("Sent to %1").arg(phoneName)
                : qsTr("Queued for %1").arg(phoneName)
        }
        return AppController.edgeDropOpenOnDevice
            ? qsTr("Drop to send & open on %1").arg(phoneName)
            : qsTr("Drop to send to %1").arg(phoneName)
    }

    function syncLifecycle() {
        if (AppController.edgeDropHovering) {
            dismissing = false
            confirmHideTimer.stop()
            closeDelayTimer.stop()
            return
        }
        if (AppController.edgeDropSent) {
            dismissing = false
            closeDelayTimer.stop()
            confirmHideTimer.restart()
            return
        }
        confirmHideTimer.stop()
        if (Theme.reduceMotion) {
            AppController.finishEdgeDropClose()
        } else {
            closeDelayTimer.restart()
        }
    }

    Component.onCompleted: {
        AppController.styleEdgeDropWindow()
        positioned = true
        Qt.callLater(() => AppController.styleEdgeDropWindow())
        syncLifecycle()
    }

    Connections {
        target: AppController
        function onEdgeDropHoveringChanged() { edgeDropWin.syncLifecycle() }
        function onEdgeDropSentChanged() { edgeDropWin.syncLifecycle() }
    }

    Timer {
        id: confirmHideTimer
        interval: 1500
        repeat: false
        onTriggered: {
            edgeDropWin.dismissing = true
            if (Theme.reduceMotion) {
                AppController.finishEdgeDropClose()
            } else {
                closeDelayTimer.restart()
            }
        }
    }

    Timer {
        id: closeDelayTimer
        interval: 160
        repeat: false
        onTriggered: AppController.finishEdgeDropClose()
    }

    Item {
        id: slideWrap
        anchors.fill: parent
        opacity: edgeDropWin.shown ? 1.0 : 0.0

        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
        }

        transform: Translate {
            x: edgeDropWin.shown ? 0 : (edgeDropWin.side === "left" ? -32 : 32)
            Behavior on x {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
            }
        }

        // Subtle outer ambient halo around the frosted pill
        Rectangle {
            anchors.centerIn: pill
            width: pill.width + 6
            height: pill.height + 6
            radius: height / 2
            color: "transparent"
            border.width: 1.5
            border.color: AppController.edgeDropSent
                ? Qt.rgba(0.29, 0.87, 0.50, 0.34)
                : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.28)
        }

        Rectangle {
            id: pill
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: edgeDropWin.side === "right" ? parent.right : undefined
            anchors.left: edgeDropWin.side === "left" ? parent.left : undefined
            anchors.rightMargin: 8
            anchors.leftMargin: 8
            width: Math.min(parent.width - 16, Math.max(196, contentRow.implicitWidth + 32))
            height: 46
            radius: height / 2
            clip: true
            color: Theme.glass
                ? (Theme.dark ? Qt.rgba(0.08, 0.09, 0.12, 0.88) : Qt.rgba(0.98, 0.985, 0.995, 0.90))
                : Theme.surfaceContainerHigh
            border.width: 1
            border.color: AppController.edgeDropSent
                ? Qt.rgba(0.29, 0.87, 0.50, 0.65)
                : Theme.primary

            Behavior on width {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
            }

            GlassRim {
                radius: pill.radius
                strength: 1.1
            }

            Row {
                id: contentRow
                anchors.centerIn: parent
                spacing: 10

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28
                    height: 28
                    radius: 14
                    color: AppController.edgeDropSent
                        ? Qt.rgba(0.29, 0.87, 0.50, 0.20)
                        : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.18)

                    Icon {
                        anchors.centerIn: parent
                        width: 15
                        height: 15
                        path: AppController.edgeDropSent ? Icons.check : Icons.send
                        color: AppController.edgeDropSent ? "#22C55E" : Theme.primary
                    }
                }

                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(implicitWidth, edgeDropWin.width - 86)
                    elide: Text.ElideRight
                    role: "bodySmall"
                    weight: 600
                    color: Theme.surfaceContent
                    text: edgeDropWin.pillLabel
                }
            }
        }
    }
}
