// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// How the Messages page lays out a conversation: newest at the bottom (a
// list view that runs from the bottom up, with the newest message first), so
// it opens at the newest message, stays there when one arrives, and keeps the
// reader's place when older ones are added above or a bubble changes height.
Item {
    id: root
    width: 300
    height: 200

    SyncModel { id: model }

    // Newest first: "m<n>" for n from `newest` down to `oldest`.
    function rows(newest, oldest) {
        const out = []
        for (let i = newest; i >= oldest; i--)
            out.push({ id: "m" + i, tall: 0 })
        return out
    }

    ScrollRail { flick: view }
    WheelScroll { id: wheel; flick: view }
    ListView {
        id: view
        anchors.fill: parent
        clip: true
        verticalLayoutDirection: ListView.BottomToTop
        spacing: 6
        boundsBehavior: Flickable.StopAtBounds
        model: model
        delegate: Rectangle {
            id: item
            required property string json
            required property string rid
            readonly property var row: JSON.parse(json)
            width: view.width
            height: 40 + row.tall
            color: "#ccc"
        }
    }

    function bottomOf(index) {
        const item = view.itemAtIndex(index)
        return item ? view.contentItem.mapToItem(root, 0, item.y + item.height).y : NaN
    }
    function topOf(index) {
        const item = view.itemAtIndex(index)
        return item ? view.contentItem.mapToItem(root, 0, item.y).y : NaN
    }

    TestCase {
        name: "ChatList"
        when: windowShown

        function init() {
            model.clear()
            model.sync(root.rows(59, 20))
            view.positionViewAtBeginning()
            wait(30)
        }

        function test_it_opens_at_the_newest() {
            fuzzyCompare(root.bottomOf(0), root.height, 1)
            verify(view.atYEnd, "at the newest end")
        }

        function test_a_short_conversation_sits_at_the_bottom() {
            model.clear()
            model.sync(root.rows(1, 0))
            view.positionViewAtBeginning()
            wait(30)
            fuzzyCompare(root.bottomOf(0), root.height, 1)
            verify(root.topOf(1) > 40, "the older one above the newer")
        }

        function test_a_new_message_shows_when_at_the_newest() {
            model.sync(root.rows(60, 20))
            wait(30)
            fuzzyCompare(root.bottomOf(0), root.height, 1)
            compare(model.idAt(0), "m60")
        }

        function test_older_messages_added_above_keep_the_reader_in_place() {
            view.contentY -= 300
            wait(30)
            const before = root.topOf(model.indexOfId("m50"))
            const y = view.contentY
            model.sync(root.rows(59, 0))
            wait(30)
            fuzzyCompare(root.topOf(model.indexOfId("m50")), before, 1)
            compare(view.contentY, y)
        }

        function test_a_new_message_keeps_the_reader_in_place_when_scrolled_up() {
            view.contentY -= 300
            wait(30)
            const before = root.topOf(model.indexOfId("m50"))
            model.sync(root.rows(60, 20))
            wait(30)
            fuzzyCompare(root.topOf(model.indexOfId("m50")), before, 1)
        }

        function test_the_wheel_and_the_rail_see_the_whole_list() {
            wheel.clamp(-1e6)
            verify(wheel.clamp(-1e6) <= view.contentY, "can scroll up")
            verify(wheel.clamp(1e6) >= view.contentY, "can scroll down")
            verify(view.visibleArea.heightRatio < 1)
            fuzzyCompare(view.visibleArea.yPosition, 1 - view.visibleArea.heightRatio, 0.01, "at the newest end of the list")
            view.contentY = wheel.clamp(-1e6)
            wait(30)
            verify(view.visibleArea.yPosition < 0.05, "at the oldest end of the list")
        }

        function test_a_bubble_growing_at_the_newest_end_stays_at_the_bottom() {
            const next = root.rows(59, 20)
            next[0].tall = 100
            model.sync(next)
            wait(30)
            fuzzyCompare(root.bottomOf(0), root.height, 1)
        }
    }
}
