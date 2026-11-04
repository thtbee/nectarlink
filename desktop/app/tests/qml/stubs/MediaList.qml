// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust MediaList in QML tests: nothing playing.
ListModel {
    property string activeArt: ""
    property string activeColors: ""
    function command(id, action) {}
}
