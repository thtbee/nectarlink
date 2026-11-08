// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A navigation rail destination: an icon in a pill indicator above a label.
FocusScope {
    id: item
    property string iconPath
    property string text
    property bool selected: false
    readonly property alias hovered: mouse.containsMouse
    property bool activeHover: mouse.containsMouse
    signal clicked

    implicitWidth: 72
    implicitHeight: 56
    activeFocusOnTab: true

    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: selected
    Accessible.onPressAction: item.clicked()
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            item.clicked()
            event.accepted = true
        }
    }

    // Fluent: the whole item gets a subtle fill on hover and when selected, and
    // the selected one an accent pill at its left edge, as Windows 11's own
    // apps draw their navigation.
    Rectangle {
        visible: Theme.fluent
        anchors.fill: parent
        anchors.leftMargin: 2
        anchors.rightMargin: 2
        radius: 4
        color: item.selected
            ? (mouse.pressed ? Theme.subtleFillTertiary : Theme.subtleFillSecondary)
            : (mouse.pressed ? Theme.subtleFillTertiary : (item.activeHover ? Theme.subtleFillSecondary : "transparent"))
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 83 } }
        Rectangle {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: 3
            height: mouse.pressed ? 10 : 16
            radius: 1.5
            color: Theme.primary
            opacity: item.selected ? 1 : 0
            Behavior on height { enabled: !Theme.reduceMotion; NumberAnimation { duration: 167; easing.type: Easing.OutCubic } }
            Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: 167 } }
        }
    }

    Rectangle {
        id: pill
        anchors.horizontalCenter: parent.horizontalCenter
        y: Theme.fluent ? 6 : 2
        width: 52; height: Theme.fluent ? 24 : 30
        radius: Theme.pill(height)
        color: "transparent"
        scale: mouse.pressed ? Theme.pressScale : 1.0
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

        Rectangle {
            anchors.centerIn: parent
            height: parent.height
            width: item.selected ? parent.width : (item.activeHover ? 42 : 24)
            radius: parent.radius
            visible: !Theme.fluent
            color: item.selected
                ? (Theme.glass
                    ? Qt.tint(Qt.rgba(1, 1, 1, Theme.dark ? 0.14 : 0.62), Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.28 : 0.22))
                    : Theme.indicator)
                : Theme.surfaceContent
            opacity: item.selected ? 1.0 : (item.activeHover ? 0.08 : 0.0)
            border.width: Theme.glass && item.selected ? 1 : 0
            border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.28 : 0.78)
            Behavior on width {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
            }
            Behavior on opacity {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic }
            }
            Behavior on color {
                enabled: !Theme.reduceMotion
                ColorAnimation { duration: Theme.fadeFast }
            }
        }

        Icon {
            anchors.centerIn: parent
            path: item.iconPath
            color: Theme.fluent
                ? (item.selected ? Theme.primary : Theme.surfaceContent)
                : (item.selected ? Theme.indicatorContent : (item.activeHover ? Theme.surfaceContent : Theme.surfaceContentVariant))
            scale: Theme.fluent ? 1.0 : (item.selected ? 1.05 : (item.activeHover ? 1.02 : 1.0))
            Behavior on scale {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
            }
            Behavior on color {
                enabled: !Theme.reduceMotion
                ColorAnimation { duration: Theme.fadeFast }
            }
        }
    }
    Txt {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: pill.bottom
        anchors.topMargin: 4
        text: item.text
        role: "caption"
        weight: Theme.graphite || Theme.fluent ? 400 : (item.selected ? 600 : 500)
        color: Theme.fluent ? Theme.surfaceContent
            : (item.selected || item.activeHover ? Theme.surfaceContent : Theme.surfaceContentVariant)
        Behavior on color {
            enabled: !Theme.reduceMotion
            ColorAnimation { duration: Theme.fadeFast }
        }
    }
    Rectangle {
        anchors.fill: Theme.fluent ? item : pill
        anchors.margins: Theme.fluent ? 0 : -3
        radius: Theme.fluent ? 6 : pill.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(item)
    }
    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Theme.controlCursor
        onClicked: item.clicked()
    }
}
