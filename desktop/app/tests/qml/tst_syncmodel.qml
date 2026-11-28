// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A SyncModel changes only the rows that differ, so a list view over it keeps
// its place and its delegates when the list is read again.
Item {
    id: root
    width: 300
    height: 200

    SyncModel { id: model }

    function ids() {
        const out = []
        for (let i = 0; i < model.count; i++)
            out.push(model.idAt(i))
        return out
    }

    function rows(from, to) {
        const out = []
        for (let i = from; i < to; i++)
            out.push({ id: "r" + i, text: "row " + i })
        return out
    }

    ListView {
        id: view
        anchors.fill: parent
        clip: true
        model: model
        delegate: Item {
            id: item
            required property string json
            required property string rid
            property int built: 0
            width: view.width
            height: 40
            Component.onCompleted: root.built++
            Text { text: JSON.parse(item.json).text }
        }
    }
    property int built: 0

    TestCase {
        name: "SyncModel"
        when: windowShown

        function init() {
            model.clear()
            root.built = 0
        }

        function test_rows_are_added_removed_and_moved_in_order() {
            model.sync(rows(0, 5))
            compare(root.ids().join(), "r0,r1,r2,r3,r4")
            model.sync([{ id: "r4", text: "row 4" }, { id: "r0", text: "row 0" }, { id: "new", text: "x" }, { id: "r2", text: "row 2" }])
            compare(root.ids().join(), "r4,r0,new,r2")
            model.sync([])
            compare(model.count, 0)
        }

        function test_a_changed_row_is_updated_in_place() {
            model.sync(rows(0, 3))
            compare(JSON.parse(model.get(1).json).text, "row 1")
            model.sync([{ id: "r0", text: "row 0" }, { id: "r1", text: "changed" }, { id: "r2", text: "row 2" }])
            compare(JSON.parse(model.get(1).json).text, "changed")
            compare(root.ids().join(), "r0,r1,r2")
        }

        function test_a_row_repeated_is_listed_once() {
            model.sync([{ id: "a" }, { id: "a" }, { id: "b" }])
            compare(root.ids().join(), "a,b")
        }

        function test_nothing_changing_changes_nothing() {
            model.sync(rows(0, 4))
            let changes = 0
            const count = () => changes++
            model.dataChanged.connect(count)
            model.rowsInserted.connect(count)
            model.rowsRemoved.connect(count)
            model.rowsMoved.connect(count)
            model.sync(rows(0, 4))
            compare(changes, 0)
        }

        function test_a_view_keeps_its_place_and_delegates_when_rows_change() {
            model.sync(rows(0, 40))
            view.contentY = 400
            wait(50)
            const at = view.contentY
            const first = view.itemAtIndex(10)
            verify(first)
            // The list is read again: one changed, one moved elsewhere, one added at the top.
            const next = rows(0, 40)
            next[12].text = "changed"
            next.splice(2, 0, next.splice(35, 1)[0])
            next.unshift({ id: "fresh", text: "fresh" })
            model.sync(next)
            wait(50)
            compare(view.contentY, at)
            // One added and one moved in above it.
            compare(model.indexOfId("r10"), 12)
        }

        function test_a_view_does_not_build_its_delegates_again() {
            model.sync(rows(0, 6))
            wait(50)
            const before = root.built
            verify(before > 0)
            model.sync(rows(0, 6).map(r => ({ id: r.id, text: r.text })))
            wait(50)
            compare(root.built, before)
            model.sync(rows(0, 6).reverse())
            wait(50)
            compare(root.built, before)
        }
    }
}
