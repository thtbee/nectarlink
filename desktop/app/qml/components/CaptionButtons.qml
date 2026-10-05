// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Minimize, maximize/restore and close, drawn like Windows 11's own (same
// size and glyphs) in the app's colors. The maximize button is registered
// with the window so Windows shows Snap Layouts when hovering it.
Row {
    id: buttons
    required property NativeWindow window
    readonly property bool maximized: window.visibility === Window.Maximized

    height: 36
    z: 1000

    CaptionButton {
        id: minimize
        glyph: ""
        label: qsTr("Minimize")
        onActivated: buttons.window.showMinimized()
        Component.onCompleted: buttons.window.addCaptionHole(minimize)
    }
    CaptionButton {
        id: maximize
        glyph: buttons.maximized ? "" : ""
        label: buttons.maximized ? qsTr("Restore") : qsTr("Maximize")
        // Mouse input for this button goes through Windows (Snap Layouts).
        nativeInput: true
        hovered: buttons.window.maximizeHovered
        pressed: buttons.window.maximizePressed
        onActivated: buttons.window.toggleMaximized()
        Component.onCompleted: buttons.window.maximizeButton = maximize
    }
    CaptionButton {
        id: close
        glyph: ""
        label: qsTr("Close")
        danger: true
        onActivated: buttons.window.close()
        Component.onCompleted: buttons.window.addCaptionHole(close)
    }

    component CaptionButton: Item {
        id: button
        property string glyph
        property string label
        property bool danger: false
        property bool nativeInput: false
        property bool hovered: !nativeInput && hover.hovered
        property bool pressed: !nativeInput && tap.pressed
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
            text: button.glyph
            font.family: "Segoe Fluent Icons"
            font.pixelSize: 10
            color: button.danger && (button.hovered || button.pressed) ? "#FFFFFF" : Theme.surfaceContent
            opacity: buttons.window.active || button.hovered ? 1 : 0.45
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: 2
            color: "transparent"
            border.width: 2
            border.color: Theme.primary
            visible: button.activeFocus
        }
        HoverHandler { id: hover; enabled: !button.nativeInput }
        TapHandler { id: tap; enabled: !button.nativeInput; onTapped: button.activated() }
    }
}
