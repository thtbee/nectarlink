// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A short message at the bottom of the window that hides itself, with an
// optional context action chip (e.g. "Open", "Call", "Track").
Rectangle {
    id: toast
    property int timeout: 4000
    property int actionTimeout: 6500
    property string actionText: ""
    signal actionTriggered()

    function show(message) {
        label.text = message
        actionText = ""
        shown = true
        timer.interval = toast.timeout
        timer.restart()
    }

    function showWithAction(message, actionLabel) {
        label.text = message
        actionText = actionLabel || ""
        shown = true
        timer.interval = actionText.length > 0 ? toast.actionTimeout : toast.timeout
        timer.restart()
    }

    property bool shown: false
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: shown ? 24 : -height
    Behavior on anchors.bottomMargin { SpringAnimation { spring: Theme.springStandard; damping: Theme.dampingStandard } }
    opacity: shown ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeFast } }
    z: 200

    readonly property bool hasAction: actionText.length > 0
    width: Math.min(contentRow.implicitWidth + (hasAction ? 28 : 40), parent.width - 48)
    height: 44
    radius: Theme.pill(height)
    color: Theme.surfaceContent

    Accessible.role: Accessible.AlertMessage
    Accessible.name: hasAction ? (label.text + " " + actionText) : label.text

    Row {
        id: contentRow
        anchors.centerIn: parent
        spacing: 12

        Txt {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, toast.parent.width - (toast.hasAction ? actionChip.width + 100 : 88))
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            role: "body"
            color: Theme.surface
        }

        Rectangle {
            id: actionChip
            visible: toast.hasAction
            anchors.verticalCenter: parent.verticalCenter
            width: chipLabel.implicitWidth + 24
            height: 30
            radius: Theme.pill(height)
            color: chipHover.hovered
                ? Qt.rgba(Theme.primaryContainer.r, Theme.primaryContainer.g, Theme.primaryContainer.b, 0.92)
                : Theme.primaryContainer
            border.width: Theme.focusVisible(actionChip) ? 2 : 0
            border.color: Theme.surface
            activeFocusOnTab: toast.shown && toast.hasAction

            Accessible.role: Accessible.Button
            Accessible.name: toast.actionText
            Accessible.onPressAction: trigger()

            function trigger() {
                toast.shown = false
                timer.stop()
                toast.actionTriggered()
            }

            Keys.onPressed: (event) => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                    trigger()
                    event.accepted = true
                }
            }

            Txt {
                id: chipLabel
                anchors.centerIn: parent
                text: toast.actionText
                role: "label"
                color: Theme.primaryContainerContent
            }

            HoverHandler { id: chipHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: actionChip.trigger() }
        }
    }

    // Stays while the pointer is on it, so its action can be reached.
    HoverHandler {
        onHoveredChanged: if (hovered) timer.stop(); else if (toast.shown) timer.restart()
    }
    Timer { id: timer; interval: toast.timeout; onTriggered: toast.shown = false }
}

