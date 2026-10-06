// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// What plays on a phone: artwork, title and artist, a seek bar that moves
// with the music, and the player's controls. Commands go to the phone;
// what it plays next comes back as new state.
Card {
    id: card
    required property string deviceId
    required property string playerId
    required property string app
    required property string title
    required property string artist
    required property bool playing
    required property real duration
    required property real position
    required property real positionAt
    required property string art
    required property bool canPlay
    required property bool canPause
    required property bool canNext
    required property bool canPrevious
    required property bool canSeek

    // Time, ticking while it plays so the bar and clock move.
    property real now: Date.now()
    Timer {
        interval: 250
        repeat: true
        running: card.visible && card.playing && card.duration > 0
        onTriggered: card.now = Date.now()
        onRunningChanged: card.now = Date.now()
    }
    onPositionAtChanged: now = Date.now()

    readonly property real elapsed: position < 0 ? 0
        : Math.min(duration > 0 ? duration : Infinity, position + (playing ? Math.max(0, now - positionAt) : 0))
    // While dragging, the bar shows where the user is pointing.
    property real dragTo: -1
    readonly property real shown: dragTo >= 0 ? dragTo : elapsed

    function send(action, to) { MediaList.command(deviceId, playerId, action, to === undefined ? 0 : to) }
    function clock(ms) {
        const s = Math.floor(ms / 1000)
        const h = Math.floor(s / 3600)
        const m = Math.floor((s % 3600) / 60)
        const ss = String(s % 60).padStart(2, "0")
        return h > 0 ? h + ":" + String(m).padStart(2, "0") + ":" + ss : m + ":" + ss
    }

    Column {
        width: parent.width
        spacing: 14

        Txt { text: qsTr("Now playing"); role: "label"; muted: true }

        Row {
            width: parent.width
            spacing: 14
            Rectangle {
                id: cover
                width: 72; height: 72
                radius: Theme.radiusMd
                color: Theme.secondaryContainer
                clip: true
                Icon {
                    anchors.centerIn: parent
                    width: 28; height: 28
                    visible: artwork.status !== Image.Ready
                    path: Icons.music
                    color: Theme.secondaryContainerContent
                }
                Image {
                    id: artwork
                    anchors.fill: parent
                    source: card.art
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    sourceSize: Qt.size(144, 144)
                    opacity: status === Image.Ready ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
                }
            }
            Column {
                width: parent.width - cover.width - parent.spacing
                anchors.verticalCenter: parent.verticalCenter
                spacing: 3
                Txt {
                    width: parent.width
                    role: "title"
                    text: card.title.length > 0 ? card.title : card.app
                    elide: Text.ElideRight
                    maximumLineCount: 2
                    wrapMode: Text.WordWrap
                }
                Txt {
                    width: parent.width
                    visible: card.artist.length > 0
                    role: "bodySmall"
                    muted: true
                    text: card.artist
                    elide: Text.ElideRight
                }
                Txt {
                    width: parent.width
                    role: "caption"
                    muted: true
                    text: card.app
                    elide: Text.ElideRight
                }
            }
        }

        // ---- Seek bar ----
        Column {
            width: parent.width
            spacing: 4
            visible: card.duration > 0
            Item {
                id: bar
                width: parent.width
                height: 16
                readonly property real fraction: card.duration > 0 ? Math.max(0, Math.min(1, card.shown / card.duration)) : 0
                Rectangle {
                    id: track
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width
                    height: 4
                    radius: 2
                    color: Theme.surfaceContainerHighest
                    Rectangle {
                        width: parent.width * bar.fraction
                        height: parent.height
                        radius: parent.radius
                        color: Theme.primary
                    }
                }
                Rectangle {
                    visible: card.canSeek && (seek.containsMouse || seek.pressed)
                    x: bar.width * bar.fraction - width / 2
                    anchors.verticalCenter: parent.verticalCenter
                    width: 12; height: 12; radius: 6
                    color: Theme.primary
                }
                MouseArea {
                    id: seek
                    anchors.fill: parent
                    anchors.margins: -4
                    enabled: card.canSeek
                    hoverEnabled: true
                    cursorShape: card.canSeek ? Qt.PointingHandCursor : Qt.ArrowCursor
                    function at(x) { return Math.max(0, Math.min(1, (x - 4) / bar.width)) * card.duration }
                    onPressed: (mouse) => card.dragTo = at(mouse.x)
                    onPositionChanged: (mouse) => { if (pressed) card.dragTo = at(mouse.x) }
                    onReleased: (mouse) => {
                        card.send("seek", Math.round(at(mouse.x)))
                        card.dragTo = -1
                    }
                    onCanceled: card.dragTo = -1
                }
            }
            Item {
                width: parent.width
                height: elapsedText.height
                Txt { id: elapsedText; role: "caption"; muted: true; text: card.clock(card.shown) }
                Txt { anchors.right: parent.right; role: "caption"; muted: true; text: card.clock(card.duration) }
            }
        }

        // ---- Controls ----
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 18
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.skipPrevious
                label: qsTr("Previous")
                enabled: card.canPrevious
                onClicked: card.send("previous")
            }
            IconButton {
                id: playPause
                anchors.verticalCenter: parent.verticalCenter
                width: 48; height: 48
                tonal: true
                iconPath: card.playing ? Icons.pause : Icons.play
                label: card.playing ? qsTr("Pause") : qsTr("Play")
                enabled: card.playing ? card.canPause : card.canPlay
                onClicked: card.send(card.playing ? "pause" : "play")
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.skipNext
                label: qsTr("Next")
                enabled: card.canNext
                onClicked: card.send("next")
            }
        }
    }
}
