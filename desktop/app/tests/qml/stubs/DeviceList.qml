// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust DeviceList in QML tests: no phones.
QtObject {
    property int count: 0
    function deviceNameAt(index) { return "" }
}
