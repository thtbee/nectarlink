// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust Preferences in QML tests: Bloom, light.
QtObject {
    property string theme: "bloom"
    property string colorMode: "light"
    property bool amoled: false
    property int uiScale: 100
    property bool combMotion: false
    property string seed: ""
    property string commandPaletteHotkey: ""
    property bool streamPcAudioToPhone: false
    property bool nectarIsland: true
    property bool nectarIslandCalls: true
    property bool nectarIslandMedia: true
    property bool nectarIslandNotifications: true
    property bool nectarIslandTransfers: true
}
