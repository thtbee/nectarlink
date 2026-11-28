// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust DeckController in QML tests: this PC's volume.
QtObject {
    property int volume: 50
    property bool muted: false
}
