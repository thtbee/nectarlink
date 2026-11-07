// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Phones whose live screen is shown inside Home's phone frame rather than in a
// window of its own. App.qml skips their screen session when it opens mirror
// windows.
QtObject {
    property var devices: ({})

    function isEmbedded(device) {
        return devices[device] === true
    }
    function embed(device) {
        const next = Object.assign({}, devices)
        next[device] = true
        devices = next
    }
    function release(device) {
        if (!isEmbedded(device))
            return
        const next = Object.assign({}, devices)
        delete next[device]
        devices = next
    }
}
