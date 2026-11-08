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
        if (Theme.fluent) {
            if (variant === "fill")
                return mouse.pressed ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.80)
                    : (mouse.containsMouse ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.90) : Theme.primary)
            if (variant === "text")
                return mouse.pressed ? Theme.subtleFillTertiary : (mouse.containsMouse ? Theme.subtleFillSecondary : "transparent")
            return mouse.pressed ? Theme.controlFillPressed : (mouse.containsMouse ? Theme.controlFillHover : Theme.controlFill)
        }
        if (variant === "fill")
            return Theme.graphite ? Theme.surfaceContent : Theme.primary
        if (variant === "tonal") {
            if (Theme.glass)
                return Qt.tint(
                    Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.54),
                    Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.18 : 0.14)
                )
            return Theme.secondaryContainer
        }
        return "transparent"
    }
    readonly property color foreground: {
        if (Theme.fluent) {
            if (variant === "fill")
                return mouse.pressed ? Qt.rgba(Theme.primaryContent.r, Theme.primaryContent.g, Theme.primaryContent.b, 0.70) : Theme.primaryContent
            return mouse.pressed ? Theme.surfaceContentVariant : Theme.surfaceContent
        }
        if (variant === "fill")
            return Theme.graphite ? Theme.surface : Theme.primaryContent
        if (variant === "tonal")
            return Theme.glass ? Theme.surfaceContent : Theme.secondaryContainerContent
        if (variant === "text")
            return Theme.graphite ? Theme.surfaceContent : Theme.primary
        return Theme.surfaceContent
    }

    implicitHeight: Theme.fluent ? (small ? 28 : 32) : (small ? 32 : 40)
    implicitWidth: row.implicitWidth + (variant === "text" ? 20 : (Theme.fluent ? 24 : (small ? 26 : 36)))
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

    // Hive: a soft honey halo around a filled button under the pointer.
    Rectangle {
        visible: Theme.hive && button.variant === "fill"
        x: -4; y: -4
        width: parent.width + 8; height: parent.height + 8
        radius: body.radius + 4
        color: Theme.glow
        opacity: button.interactive && (mouse.containsMouse || Theme.focusVisible(button)) ? 1 : 0
        scale: body.scale
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
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
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        border.width: Theme.fluent ? (button.variant === "text" ? 0 : 1)
            : (button.variant === "outline" || (Theme.glass && (button.variant === "tonal" || button.variant === "fill")) ? 1 : 0)
        border.color: Theme.fluent
            ? (button.variant === "fill" ? Qt.rgba(1, 1, 1, 0.08) : Theme.controlStroke)
            : button.variant === "outline"
            ? (Theme.glass ? Qt.rgba(1, 1, 1, Theme.dark ? 0.14 : 0.38)
                : (Theme.hive ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, Theme.dark ? 0.42 : 0.38) : Theme.outline))
            : Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.32)
        scale: mouse.pressed && button.interactive ? Theme.pressScale : 1
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

        // Hive: honey catches the light along the top of a filled button.
        Rectangle {
            visible: Theme.hive && button.variant === "fill"
            anchors.fill: parent
            radius: parent.radius
            gradient: Gradient {
                GradientStop { position: 0.0; color: Qt.rgba(1, 1, 1, Theme.dark ? 0.22 : 0.16) }
                GradientStop { position: 0.55; color: Qt.rgba(1, 1, 1, 0) }
            }
        }

        // Fluent: the darker hairline along the bottom that makes a control look raised.
        Rectangle {
            visible: Theme.fluent && button.variant !== "text" && !mouse.pressed
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.leftMargin: parent.radius
            anchors.rightMargin: parent.radius
            height: 1
            color: button.variant === "fill" ? Qt.rgba(0, 0, 0, Theme.dark ? 0.14 : 0.40) : Theme.controlStrokeBottom
        }

        // Hover & Press state layer (Fluent changes the fill itself).
        Rectangle {
            visible: !Theme.fluent
            anchors.fill: parent
            radius: parent.radius
            color: button.foreground
            opacity: !button.interactive ? 0 : (mouse.pressed ? 0.14 : (mouse.containsMouse ? 0.07 : 0))
            Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
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
                weight: Theme.fluent ? 400 : (Theme.graphite ? Theme.labelStyle.weight : Theme.typography.label.weight)
                font.underline: Theme.graphite && button.variant === "text"
                maximumLineCount: 1
                elide: Text.ElideRight
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
        visible: Theme.focusVisible(button)
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        enabled: button.interactive
        hoverEnabled: true
        cursorShape: button.interactive ? Theme.controlCursor : Qt.ArrowCursor
        onClicked: button.clicked()
    }
}
