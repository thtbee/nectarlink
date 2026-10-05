// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// An indeterminate progress ring. Pulses instead of spinning when the
// user asked for reduced motion.
Item {
    id: spinner
    property color color: Theme.primary
    implicitWidth: 20
    implicitHeight: 20

    Shape {
        anchors.fill: parent
        preferredRendererType: Shape.CurveRenderer
        RotationAnimator on rotation {
            running: spinner.visible && !Theme.reduceMotion
            from: 0; to: 360; duration: 900
            loops: Animation.Infinite
        }
        SequentialAnimation on opacity {
            running: spinner.visible && Theme.reduceMotion
            loops: Animation.Infinite
            NumberAnimation { to: 0.35; duration: 600 }
            NumberAnimation { to: 1; duration: 600 }
        }
        ShapePath {
            strokeColor: spinner.color
            strokeWidth: Math.max(2, spinner.width / 10)
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc {
                centerX: spinner.width / 2; centerY: spinner.height / 2
                radiusX: spinner.width / 2 - 2; radiusY: spinner.height / 2 - 2
                startAngle: 0; sweepAngle: 270
            }
        }
    }
}
