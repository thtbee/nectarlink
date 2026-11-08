// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A slider for a whole number between `from` and `to`, in steps of `step`.
// It shows `value` and reports where the user moves it with `moved`; the
// owner sets `value` (so a drag that's refused springs back). Arrow keys
// step it; screen readers get its name and value.
FocusScope {
    id: slider
    property int from: 0
    property int to: 100
    property int step: 5
    property int value: 0
    property string label
    // How the value reads ("150%"), for the screen reader.
    property string valueText: String(value)
    signal moved(int value)

    readonly property real fraction: to > from ? (Math.max(from, Math.min(to, value)) - from) / (to - from) : 0

    implicitWidth: 200
    implicitHeight: 24
    activeFocusOnTab: enabled
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.Slider
    Accessible.name: label
    Accessible.description: valueText

    function stepped(next) {
        const clamped = Math.max(from, Math.min(to, Math.round(next / step) * step))
        if (clamped !== value)
            moved(clamped)
    }

    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Left || event.key === Qt.Key_Down) {
            stepped(value - step)
            event.accepted = true
        } else if (event.key === Qt.Key_Right || event.key === Qt.Key_Up) {
            stepped(value + step)
            event.accepted = true
        } else if (event.key === Qt.Key_Home) {
            stepped(from)
            event.accepted = true
        } else if (event.key === Qt.Key_End) {
            stepped(to)
            event.accepted = true
        }
    }

    Rectangle {
        id: track
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width
        height: Theme.fluent ? 4 : 6
        radius: height / 2
        color: Theme.fluent ? Theme.controlStrong : Theme.surfaceContainerHighest
        border.width: Theme.graphite ? 1 : 0
        border.color: Theme.outlineVariant

        Rectangle {
            id: fill
            width: parent.width * slider.fraction
            height: parent.height
            radius: parent.radius
            color: Theme.graphite ? Theme.surfaceContent : Theme.primary
            Behavior on width {
                enabled: !Theme.reduceMotion && !drag.pressed
                NumberAnimation { duration: 120; easing.type: Easing.OutCubic }
            }
        }
    }

    // Fluent: WinUI's thumb, a raised disc with an accent dot that grows under
    // the pointer and shrinks while dragged.
    Rectangle {
        visible: Theme.fluent
        x: Math.max(0, Math.min(slider.width - width, fill.width - width / 2))
        anchors.verticalCenter: parent.verticalCenter
        width: 20; height: 20; radius: 10
        color: Theme.dark ? "#454545" : "#FFFFFF"
        border.width: 1
        border.color: Theme.controlStroke
        Rectangle {
            anchors.centerIn: parent
            readonly property real size: drag.pressed ? 10 : (drag.containsMouse ? 14 : 12)
            width: size; height: size; radius: size / 2
            color: Theme.primary
            Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: 83; easing.type: Easing.OutCubic } }
            Behavior on height { enabled: !Theme.reduceMotion; NumberAnimation { duration: 83; easing.type: Easing.OutCubic } }
        }
    }

    Rectangle {
        id: handle
        visible: !Theme.fluent
        x: Math.max(0, Math.min(slider.width - width, fill.width - width / 2))
        anchors.verticalCenter: parent.verticalCenter
        width: 16; height: 16; radius: 8
        color: Theme.graphite ? Theme.surfaceContent : Theme.primary
        border.width: Theme.focusVisible(slider) ? 2 : 0
        border.color: Theme.surface
        scale: drag.pressed ? 1.15 : 1.0
        Behavior on scale {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
        }
    }

    MouseArea {
        id: drag
        anchors.fill: parent
        anchors.margins: -4
        enabled: slider.enabled
        hoverEnabled: true
        cursorShape: Theme.controlCursor
        function valueAt(x) {
            const f = Math.max(0, Math.min(1, (x - 4) / slider.width))
            return slider.from + f * (slider.to - slider.from)
        }
        onPressed: (mouse) => {
            slider.forceActiveFocus(Qt.MouseFocusReason)
            slider.stepped(valueAt(mouse.x))
        }
        onPositionChanged: (mouse) => { if (pressed) slider.stepped(valueAt(mouse.x)) }
    }
}
