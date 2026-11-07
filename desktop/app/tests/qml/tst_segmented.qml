// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A Segmented follows its value and its options as they change, as the
// webcam's resolution choice does when the phone's camera can't do 4K.
Item {
    id: root
    width: 400
    height: 100

    property bool uhd: true
    property int size: 125

    Segmented {
        id: choice
        options: [
            { value: "720", label: "720p" },
            { value: "1080", label: "1080p" }
        ].concat(root.uhd ? [{ value: "2160", label: "4K" }] : [])
        value: "2160"
    }

    Segmented {
        id: numbers
        y: 50
        options: [
            { value: 100, label: "100%" },
            { value: 125, label: "125%" }
        ]
        value: root.size
    }

    SignalSpy {
        id: picked
        target: choice
        signalName: "picked"
    }

    TestCase {
        name: "Segmented"
        when: windowShown

        function init() {
            root.uhd = true
            choice.value = "2160"
            picked.clear()
        }

        function test_a_number_is_selected_too() {
            compare(numbers.selectedIndex, 1)
            verify(numbers.selectedItem !== null)
            root.size = 100
            compare(numbers.selectedIndex, 0)
        }

        function test_the_value_is_selected() {
            compare(choice.selectedIndex, 2)
            verify(choice.selectedItem !== null)
        }

        function test_options_that_go_take_their_choice_with_them() {
            root.uhd = false
            compare(choice.options.length, 2)
            compare(choice.selectedIndex, -1, "nothing selected while the value isn't offered")
            choice.value = "1080"
            compare(choice.selectedIndex, 1)
            root.uhd = true
            compare(choice.options.length, 3)
            compare(choice.selectedIndex, 1, "the choice stays when an option comes back")
        }

        function test_a_click_picks_its_option() {
            const first = choice.selectedItem.parent.children[0]
            mouseClick(first)
            compare(picked.count, 1)
            compare(picked.signalArguments[0][0], "720")
            compare(choice.value, "2160", "the value is the owner's to change")
        }
    }
}
