// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A fake call, labelled as a test, that runs the sound path of a real one
// (Settings › Calls through this PC): the PC's microphone goes to the phone
// and back when it offers the test, else it's recorded and played here. It
// shows the microphone as the system describes it, the levels, and what was
// sent, dropped and returned, so a call that sounds wrong can be looked into.
// Everything it shows comes from PhoneCall.callAudioTest (JSON), polled while
// it's visible.
Card {
    id: card

    // The test as the PC last described it.
    readonly property var t: { try { return JSON.parse(PhoneCall.callAudioTest) } catch (e) { return ({}) } }
    readonly property string stage: t.stage || "idle"
    readonly property bool active: stage === "starting" || stage === "running" || stage === "recording" || stage === "playing"
    readonly property var source: t.source || ({})
    readonly property var latency: t.latency_ms || null
    readonly property var hints: t.hints || []

    Timer {
        interval: 100
        repeat: true
        running: card.visible
        triggeredOnStart: true
        onTriggered: PhoneCall.pollCallAudioTest()
    }
    // Leaving the page ends a test that's still going.
    onVisibleChanged: if (!visible) PhoneCall.stopCallAudioTest()
    Component.onDestruction: PhoneCall.stopCallAudioTest()

    function db(v) { return v <= -89.5 ? "-∞" : v.toFixed(0) }
    function kHz(rate) { return rate % 1000 === 0 ? (rate / 1000) + " kHz" : (rate / 1000).toFixed(1) + " kHz" }
    function channelWord(n) { return n === 1 ? qsTr("mono") : n === 2 ? qsTr("stereo") : qsTr("%1 channels").arg(n) }
    function seconds(ms) { return Math.ceil(ms / 1000) }

    // One value in the list of facts: its name, then what it is.
    component Fact: Row {
        property string label
        property string value
        property bool bad: false
        width: parent.width
        spacing: 8
        Txt { width: 118; role: "caption"; muted: true; text: parent.label; elide: Text.ElideRight }
        Txt {
            width: parent.width - 126
            role: "bodySmall"
            color: parent.bad ? Theme.warning : Theme.surfaceContent
            text: parent.value
            wrapMode: Text.WordWrap
        }
    }

    // A level bar from -60 to 0 dBFS: the loudness (rms) and its peak.
    component LevelBar: Item {
        id: bar
        property string label
        property real rms: -90
        property real peak: -90
        property real loudest: -90
        property bool clipped: false
        width: parent.width
        height: 34
        readonly property real span: 60
        function frac(v) { return Math.max(0, Math.min(1, (v + span) / span)) }
        Txt { id: barLabel; width: 118; role: "caption"; muted: true; text: bar.label; anchors.verticalCenter: parent.verticalCenter }
        Rectangle {
            id: track
            anchors.left: barLabel.right
            anchors.leftMargin: 8
            anchors.right: reading.left
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            height: 12
            radius: height / 2
            color: Theme.surfaceContainerHighest
            Rectangle {
                width: parent.width * bar.frac(bar.rms)
                height: parent.height
                radius: parent.radius
                color: bar.clipped || bar.peak > -1 ? Theme.error : bar.peak > -6 ? Theme.warning : Theme.primary
                Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: 90 } }
            }
            // The peak, a tick that falls back slowly.
            Rectangle {
                x: Math.min(parent.width - width, parent.width * bar.frac(bar.peak))
                width: 3
                height: parent.height + 6
                y: -3
                radius: 1.5
                color: Theme.surfaceContent
                visible: bar.peak > -89
                Behavior on x { enabled: !Theme.reduceMotion; NumberAnimation { duration: 160 } }
            }
        }
        Txt {
            id: reading
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            width: 92
            horizontalAlignment: Text.AlignRight
            role: "caption"
            text: card.db(bar.rms) + " / " + card.db(bar.peak) + " dB"
        }
    }

    Column {
        width: parent.width
        spacing: 14

        Item {
            width: parent.width
            height: heading.height
            Txt { id: heading; text: qsTr("Test call"); role: "label"; muted: true }
            Txt {
                anchors.right: parent.right
                role: "label"
                color: card.stage === "failed" ? Theme.error : Theme.surfaceContentVariant
                text: card.stage === "starting" ? qsTr("Starting…")
                    : card.stage === "running" ? qsTr("Live · %1 s left").arg(card.seconds(card.t.remaining_ms || 0))
                    : card.stage === "recording" ? qsTr("Recording · %1 s").arg(card.seconds(card.t.remaining_ms || 0))
                    : card.stage === "playing" ? qsTr("Playing back")
                    : card.stage === "done" ? qsTr("Finished")
                    : card.stage === "failed" ? qsTr("Failed") : ""
            }
        }

        Row {
            width: parent.width
            spacing: 12
            Avatar {
                id: avatar
                anchors.verticalCenter: parent.verticalCenter
                width: 44; height: 44
                iconPath: Icons.call
                emphasized: true
            }
            Column {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - avatar.width - parent.spacing
                spacing: 2
                Txt { width: parent.width; role: "title"; text: qsTr("Test call audio"); elide: Text.ElideRight }
                Txt {
                    width: parent.width
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.WordWrap
                    text: qsTr("Not a real call: nobody is called, nothing is recorded or saved.")
                }
            }
        }

        // Which test ran, and why.
        Rectangle {
            visible: card.t.mode === "loopback" || card.t.mode === "local"
            width: parent.width
            height: modeText.height + 16
            radius: Theme.radiusMd
            color: Theme.surfaceContainerHigh
            Txt {
                id: modeText
                x: 12; y: 8
                width: parent.width - 24
                role: "bodySmall"
                wrapMode: Text.WordWrap
                text: card.t.mode === "loopback"
                    ? qsTr("Through your phone: your voice goes to the phone, plays on its speaker and comes back here.")
                    : card.t.reason === "offline"
                        ? qsTr("This PC only: no phone is connected. It records 5 seconds, then plays them back.")
                    : card.t.reason === "refused"
                        ? qsTr("This PC only: the phone didn't take the test. It records 5 seconds, then plays them back.")
                        : qsTr("This PC only: the phone's NectarLink doesn't offer this test (update it, or check that calls through this PC are on for it). It records 5 seconds, then plays them back.")
            }
        }

        // ---- What the PC records from and plays on ----
        Column {
            width: parent.width
            spacing: 6
            Fact {
                label: qsTr("Microphone")
                value: card.source.name && card.source.name.length > 0 ? card.source.name : qsTr("System default")
            }
            Fact {
                label: qsTr("Its own format")
                visible: card.source.native_rate !== undefined && card.source.native_rate !== null
                value: card.kHz(card.source.native_rate || 0)
                    + (card.source.native_channels ? ", " + card.channelWord(card.source.native_channels) : "")
                    + (card.source.bluetooth ? " · " + qsTr("Bluetooth") : "")
            }
            Fact {
                label: qsTr("Sent to the call")
                value: card.kHz(card.source.rate || 16000) + ", " + card.channelWord(card.source.channels || 1) + ", 16-bit"
            }
            Fact {
                visible: card.source.system_volume !== undefined && card.source.system_volume !== null
                label: qsTr("Input volume")
                value: (card.source.system_volume || 0) + " %"
                bad: (card.source.system_volume || 0) < 25
            }
            Fact {
                label: qsTr("Plays on")
                value: card.t.output && card.t.output.length > 0 ? card.t.output : qsTr("System default output")
            }
        }

        // ---- Levels ----
        Column {
            width: parent.width
            spacing: 2
            LevelBar {
                label: qsTr("Your microphone")
                rms: card.t.input ? card.t.input.rms : -90
                peak: card.t.input ? card.t.input.peak : -90
                clipped: (card.t.clipped || 0) > 0
            }
            LevelBar {
                visible: card.stage === "running" || card.stage === "playing" || (card.t.returned_chunks || 0) > 0
                label: card.t.mode === "loopback" ? qsTr("Back from the phone") : qsTr("Played back")
                rms: card.t.returned ? card.t.returned.rms : -90
                peak: card.t.returned ? card.t.returned.peak : -90
            }
            Txt {
                width: parent.width
                role: "caption"
                muted: true
                text: qsTr("Loudness / peak, in dBFS (0 is the loudest). Speaking normally reads about -30 to -12.")
                wrapMode: Text.WordWrap
            }
        }

        // ---- What went where ----
        Column {
            width: parent.width
            spacing: 6
            Fact {
                label: qsTr("Clipping")
                value: (card.t.clipped || 0) > 0 ? qsTr("Yes, %1 samples cut off").arg(card.t.clipped) : qsTr("No")
                bad: (card.t.clipped || 0) > 0
            }
            Fact {
                label: qsTr("Sent")
                value: qsTr("%1 chunks, %2 KB").arg(card.t.sent_chunks || 0).arg(Math.round((card.t.sent_bytes || 0) / 1024))
            }
            Fact {
                visible: card.t.mode === "loopback"
                label: qsTr("Dropped")
                value: qsTr("%1 chunks").arg(card.t.dropped || 0)
                bad: (card.t.dropped || 0) > 0
            }
            Fact {
                visible: card.t.mode === "loopback"
                label: qsTr("Back")
                value: qsTr("%1 chunks · %2 late · %3 ms missing").arg(card.t.returned_chunks || 0).arg(card.t.late || 0).arg(card.t.lost_ms || 0)
                bad: (card.t.late || 0) > 0 || (card.t.lost_ms || 0) > 0
            }
            Fact {
                visible: card.t.mode === "loopback"
                label: qsTr("Round trip")
                value: card.latency
                    ? qsTr("%1 ms now · %2 ms average · %3 ms at worst").arg(card.latency.last || 0).arg(card.latency.average).arg(card.latency.max)
                    : qsTr("Not measured yet")
            }
        }

        // ---- What it means ----
        Column {
            visible: card.hints.length > 0 || !!(card.t.error && card.t.error.length > 0)
            width: parent.width
            spacing: 4
            Txt {
                visible: !!(card.t.error && card.t.error.length > 0)
                width: parent.width
                role: "bodySmall"
                color: Theme.error
                wrapMode: Text.WordWrap
                text: card.t.error || ""
            }
            Repeater {
                model: card.hints
                delegate: Txt {
                    required property string modelData
                    width: parent.width
                    role: "bodySmall"
                    color: Theme.warning
                    wrapMode: Text.WordWrap
                    text: modelData === "quiet" ? qsTr("The microphone is almost silent. Check the input volume, that it isn't muted, and that it's the microphone you mean.")
                        : modelData === "clipping" ? qsTr("The sound is cut off at its loudest. Lower the microphone's input volume.")
                        : modelData === "bluetooth" ? qsTr("A Bluetooth headset's microphone is low quality (8 or 16 kHz), and playing sound on it at the same time makes the call sound worse. A wired or built-in microphone sounds better.")
                        : modelData === "low_volume" ? qsTr("The system's input volume is low. Raise it in the system's sound settings.")
                        : modelData === "dropped" ? qsTr("Some sound was dropped because the connection couldn't keep up.")
                        : modelData === "link" ? qsTr("Sound came back late or not at all: the connection to the phone is slow or unstable.")
                        : ""
                }
            }
        }

        Txt {
            visible: card.t.mode === "loopback" && card.stage === "running"
            width: parent.width
            role: "caption"
            muted: true
            wrapMode: Text.WordWrap
            text: qsTr("Speak now. Your phone plays what it hears, and what comes back plays on this PC. Use headphones here, or the speakers will feed back into the microphone.")
        }

        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 12
            Button {
                visible: !card.active
                variant: "tonal"
                iconPath: Icons.play
                text: card.stage === "idle" ? qsTr("Start test") : qsTr("Test again")
                onClicked: PhoneCall.startCallAudioTest()
            }
            Button {
                visible: card.active
                variant: "tonal"
                iconPath: Icons.stopSquare
                text: qsTr("Stop test")
                onClicked: PhoneCall.stopCallAudioTest()
            }
        }
    }
}
