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

    implicitWidth: 46
    implicitHeight: 28
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

    Rectangle {
        id: track
        anchors.fill: parent
        radius: Theme.graphite ? Theme.radiusSm : height / 2
        color: toggle.checked ? Theme.primary : Theme.surfaceContainerHighest
        border.width: toggle.checked ? 0 : 1.5
        border.color: Theme.outline
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

        Rectangle {
            id: thumb
            property real size: toggle.checked ? 19 : (tap.pressed ? 22 : 15)
            width: size; height: size
            radius: Theme.graphite ? Theme.radiusXs : size / 2
            anchors.verticalCenter: parent.verticalCenter
            x: toggle.checked ? parent.width - width - 4.5 : 6.5
            color: toggle.checked ? Theme.primaryContent : Theme.outline
            Behavior on x { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
            Behavior on size { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
            Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
        }
    }
    Rectangle {
        anchors.fill: track
        anchors.margins: -3
        radius: track.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: toggle.activeFocus
    }
    HoverHandler { cursorShape: Qt.PointingHandCursor }
    TapHandler { id: tap; onTapped: toggle.toggled(!toggle.checked) }
}
