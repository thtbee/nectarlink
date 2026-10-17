// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes

// The uninstaller's bee: wings and antennae drooping, a frown, and now and
// then a small tear. Drawn on a 140 × 120 grid and scaled to fit.
Item {
    id: bee

    property bool animate: true
    property bool dark: false
    property color bodyColor: dark ? "#D9AE68" : "#E7B86A"
    property color stripeColor: dark ? "#2A2019" : "#3B2A1D"
    property color headColor: dark ? "#2A2019" : "#3B2A1D"
    property color wingColor: dark ? Qt.rgba(0.86, 0.90, 1.0, 0.30) : Qt.rgba(1, 1, 1, 0.92)
    property color wingEdge: dark ? Qt.rgba(1, 1, 1, 0.45) : Qt.rgba(0.23, 0.16, 0.11, 0.32)
    property color tearColor: dark ? "#9CC7F0" : "#6FA8DC"

    implicitWidth: 140
    implicitHeight: 120

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("A sad bee")

    Item {
        id: art
        width: 140
        height: 120
        anchors.centerIn: parent
        scale: Math.min(bee.width / 140, bee.height / 120)

        // A slow, heavy bob.
        property real bob: 0
        SequentialAnimation on bob {
            running: bee.animate
            loops: Animation.Infinite
            NumberAnimation { from: 0; to: 3; duration: 1900; easing.type: Easing.InOutSine }
            NumberAnimation { from: 3; to: 0; duration: 1900; easing.type: Easing.InOutSine }
        }
        transform: Translate { y: art.bob }

        // Wings: two ovals rising from the back, bases tucked behind the
        // body, tipped backwards (drooping). The far one is smaller and fainter.
        Repeater {
            model: [
                { cx: 88, cy: 32, w: 17, h: 26, r: 38, a: 0.7 },
                { cx: 74, cy: 29, w: 20, h: 31, r: 22, a: 1.0 }
            ]
            Rectangle {
                required property var modelData
                x: modelData.cx - width / 2
                y: modelData.cy - height / 2
                width: modelData.w
                height: modelData.h
                radius: width / 2
                color: bee.wingColor
                opacity: modelData.a
                border.width: 1.2
                border.color: bee.wingEdge
                antialiasing: true
                transformOrigin: Item.Bottom
                property real flutter: 0
                rotation: modelData.r + flutter
                SequentialAnimation on flutter {
                    running: bee.animate
                    loops: Animation.Infinite
                    PauseAnimation { duration: 2600 }
                    NumberAnimation { from: 0; to: -7; duration: 240; easing.type: Easing.OutSine }
                    NumberAnimation { from: -7; to: 0; duration: 520; easing.type: Easing.InOutSine }
                }
                // A vein down the middle.
                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: parent.height * 0.22
                    width: 1
                    height: parent.height * 0.62
                    radius: 0.5
                    color: bee.wingEdge
                }
            }
        }

        // Body with stripes and a stinger.
        Shape {
            anchors.fill: parent
            Component.onCompleted: if ("preferredRendererType" in this) preferredRendererType = 2
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.stripeColor
                startX: 110; startY: 62
                PathLine { x: 120; y: 67 }
                PathLine { x: 110; y: 72 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.bodyColor
                PathAngleArc { centerX: 78; centerY: 67; radiusX: 34; radiusY: 24; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.stripeColor
                startX: 68; startY: 44.66
                PathLine { x: 76; y: 43.64 }
                PathLine { x: 76; y: 90.36 }
                PathLine { x: 68; y: 89.34 }
                PathLine { x: 68; y: 44.66 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.stripeColor
                startX: 86; startY: 44.27
                PathLine { x: 94; y: 46.42 }
                PathLine { x: 94; y: 87.58 }
                PathLine { x: 86; y: 89.73 }
                PathLine { x: 86; y: 44.27 }
            }
        }

        // Head, face and drooping antennae.
        Shape {
            anchors.fill: parent
            Component.onCompleted: if ("preferredRendererType" in this) preferredRendererType = 2
            ShapePath {
                strokeWidth: 2.2
                strokeColor: bee.headColor
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 36; startY: 47
                PathQuad { x: 22; y: 44; controlX: 28; controlY: 36 }
            }
            ShapePath {
                strokeWidth: 2.2
                strokeColor: bee.headColor
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 44; startY: 46
                PathQuad { x: 54; y: 38; controlX: 50; controlY: 34 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.headColor
                PathAngleArc { centerX: 22; centerY: 45; radiusX: 2.6; radiusY: 2.6; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.headColor
                PathAngleArc { centerX: 54; centerY: 39; radiusX: 2.6; radiusY: 2.6; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.headColor
                PathAngleArc { centerX: 40; centerY: 63; radiusX: 19; radiusY: 18; startAngle: 0; sweepAngle: 360 }
            }
            // Keeps the dark head apart from a dark background.
            ShapePath {
                strokeWidth: 1.2
                strokeColor: bee.dark ? Qt.rgba(1, 1, 1, 0.22) : "transparent"
                fillColor: "transparent"
                PathAngleArc { centerX: 40; centerY: 63; radiusX: 19.4; radiusY: 18.4; startAngle: 0; sweepAngle: 360 }
            }
            // Eyes, looking down.
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: "#FFFFFF"
                PathAngleArc { centerX: 33; centerY: 60; radiusX: 3.6; radiusY: 3.6; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: "#FFFFFF"
                PathAngleArc { centerX: 46; centerY: 60; radiusX: 3.6; radiusY: 3.6; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.headColor
                PathAngleArc { centerX: 33; centerY: 61.4; radiusX: 1.7; radiusY: 1.7; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeWidth: 0
                strokeColor: "transparent"
                fillColor: bee.headColor
                PathAngleArc { centerX: 46; centerY: 61.4; radiusX: 1.7; radiusY: 1.7; startAngle: 0; sweepAngle: 360 }
            }
            // Brows, raised in the middle.
            ShapePath {
                strokeWidth: 1.8
                strokeColor: "#FFFFFF"
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 29; startY: 54.5
                PathLine { x: 36; y: 52 }
            }
            ShapePath {
                strokeWidth: 1.8
                strokeColor: "#FFFFFF"
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 43; startY: 52
                PathLine { x: 50; y: 54.5 }
            }
            // A small frown.
            ShapePath {
                strokeWidth: 1.8
                strokeColor: "#FFFFFF"
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 35.5; startY: 71.5
                PathQuad { x: 44.5; y: 71.5; controlX: 40; controlY: 67.5 }
            }
        }

        // Now and then, a tear.
        Rectangle {
            id: tear
            x: 30.5
            width: 4
            height: 5.5
            radius: 2
            color: bee.tearColor
            visible: bee.animate
            property real fall: 0
            y: 64 + fall * 14
            opacity: fall < 0.05 ? fall * 20 : 1 - fall
            SequentialAnimation on fall {
                running: bee.animate
                loops: Animation.Infinite
                PauseAnimation { duration: 3200 }
                NumberAnimation { from: 0; to: 1; duration: 1500; easing.type: Easing.InQuad }
                PropertyAction { value: 0 }
            }
        }
    }
}
