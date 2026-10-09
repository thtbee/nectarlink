// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The call in progress on the phone: who, for how long, and its controls.
// The call's audio stays on the phone; these change what the phone does.
Card {
    id: card

    // Ticks once a second for the call's clock.
    property real now: Date.now()
    Timer {
        interval: 1000
        repeat: true
        running: card.visible && PhoneCall.since > 0
        triggeredOnStart: true
        onTriggered: card.now = Date.now()
    }
    property bool keypad: false
    // Keys pressed on this call, as the phone's keypad would show them.
    property string pressed: ""
    Connections {
        target: PhoneCall
        function onActiveChanged() {
            card.keypad = false
            card.pressed = ""
        }
    }

    function clock(ms) {
        const s = Math.max(0, Math.floor(ms / 1000))
        const h = Math.floor(s / 3600)
        const m = Math.floor((s % 3600) / 60)
        const ss = String(s % 60).padStart(2, "0")
        return h > 0 ? h + ":" + String(m).padStart(2, "0") + ":" + ss : m + ":" + ss
    }

    // A round toggle with its name under it.
    component Control: Column {
        id: control
        property string iconPath
        property string label
        property bool on: false
        signal clicked
        width: 60
        spacing: 6
        IconButton {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 44; height: 44
            tonal: !control.on
            iconPath: control.iconPath
            label: control.label
            iconColor: control.on ? (Theme.graphite ? Theme.surface : Theme.primaryContent)
                                  : Theme.secondaryContainerContent
            Rectangle {
                z: -1
                anchors.fill: parent
                radius: Theme.pill(height)
                visible: control.on
                color: Theme.graphite ? Theme.surfaceContent : Theme.primary
            }
            onClicked: control.clicked()
        }
        Txt {
            anchors.horizontalCenter: parent.horizontalCenter
            role: "caption"
            muted: !control.on
            text: control.label
        }
    }

    Column {
        width: parent.width
        spacing: 14

        Item {
            width: parent.width
            height: heading.height
            Txt { id: heading; text: qsTr("On a call"); role: "label"; muted: true }
            Txt {
                anchors.right: parent.right
                role: "label"
                color: PhoneCall.held ? Theme.warning : Theme.surfaceContentVariant
                text: PhoneCall.held ? qsTr("On hold")
                    : PhoneCall.since > 0 ? card.clock(card.now - PhoneCall.since) : ""
            }
        }

        Row {
            width: parent.width
            spacing: 12
            Avatar {
                id: avatar
                anchors.verticalCenter: parent.verticalCenter
                width: 44; height: 44
                // A number alone has no letter worth showing.
                name: PhoneCall.number.length > 0 ? PhoneCall.caller : ""
                iconPath: PhoneCall.number.length > 0 ? "" : Icons.person
                emphasized: true
            }
            Column {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - avatar.width - parent.spacing
                spacing: 2
                Txt {
                    width: parent.width
                    role: "title"
                    text: PhoneCall.caller
                    elide: Text.ElideRight
                }
                Txt {
                    width: parent.width
                    visible: text.length > 0
                    role: "bodySmall"
                    muted: true
                    text: card.pressed.length > 0 ? card.pressed : PhoneCall.number
                    elide: Text.ElideLeft
                }
            }
        }

        // ---- What the phone does on the call ----
        Row {
            visible: PhoneCall.controls
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 6
            Control {
                iconPath: PhoneCall.muted ? Icons.micOff : Icons.mic
                label: PhoneCall.muted ? qsTr("Muted") : qsTr("Mute")
                on: PhoneCall.muted
                onClicked: PhoneCall.setMute(!PhoneCall.muted)
            }
            Control {
                iconPath: Icons.speaker
                label: qsTr("Speaker")
                on: PhoneCall.speaker
                onClicked: PhoneCall.setSpeakerOn(!PhoneCall.speaker)
            }
            Control {
                visible: PhoneCall.canHold || PhoneCall.held
                iconPath: Icons.pause
                label: PhoneCall.held ? qsTr("Resume") : qsTr("Hold")
                on: PhoneCall.held
                onClicked: PhoneCall.setHold(!PhoneCall.held)
            }
            Control {
                iconPath: Icons.dialpad
                label: qsTr("Keypad")
                on: card.keypad
                onClicked: card.keypad = !card.keypad
            }
        }

        Grid {
            id: keys
            visible: PhoneCall.controls && card.keypad
            anchors.horizontalCenter: parent.horizontalCenter
            columns: 3
            columnSpacing: 10
            rowSpacing: 8
            Repeater {
                model: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"]
                delegate: Rectangle {
                    id: key
                    required property string modelData
                    width: 64; height: 40
                    radius: Theme.pill(height)
                    color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                    border.width: Theme.graphite ? 1 : 0
                    border.color: Theme.outlineVariant
                    scale: keyTap.pressed ? Theme.pressScale : 1
                    Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
                    activeFocusOnTab: true
                    function press() {
                        PhoneCall.press(modelData)
                        card.pressed = (card.pressed + modelData).slice(-24)
                    }
                    Accessible.role: Accessible.Button
                    Accessible.name: modelData
                    Accessible.onPressAction: press()
                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            key.press()
                            event.accepted = true
                        }
                    }
                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -3
                        radius: parent.radius + 3
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        visible: Theme.focusVisible(key)
                    }
                    Txt { anchors.centerIn: parent; role: "title"; text: key.modelData }
                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { id: keyTap; onTapped: key.press() }
                }
            }
        }

        Txt {
            width: parent.width
            visible: !PhoneCall.controls && PhoneCall.canEnd
            role: "caption"
            muted: true
            wrapMode: Text.WordWrap
            text: qsTr("Mute, the speaker and the keypad work from here once Wireless debugging is set up in Nectarlink on the phone (Android 12 or later).")
        }

        // ---- Volume and hanging up ----
        Row {
            visible: PhoneCall.canEnd
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 18
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.volumeDown
                label: qsTr("Call volume down")
                onClicked: PhoneCall.volume(false)
            }
            FocusScope {
                id: end
                anchors.verticalCenter: parent.verticalCenter
                width: 72; height: 44
                activeFocusOnTab: true
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("End call")
                Accessible.onPressAction: PhoneCall.end()
                Keys.onPressed: (event) => {
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                        PhoneCall.end()
                        event.accepted = true
                    }
                }
                Rectangle {
                    id: endBody
                    anchors.fill: parent
                    radius: Theme.pill(height)
                    color: Theme.error
                    scale: endTap.pressed ? Theme.pressScale : 1
                    Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }
                    Rectangle {
                        anchors.fill: parent
                        radius: parent.radius
                        color: Theme.errorContent
                        opacity: endTap.pressed ? 0.16 : (endHover.hovered ? 0.08 : 0)
                    }
                    Icon {
                        anchors.centerIn: parent
                        width: 22; height: 22
                        path: Icons.callEnd
                        color: Theme.errorContent
                    }
                }
                Rectangle {
                    anchors.fill: endBody
                    anchors.margins: -3
                    radius: endBody.radius + 3
                    color: "transparent"
                    border.width: 2
                    border.color: Theme.primary
                    visible: Theme.focusVisible(end)
                }
                HoverHandler { id: endHover; cursorShape: Qt.PointingHandCursor }
                TapHandler { id: endTap; onTapped: PhoneCall.end() }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.volumeUp
                label: qsTr("Call volume up")
                onClicked: PhoneCall.volume(true)
            }
        }
    }
}
