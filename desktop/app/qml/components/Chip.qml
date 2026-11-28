// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A small pill with optional icon. `selected` uses the indicator colors.
Rectangle {
    id: chip
    property string text
    property string iconPath
    property bool selected: false
    property bool interactive: false
    signal clicked()

    implicitHeight: 30
    implicitWidth: row.implicitWidth + 24
    radius: Theme.pill(height)
    color: Theme.graphite
        ? (selected ? Theme.surfaceContent : "transparent")
        : (Theme.glass
            ? (selected
                ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, chip.interactive && mouse.pressed ? (Theme.dark ? 0.38 : 0.32) : (chip.interactive && mouse.containsMouse ? (Theme.dark ? 0.33 : 0.28) : (Theme.dark ? 0.28 : 0.22)))
                : (chip.interactive && mouse.pressed
                    ? Qt.rgba(1, 1, 1, Theme.dark ? 0.22 : 0.82)
                    : (chip.interactive && mouse.containsMouse
                        ? Qt.rgba(1, 1, 1, Theme.dark ? 0.18 : 0.74)
                        : Theme.tileColor)))
            : (selected ? Theme.secondaryContainer : Theme.surfaceContainerHigh))
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.glass ? 190 : Theme.fadeFast; easing.type: Easing.OutCubic } }
    border.width: Theme.graphite || Theme.glass ? 1 : 0
    border.color: Theme.glass
        ? (selected
            ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, chip.interactive && mouse.containsMouse ? 0.76 : 0.56)
            : Qt.rgba(1, 1, 1, chip.interactive && mouse.containsMouse ? (Theme.dark ? 0.34 : 0.88) : (Theme.dark ? 0.20 : 0.66)))
        : (selected ? Theme.surfaceContent : Theme.outlineVariant)
    Behavior on border.color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 190; easing.type: Easing.OutCubic } }
    scale: chip.interactive && mouse.pressed && !Theme.reduceMotion ? Theme.pressScale : 1.0
    Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.glass ? 140 : 120; easing.type: Easing.OutCubic } }

    GlassShadow {
        visible: Theme.glass && !Theme.amoled && chip.interactive
        radius: chip.radius
        elevation: mouse.containsMouse ? 0.58 : 0.42
    }

    Rectangle {
        visible: Theme.glass
        z: -1
        anchors.fill: parent
        anchors.margins: -1
        radius: chip.radius + 1
        color: "transparent"
        border.width: 1
        border.color: Theme.dark ? Qt.rgba(0, 0, 0, 0.28) : Qt.rgba(0, 0, 0, 0.07)
    }

    GlassSheen {
        radius: chip.radius
        strength: chip.interactive && mouse.containsMouse ? 1.32 : 1.0
    }

    activeFocusOnTab: interactive && visible && enabled
    Accessible.role: interactive ? Accessible.Button : Accessible.StaticText
    Accessible.name: chip.text
    Accessible.onPressAction: if (chip.interactive) chip.clicked()
    Keys.onPressed: (event) => {
        if (chip.interactive && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            chip.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: chip.radius
        color: label.color
        opacity: !chip.interactive ? 0 : (Theme.glass ? (mouse.pressed ? 0.06 : 0) : (mouse.pressed ? 0.14 : (mouse.containsMouse ? 0.07 : 0)))
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -3
        radius: chip.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: chip.interactive && Theme.focusVisible(chip)
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 6
        Icon {
            visible: chip.iconPath.length > 0
            anchors.verticalCenter: parent.verticalCenter
            width: 15; height: 15
            path: chip.iconPath
            color: label.color
        }
        Txt {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            text: chip.text
            role: "label"
            color: Theme.graphite ? (chip.selected ? Theme.surface : Theme.surfaceContent)
                                  : (chip.selected ? Theme.secondaryContainerContent : Theme.surfaceContent)
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        enabled: chip.interactive
        hoverEnabled: chip.interactive
        cursorShape: chip.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: chip.clicked()
    }
}
