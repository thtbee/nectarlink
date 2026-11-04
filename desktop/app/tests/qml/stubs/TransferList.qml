// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust TransferList in QML tests. Like the real one, a test
// changes the rows first and `active` after.
ListModel {
    property int active: 0
    property int waitingCount: 0
    function cancel(id) {}
}
