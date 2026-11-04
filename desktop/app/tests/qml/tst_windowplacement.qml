// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A window comes back on the monitor it was on, wherever that monitor sits.
Item {
    WindowPlacement {
        id: placement
    }

    TestCase {
        name: "WindowPlacement"

        readonly property var mainScreen: ({ virtualX: 0, virtualY: 0, width: 1920, height: 1080 })
        readonly property var leftScreen: ({ virtualX: -1920, virtualY: 0, width: 1920, height: 1080 })
        readonly property var aboveScreen: ({ virtualX: 0, virtualY: -1080, width: 1920, height: 1080 })
        readonly property var rightScreen: ({ virtualX: 1920, virtualY: 0, width: 2560, height: 1440 })

        function test_stays_on_its_monitor_data() {
            return [
                { tag: "main", x: 100, y: 100 },
                { tag: "left of main", x: -1500, y: 200 },
                { tag: "above main", x: 300, y: -900 },
                { tag: "right of main", x: 2500, y: 300 }
            ]
        }

        function test_stays_on_its_monitor(data) {
            const spot = placement.spot(data.x, data.y, 400, 800, [mainScreen, leftScreen, aboveScreen, rightScreen], mainScreen)
            compare(spot.x, data.x)
            compare(spot.y, data.y)
        }

        function test_comes_back_when_its_monitor_is_gone() {
            const spot = placement.spot(-1500, 200, 400, 800, [mainScreen], mainScreen)
            compare(spot.x, 0)
            compare(spot.y, 200)
        }

        function test_a_title_bar_above_every_monitor_comes_back() {
            const spot = placement.spot(100, -40, 400, 800, [mainScreen], mainScreen)
            compare(spot.y, 0)
        }

        function test_real_screens_have_what_it_reads() {
            const screens = Qt.application.screens
            verify(screens.length > 0)
            compare(typeof screens[0].virtualX, "number")
            compare(typeof screens[0].virtualY, "number")
            compare(typeof screens[0].width, "number")
            compare(typeof screens[0].height, "number")
        }
    }
}
