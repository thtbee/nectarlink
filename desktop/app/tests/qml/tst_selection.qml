// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// Selecting conversations or messages: Ctrl+click toggles, Shift+click
// selects a range, and a plain click selects once something is selected.
Item {
    width: 100
    height: 100

    Selection { id: sel }
    readonly property var order: ["a", "b", "c", "d", "e"]

    TestCase {
        name: "Selection"

        function init() { sel.clear() }

        function test_a_plain_click_opens_until_something_is_selected() {
            compare(sel.click("a", order, false, false), false)
            compare(sel.count, 0)
            compare(sel.active, false)
        }

        function test_ctrl_click_starts_selecting_and_toggles() {
            compare(sel.click("b", order, true, false), true)
            compare(sel.active, true)
            compare(sel.has("b"), true)
            // Now a plain click toggles too.
            compare(sel.click("d", order, false, false), true)
            compare(sel.inOrder(order).join(), "b,d")
            compare(sel.click("b", order, true, false), true)
            compare(sel.inOrder(order).join(), "d")
            // The last one let go of: the mode ends, as the Select button wasn't used.
            sel.click("d", order, false, false)
            compare(sel.count, 0)
            compare(sel.active, false)
        }

        function test_shift_click_selects_from_the_last_one_clicked() {
            sel.click("b", order, true, false)
            sel.click("e", order, false, true)
            compare(sel.inOrder(order).join(), "b,c,d,e")
            // Either direction.
            sel.clear()
            sel.click("d", order, true, false)
            sel.click("a", order, false, true)
            compare(sel.inOrder(order).join(), "a,b,c,d")
        }

        function test_shift_click_with_none_clicked_yet_selects_that_one() {
            compare(sel.click("c", order, false, true), true)
            compare(sel.inOrder(order).join(), "c")
            compare(sel.active, true)
        }

        function test_the_select_button_stays_on_with_nothing_selected() {
            sel.start()
            compare(sel.active, true)
            compare(sel.count, 0)
            sel.click("a", order, false, false)
            sel.click("a", order, false, false)
            compare(sel.count, 0)
            compare(sel.active, true, "until cancelled")
            sel.clear()
            compare(sel.active, false)
        }

        function test_select_all_and_cancel() {
            sel.selectAll(order)
            compare(sel.count, 5)
            compare(sel.active, true)
            sel.clear()
            compare(sel.count, 0)
            compare(sel.anchorId, "")
        }

        function test_what_leaves_the_list_leaves_the_selection() {
            sel.selectAll(order)
            sel.prune(["a", "c", "x"])
            compare(sel.inOrder(order).join(), "a,c")
            sel.prune(["x"])
            compare(sel.count, 0)
            compare(sel.active, false)
        }

        function test_the_selection_is_replaced_at_each_change_so_bindings_follow() {
            let seen = 0
            const spy = () => seen++
            sel.pickedChanged.connect(spy)
            sel.toggle("a")
            sel.toggle("b")
            compare(seen, 2)
            sel.pickedChanged.disconnect(spy)
        }
    }
}
