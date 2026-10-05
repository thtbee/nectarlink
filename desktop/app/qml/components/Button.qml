// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A pill button. Variants: "fill" (primary action), "tonal", "outline",
// "text". Sizes: "md" (40 px) and "sm" (32 px). Works with mouse, touch and
// keyboard (Tab to focus, Enter or Space to press).
FocusScope {
    id: button
    property string text
    property string iconPath
    property string variant: "fill"
    property string size: "md"
    property bool busy: false
    signal clicked

    readonly property bool small: size === "sm"
    readonly property bool interactive: enabled && !busy
    readonly property color background: {
        if (variant === "fill")
            return Theme.graphite ? Theme.surfaceContent : Theme.primary
        if (variant === "tonal")
            return Theme.secondaryContainer
        return "transparent"
    }
    readonly property color foreground: {
        if (variant === "fill")
            return Theme.graphite ? Theme.surface : Theme.primaryContent
        if (variant === "tonal")
            return Theme.secondaryContainerContent
        if (variant === "text")
            return Theme.graphite ? Theme.surfaceContent : Theme.primary
        return Theme.surfaceContent
    }

    implicitHeight: small ? 32 : 40
    implicitWidth: row.implicitWidth + (variant === "text" ? 20 : (small ? 26 : 36))
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.onPressAction: if (interactive) button.clicked()
    Keys.onPressed: (event) => {
        if (interactive && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            button.clicked()
            event.accepted = true
        }
    }

    // Graphite's flat offset shadow under filled buttons.
    Rectangle {
        visible: Theme.graphite && button.variant === "fill"
        x: 3; y: 3
        width: parent.width; height: parent.height
        radius: body.radius
        color: Theme.buttonShadow
    }

    Rectangle {
        id: body
        width: parent.width
        height: parent.height
        radius: Theme.pill(height)
        color: button.background
        border.width: button.variant === "outline" ? 1 : 0
        border.color: Theme.outline
        scale: tap.pressed && button.interactive ? Theme.pressScale : 1
        Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

        // Hover and press state layer.
        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: button.foreground
            opacity: !button.interactive ? 0 : (tap.pressed ? 0.12 : (hover.hovered ? 0.08 : 0))
            Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
        }

        Row {
            id: row
            anchors.centerIn: parent
            spacing: 8
            Spinner {
                visible: button.busy
                anchors.verticalCenter: parent.verticalCenter
                width: 16; height: 16
                color: button.foreground
            }
            Icon {
                visible: button.iconPath.length > 0 && !button.busy
                anchors.verticalCenter: parent.verticalCenter
                width: button.small ? 16 : 18; height: width
                path: button.iconPath
                color: button.foreground
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: button.text
                role: "label"
                color: button.foreground
                size: Theme.graphite ? 11 : (button.small ? 13 : 14)
                font.underline: Theme.graphite && button.variant === "text"
            }
        }
    }

    // Keyboard focus ring.
    Rectangle {
        anchors.fill: body
        anchors.margins: -3
        radius: body.radius + 3
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: button.activeFocus
    }

    HoverHandler { id: hover; cursorShape: button.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor }
    TapHandler {
        id: tap
        enabled: button.interactive
        onTapped: button.clicked()
    }
}
