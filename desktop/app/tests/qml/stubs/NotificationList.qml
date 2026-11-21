// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust NotificationList in QML tests: nothing to show.
QtObject {
    property int count: 0
    property string latestApp: ""
    property string latestTitle: ""
    property string latestIcon: ""
}
