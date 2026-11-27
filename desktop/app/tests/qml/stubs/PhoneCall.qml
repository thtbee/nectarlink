// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust PhoneCall in QML tests: no call.
QtObject {
    property bool active: false
    property bool ringing: false
    property bool held: false
    property bool muted: false
    property bool canEnd: false
    property bool controls: false
    property string caller: ""
    property string number: ""
    // The call audio test: what the PC last described, and what was asked of it.
    property string callAudioTest: "{}"
    property int testStarts: 0
    property int testStops: 0
    property int testPolls: 0
    function startCallAudioTest() { testStarts++ }
    function stopCallAudioTest() { testStops++ }
    function pollCallAudioTest() { testPolls++ }
    function answer() {}
    function end() {}
    function setMute(on) {}
}
