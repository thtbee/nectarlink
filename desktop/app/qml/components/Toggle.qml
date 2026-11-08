// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A Material switch. Click, Space or Enter flips it; `toggled` reports the
// user's choice (bind `checked` to the source of truth).
FocusScope {
    id: toggle
    property bool checked: false
    property string label
    signal toggled(bool checked)

    implicitWidth: Theme.fluent ? 40 : 46
    implicitHeight: Theme.fluent ? 20 : 28
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.CheckBox
    Accessible.name: label
    Accessible.checkable: true
    Accessible.checked: checked
    Accessible.onToggleAction: toggle.toggled(!toggle.checked)
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            toggle.toggled(!toggle.checked)
            event.accepted = true
        }
    }

    // Hive: a soft honey halo around a switch that's on.
    Rectangle {
        visible: Theme.hive
        anchors.fill: track
        anchors.margins: -3
        radius: track.radius + 3
        color: Theme.glow
        opacity: toggle.checked ? (mouse.containsMouse ? 1 : 0.55) : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
    }

    // Fluent: WinUI's ToggleSwitch. Off, a stroked track with a dark knob; on,
    // the accent with a knob in its text color. The knob grows under the
    // pointer and stretches while pressed.
    Rectangle {
        id: fluentTrack
        visible: Theme.fluent
        anchors.fill: parent
        radius: height / 2
        color: toggle.checked
            ? (mouse.pressed ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.80)
                : (mouse.containsMouse ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.90) : Theme.primary))
            : (mouse.pressed ? Theme.controlAltFill : (mouse.containsMouse ? Theme.subtleFillSecondary : Theme.controlAltFill))
        border.width: toggle.checked ? 0 : 1
        border.color: Theme.controlStrong
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 83 } }

        Rectangle {
            readonly property real size: mouse.pressed || mouse.containsMouse ? 14 : 12
            width: mouse.pressed ? 17 : size
            height: size
            radius: height / 2
            anchors.verticalCenter: parent.verticalCenter
            x: toggle.checked ? parent.width - width - (mouse.containsMouse || mouse.pressed ? 3 : 4)
                              : (mouse.containsMouse || mouse.pressed ? 3 : 4)
            color: toggle.checked ? Theme.primaryContent : Theme.controlStrong
            Behavior on x { enabled: !Theme.reduceMotion; NumberAnimation { duration: 167; easing.type: Easing.OutCubic } }
            Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: 83; easing.type: Easing.OutCubic } }
            Behavior on height { enabled: !Theme.reduceMotion; NumberAnimation { duration: 83; easing.type: Easing.OutCubic } }
        }
    }

    Rectangle {
        id: track
        visible: !Theme.fluent
        anchors.fill: parent
        radius: Theme.graphite ? Theme.radiusSm : height / 2
        color: Theme.glass
            ? (toggle.checked
                ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.62 : 0.76)
                : Qt.rgba(1, 1, 1, Theme.dark ? 0.12 : 0.44))
            : (toggle.checked ? Theme.primary : Theme.surfaceContainerHighest)
        border.width: Theme.glass ? 1 : (toggle.checked ? 0 : 1.5)
        border.color: Theme.glass
            ? (toggle.checked
                ? Qt.rgba(1, 1, 1, Theme.dark ? 0.18 : 0.42)
                : Qt.rgba(1, 1, 1, Theme.dark ? 0.12 : 0.34))
            : Theme.outline
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

        Rectangle {
            id: thumb
            property real size: mouse.pressed ? 20 : (mouse.containsMouse ? 20 : (toggle.checked ? 18 : 16))
            width: mouse.pressed ? 22 : size
            height: size
            radius: Theme.graphite ? Theme.radiusXs : height / 2
            anchors.verticalCenter: parent.verticalCenter
            x: toggle.checked ? parent.width - width - 4.5 : 6.5
            color: Theme.glass
                ? (toggle.checked
                    ? "#FFFFFF"
                    : (Theme.dark ? Qt.rgba(1, 1, 1, mouse.containsMouse ? 0.90 : 0.78) : Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, mouse.containsMouse ? 0.78 : 0.62)))
                : (toggle.checked ? Theme.primaryContent : (mouse.containsMouse ? Theme.surfaceContentVariant : Theme.outline))
            border.width: Theme.glass ? 1 : 0
            border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.55 : 0.90)
            // Quick and settled, never springy: a switch must feel immediate.
            Behavior on x { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.reduceMotion ? 0 : 140; easing.type: Easing.OutCubic } }
            Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
            Behavior on height { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        }
    }
    Rectangle {
        anchors.fill: track
        anchors.margins: -3
        radius: track.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(toggle)
    }
    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Theme.controlCursor
        onClicked: toggle.toggled(!toggle.checked)
    }
}
