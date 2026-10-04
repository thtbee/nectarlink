import QtQuick
import QtQuick.Shapes

// Stroke icon from an SVG path on a 24x24 grid (same paths as the design mockups).
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
        scale: icon.width / 24
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
