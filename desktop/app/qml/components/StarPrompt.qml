// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// Asks once for a star on GitHub (or support): right as the window opens after an update, or
// after ten minutes of use on a new install. A honeycomb cell lands in the corner
// and fills with honey, a star rises out of it, and the question unfolds from the
// cell. Any answer ends it for good. Nothing moves while it waits for
// an answer, so it costs nothing to leave open.
Item {
    id: prompt

    // Whether it may appear now, and for how long it has to before a new install
    // is asked. Minutes add up across sessions, so short ones still get there.
    property bool eligible: false
    property int minuteMs: 60000
    property int minutesOfUse: 10
    property int updatedWaitMs: 1500
    signal starred
    signal supported

    readonly property string phase: internal.phase
    readonly property real cellSize: 30

    anchors.right: parent ? parent.right : undefined
    anchors.bottom: parent ? parent.bottom : undefined
    anchors.rightMargin: 20
    anchors.bottomMargin: 20
    width: pill.width
    height: pill.height
    visible: internal.phase !== "waiting" && internal.phase !== "gone"
    opacity: internal.shown
    scale: Theme.reduceMotion ? 1 : 0.92 + 0.08 * internal.shown

    Accessible.role: Accessible.AlertMessage
    Accessible.name: question.text

    function star() {
        if (internal.phase !== "open")
            return
        prompt.starred()
        thank()
    }

    function support() {
        if (internal.phase !== "open")
            return
        prompt.supported()
        thank()
    }

    function thank() {
        Preferences.starPromptDone = true
        internal.phase = "thanks"
        arrive.stop()
        if (Theme.reduceMotion)
            thankedPause.start()
        else
            thanks.start()
    }

    function dismiss() {
        if (internal.phase !== "open")
            return
        Preferences.starPromptDone = true
        arrive.stop()
        leave.start()
    }

    QtObject {
        id: internal
        property string phase: "waiting"
        property real shown: 0
        property real cell: 0      // the cell landing, 0 to 1
        property real honey: 0     // how full it is, 0 to 1
        property real tilt: 0      // the honey's surface sloshing, in fractions of the cell
        property real starScale: 0
        property real reveal: 0    // the question unfolding, 0 to 1
        property real burst: 0     // the sparks flying off, 0 to 1

        readonly property color honeyColor: Theme.graphite ? Theme.surfaceContent : Theme.primary
        readonly property color starColor: Theme.graphite ? Theme.surface : Theme.primaryContent

        // A pointy-topped hexagon filling a box of `size`.
        function hexagon(size) {
            const r = size / 2
            const out = []
            for (let i = 0; i < 6; i++) {
                const a = Math.PI / 180 * (60 * i - 90)
                out.push(Qt.point(r + r * Math.cos(a), r + r * Math.sin(a)))
            }
            return out
        }

        // The part of the hexagon under the honey's tilted surface.
        function honeyShape(size, level, slope) {
            const points = hexagon(size)
            const surface = (x) => size * (1 - level) + (x - size / 2) * slope
            const below = (p) => p.y >= surface(p.x)
            const out = []
            for (let i = 0; i < points.length; i++) {
                const a = points[i]
                const b = points[(i + 1) % points.length]
                if (below(a))
                    out.push(a)
                if (below(a) !== below(b)) {
                    const da = a.y - surface(a.x)
                    const db = b.y - surface(b.x)
                    const t = da / (da - db)
                    out.push(Qt.point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
                }
            }
            if (out.length > 0)
                out.push(out[0])
            return out
        }

        function closed(points) {
            return points.concat([points[0]])
        }

        function star(size) {
            const out = []
            for (let i = 0; i < 10; i++) {
                const r = (i % 2 === 0 ? 0.5 : 0.21) * size
                const a = Math.PI / 180 * (36 * i - 90)
                out.push(Qt.point(size / 2 + r * Math.cos(a), size / 2 + r * Math.sin(a)))
            }
            return closed(out)
        }
    }

    Timer {
        interval: Preferences.starPromptUpdated ? prompt.updatedWaitMs : prompt.minuteMs
        repeat: true
        running: prompt.eligible && internal.phase === "waiting" && !Preferences.starPromptDone
        onTriggered: {
            if (!Preferences.starPromptUpdated) {
                Preferences.starPromptMinutes += 1
                if (Preferences.starPromptMinutes < prompt.minutesOfUse)
                    return
            }
            internal.phase = "open"
            if (Theme.reduceMotion) {
                internal.cell = internal.honey = internal.starScale = internal.reveal = 1
                quietArrive.start()
            } else {
                arrive.start()
            }
        }
    }

    NumberAnimation {
        id: quietArrive
        target: internal
        property: "shown"
        to: 1
        duration: Theme.fadeNormal
    }

    SequentialAnimation {
        id: arrive
        ParallelAnimation {
            NumberAnimation { target: internal; property: "shown"; to: 1; duration: 180 }
            NumberAnimation { target: internal; property: "cell"; to: 1; duration: 420; easing.type: Easing.OutBack; easing.overshoot: 2.2 }
        }
        ParallelAnimation {
            NumberAnimation { target: internal; property: "honey"; to: 1; duration: 720; easing.type: Easing.InOutSine }
            SequentialAnimation {
                NumberAnimation { target: internal; property: "tilt"; to: 0.34; duration: 180; easing.type: Easing.OutSine }
                NumberAnimation { target: internal; property: "tilt"; to: -0.2; duration: 220; easing.type: Easing.InOutSine }
                NumberAnimation { target: internal; property: "tilt"; to: 0.09; duration: 180; easing.type: Easing.InOutSine }
                NumberAnimation { target: internal; property: "tilt"; to: 0; duration: 140; easing.type: Easing.OutSine }
            }
        }
        NumberAnimation { target: internal; property: "starScale"; to: 1; duration: 300; easing.type: Easing.OutBack; easing.overshoot: 3 }
        PauseAnimation { duration: 120 }
        NumberAnimation { target: internal; property: "reveal"; to: 1; duration: 460; easing.type: Easing.OutCubic }
    }

    SequentialAnimation {
        id: thanks
        ParallelAnimation {
            NumberAnimation { target: internal; property: "burst"; from: 0; to: 1; duration: 620; easing.type: Easing.OutCubic }
            SequentialAnimation {
                NumberAnimation { target: internal; property: "starScale"; to: 1.5; duration: 130; easing.type: Easing.OutCubic }
                NumberAnimation { target: internal; property: "starScale"; to: 1; duration: 320; easing.type: Easing.OutBack; easing.overshoot: 2 }
            }
        }
        PauseAnimation { duration: 700 }
        ScriptAction { script: leave.start() }
    }

    Timer {
        id: thankedPause
        interval: 1200
        onTriggered: leave.start()
    }

    SequentialAnimation {
        id: leave
        NumberAnimation { target: internal; property: "reveal"; to: 0; duration: Theme.reduceMotion ? 0 : 280; easing.type: Easing.InCubic }
        ParallelAnimation {
            NumberAnimation { target: internal; property: "shown"; to: 0; duration: Theme.fadeNormal }
            NumberAnimation { target: internal; property: "cell"; to: Theme.reduceMotion ? 1 : 0; duration: Theme.reduceMotion ? 0 : 240; easing.type: Easing.InBack }
        }
        ScriptAction { script: internal.phase = "gone" }
    }

    Rectangle {
        id: pill
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 48
        width: 9 + prompt.cellSize + 9 + internal.reveal * unfolded.width
        radius: Theme.pill(height)
        color: Theme.surfaceContainerHigh
        border.width: 1
        border.color: Theme.outlineVariant

        // Holds clicks, so they don't reach the page underneath.
        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
        }

        Item {
            id: cell
            x: 9
            anchors.verticalCenter: parent.verticalCenter
            width: prompt.cellSize
            height: prompt.cellSize
            scale: internal.cell

            Shape {
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeWidth: 1.5
                    strokeColor: internal.honeyColor
                    fillColor: "transparent"
                    joinStyle: ShapePath.RoundJoin
                    PathPolyline { path: internal.closed(internal.hexagon(prompt.cellSize)) }
                }
                ShapePath {
                    strokeWidth: 0
                    strokeColor: "transparent"
                    fillColor: internal.honeyColor
                    PathPolyline {
                        path: internal.honey > 0
                            ? internal.honeyShape(prompt.cellSize, internal.honey, internal.tilt)
                            : []
                    }
                }
            }

            Shape {
                anchors.centerIn: parent
                width: 16
                height: 16
                scale: internal.starScale
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeWidth: 0
                    strokeColor: "transparent"
                    fillColor: internal.starColor
                    PathPolyline { path: internal.star(16) }
                }
            }

            // Six sparks of honey, one for each side of the cell.
            Repeater {
                model: 6
                Rectangle {
                    required property int index
                    readonly property real angle: Math.PI / 180 * (60 * index - 60)
                    readonly property real reach: 12 + 24 * internal.burst
                    visible: internal.burst > 0 && internal.burst < 1
                    x: cell.width / 2 + Math.cos(angle) * reach - width / 2
                    y: cell.height / 2 + Math.sin(angle) * reach - height / 2
                    width: 5 * (1 - internal.burst * 0.6)
                    height: width
                    radius: width / 2
                    color: internal.honeyColor
                    opacity: 1 - internal.burst
                }
            }
        }

        Item {
            anchors.left: cell.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: internal.reveal * unfolded.width
            clip: true

            Row {
                id: unfolded
                anchors.verticalCenter: parent.verticalCenter
                leftPadding: 10
                rightPadding: 8
                spacing: 8
                opacity: internal.reveal

                Txt {
                    id: question
                    anchors.verticalCenter: parent.verticalCenter
                    role: "body"
                    color: Theme.surfaceContent
                    text: internal.phase === "thanks" ? qsTr("Thank you!") : qsTr("Enjoying NectarLink?")
                    rightPadding: internal.phase === "thanks" ? 10 : 4
                }

                Button {
                    id: starButton
                    objectName: "starButton"
                    anchors.verticalCenter: parent.verticalCenter
                    visible: internal.phase !== "thanks"
                    size: "sm"
                    text: qsTr("Star on GitHub")
                    enabled: internal.reveal === 1
                    onClicked: prompt.star()
                }

                Button {
                    objectName: "supportButton"
                    anchors.verticalCenter: parent.verticalCenter
                    visible: internal.phase !== "thanks"
                    size: "sm"
                    variant: "tonal"
                    text: qsTr("Support")
                    enabled: internal.reveal === 1
                    onClicked: prompt.support()
                }

                IconButton {
                    id: closeButton
                    objectName: "closeButton"
                    anchors.verticalCenter: parent.verticalCenter
                    visible: internal.phase !== "thanks"
                    iconPath: Icons.close
                    label: qsTr("Dismiss")
                    enabled: internal.reveal === 1
                    onClicked: prompt.dismiss()
                }
            }
        }
    }
}
