// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust AppController in QML tests: still, light, no
// phone or wallpaper colors.
QtObject {
    property list<string> ringingDevices
    property bool systemDark: false
    property bool reduceMotion: true
    property bool powerSaver: false
    property string phoneColors: ""
    property string wallpaperColors: ""
    property string currentPage: "home"
    property string highlightTarget: ""
    property bool doctorBusy: false
    property var doctorChecks: []
    property var commandPaletteResults: []
    property double commandPaletteOpenMs: 0
    signal activateRequested()
    function workArea() { return null }
    function triggerHighlight(target) { highlightTarget = target }
    function styleShelfWindow(window) {}
    function featureState(feature) { return "" }
    function closeCommandPalette() {}
    function noteCommandPaletteShown() {}
    function runCommandPalette(id) {}
    function searchCommandPalette(query) {}
    function runDoctor() {}
    function doctorFix(id) {}
}
