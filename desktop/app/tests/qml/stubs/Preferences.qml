// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust Preferences in QML tests: Bloom, light.
QtObject {
    property string theme: "bloom"
    property string colorMode: "light"
    property bool amoled: false
    property bool enterSends: false
    property bool starPromptDone: false
    property bool starPromptUpdated: false
    property int starPromptMinutes: 0
    property bool communityPromptDone: false
    property bool communityPromptDue: false
    property int communityPromptCount: 0
    property bool quietWhenLocked: false
    property int uiScale: 100
    property bool combMotion: false
    property string seed: ""
    property string commandPaletteHotkey: ""
    property bool streamPcAudioToPhone: false
    property bool nectarIsland: true
    property bool nectarIslandCalls: true
    property bool nectarIslandMedia: true
    property string nectarIslandPosition: "center"
    property string nectarIslandLook: "dark"
    property bool nectarIslandPcMedia: false
    property bool nectarIslandNotifications: true
    property bool nectarIslandTransfers: true
    // The Island's own appearance and placement, with the same defaults as
    // desktop/app/src/settings.rs.
    property bool nectarIslandAlwaysVisible: true
    property bool nectarIslandClickThrough: false
    property bool nectarIslandClock: true
    property bool nectarIslandClock12h: false
    property string nectarIslandDisplay: ""
    property string nectarIslandEdge: "top"
    property bool nectarIslandHasPosition: false
    property string nectarIslandMediaSource: "phone"
    property int nectarIslandOpacity: 100
    property string nectarIslandShape: "capsule"
    property int nectarIslandSize: 100
    property string nectarIslandSkin: "classic"
    property real nectarIslandX: 0
    property real nectarIslandY: 0
    function saveIslandPosition(present, x, y) {}
}
