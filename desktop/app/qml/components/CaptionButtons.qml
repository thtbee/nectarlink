// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// Minimize, maximize/restore and close, drawn like Windows 11's own on Windows
// (Segoe Fluent Icons, or Segoe MDL2 Assets on Windows 10) and with crisp vector strokes on Linux where that font
// isn't installed. The maximize button is registered with the window so Windows
// shows Snap Layouts when hovering it.
Row {
    id: buttons
    required property NativeWindow window
    readonly property bool maximized: window.visibility === Window.Maximized

    visible: Qt.platform.os !== "osx"
    width: visible ? implicitWidth : 0

    // As tall as the top bar, like Windows 11's tall title bars, so the glyphs
    // line up with the page title and the logo.
    height: Theme.topBarHeight
    z: 1000

    CaptionButton {
        id: minimize
        glyph: "\ue921"
        iconPath: "M 1 5 L 9 5"
        label: qsTr("Minimize")
        onActivated: buttons.window.showMinimized()
        Component.onCompleted: buttons.window.addCaptionHole(minimize)
    }
    CaptionButton {
        id: maximize
        glyph: buttons.maximized ? "\ue923" : "\ue922"
        iconPath: buttons.maximized
                  ? "M 1 3 L 7 3 L 7 9 L 1 9 Z M 3 3 L 3 1 L 9 1 L 9 7 L 7 7"
                  : "M 1 1 L 9 1 L 9 9 L 1 9 Z"
        label: buttons.maximized ? qsTr("Restore") : qsTr("Maximize")
        // Mouse input for this button goes through Windows (Snap Layouts) on Windows.
        nativeInput: Qt.platform.os === "windows"
        nativeHovered: buttons.window.maximizeHovered
        nativePressed: buttons.window.maximizePressed
        onActivated: buttons.window.toggleMaximized()
        Component.onCompleted: buttons.window.maximizeButton = maximize
    }
    CaptionButton {
        id: close
        glyph: "\ue8bb"
        iconPath: "M 1 1 L 9 9 M 9 1 L 1 9"
        label: qsTr("Close")
        danger: true
        onActivated: buttons.window.close()
        Component.onCompleted: buttons.window.addCaptionHole(close)
    }

    component CaptionButton: Item {
        id: button
        property string glyph
        property string iconPath
        property string label
        property bool danger: false
        property bool nativeInput: false
        property bool nativeHovered: false
        property bool nativePressed: false
        readonly property bool hovered: nativeInput ? nativeHovered : hover.hovered
        readonly property bool pressed: nativeInput ? nativePressed : tap.pressed
        signal activated

        width: 46
        height: buttons.height
        activeFocusOnTab: true

        Accessible.role: Accessible.Button
        Accessible.name: label
        Accessible.onPressAction: button.activated()
        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                button.activated()
                event.accepted = true
            }
        }

        // Windows' close button turns red on hover in every theme.
        readonly property color dangerColor: "#C42B1C"
        Rectangle {
            anchors.fill: parent
            color: button.danger && (button.hovered || button.pressed)
                   ? (button.pressed ? Qt.lighter(button.dangerColor, 1.15) : button.dangerColor)
                   : Theme.surfaceContent
            opacity: button.danger && (button.hovered || button.pressed) ? 1
                     : (button.pressed ? 0.12 : (button.hovered ? 0.07 : 0))
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        }
        Text {
            anchors.centerIn: parent
            visible: Qt.platform.os === "windows"
            text: button.glyph
            font.family: Theme.captionIconFont
            font.pixelSize: 10
            color: button.danger && (button.hovered || button.pressed) ? "#FFFFFF" : Theme.surfaceContent
            opacity: buttons.window.active || button.hovered ? 1 : 0.45
        }
        Shape {
            anchors.centerIn: parent
            visible: Qt.platform.os !== "windows"
            width: 10
            height: 10
            opacity: buttons.window.active || button.hovered ? 1 : 0.45
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: button.danger && (button.hovered || button.pressed) ? "#FFFFFF" : Theme.surfaceContent
                strokeWidth: 1.25
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                PathSvg { path: button.iconPath }
            }
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: 2
            color: "transparent"
            border.width: 2
            border.color: Theme.primary
            visible: Theme.focusVisible(button)
        }
        HoverHandler { id: hover; enabled: !button.nativeInput }
        TapHandler { id: tap; enabled: !button.nativeInput; onTapped: button.activated() }
    }
}
