// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// Minimize, maximize/restore and close, drawn in the app's colors using crisp
// vector strokes so they render identically on Windows and Linux without
// requiring the Windows-only Segoe Fluent Icons font. The maximize button is
// registered with the window so Windows shows Snap Layouts when hovering it.
Row {
    id: buttons
    required property NativeWindow window
    readonly property bool maximized: window.visibility === Window.Maximized

    // As tall as the top bar, like Windows 11's tall title bars, so the glyphs
    // line up with the page title and the logo.
    height: Theme.topBarHeight
    z: 1000

    CaptionButton {
        id: minimize
        iconPath: "M 1 5 L 9 5"
        label: qsTr("Minimize")
        onActivated: buttons.window.showMinimized()
        Component.onCompleted: buttons.window.addCaptionHole(minimize)
    }
    CaptionButton {
        id: maximize
        iconPath: buttons.maximized
                  ? "M 1 3 L 7 3 L 7 9 L 1 9 Z M 3 3 L 3 1 L 9 1 L 9 7 L 7 7"
                  : "M 1 1 L 9 1 L 9 9 L 1 9 Z"
        label: buttons.maximized ? qsTr("Restore") : qsTr("Maximize")
        // Mouse input for this button goes through Windows WM_NCHITTEST (Snap Layouts) on Windows.
        nativeInput: Qt.platform.os === "windows"
        nativeHovered: buttons.window.maximizeHovered
        nativePressed: buttons.window.maximizePressed
        onActivated: buttons.window.toggleMaximized()
        Component.onCompleted: {
            buttons.window.maximizeButton = maximize
            buttons.window.addCaptionHole(maximize)
        }
    }
    CaptionButton {
        id: close
        iconPath: "M 1 1 L 9 9 M 9 1 L 1 9"
        label: qsTr("Close")
        danger: true
        onActivated: buttons.window.close()
        Component.onCompleted: buttons.window.addCaptionHole(close)
    }

    component CaptionButton: Item {
        id: button
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

        // The close button turns red on hover in every theme.
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
        Shape {
            anchors.centerIn: parent
            width: 10
            height: 10
            opacity: buttons.window.active || button.hovered ? 1 : 0.45
            Component.onCompleted: if ("preferredRendererType" in this) preferredRendererType = 2
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
