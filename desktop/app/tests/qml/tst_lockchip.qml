// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A locked feature that the PC can unlock takes you to the right page of Settings.
Item {
    id: root
    width: 480
    height: 120

    LockChip {
        id: chip
        maxWidth: 400
    }

    TestCase {
        name: "LockChip"
        when: windowShown

        function test_opens_the_settings_page_that_unlocks_it_data() {
            return [
                { tag: "add-on", action: "enableAddon", target: "settings:camera_audio" },
                { tag: "phone's switch", action: "enableToggle", target: "settings:devices" },
                { tag: "connection", action: "enablePath", target: "settings:connection" }
            ]
        }

        function test_opens_the_settings_page_that_unlocks_it(data) {
            AppController.highlightTarget = ""
            chip.feature = { state: "locked", action: data.action, target: "x" }
            waitForRendering(chip)
            mouseClick(chip)
            compare(AppController.highlightTarget, data.target)
        }
    }
}
