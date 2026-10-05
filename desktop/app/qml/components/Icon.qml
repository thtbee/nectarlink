// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// A stroke icon from Icons (an SVG path on a 24×24 grid).
Item {
    id: icon
    property string path
    property color color: Theme.surfaceContent
    property real stroke: 1.8

    implicitWidth: 20
    implicitHeight: 20

    Shape {
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: Math.min(icon.width, icon.height) / 24
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: icon.color
            strokeWidth: icon.stroke
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.path }
        }
    }
}
