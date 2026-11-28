// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A chat note sits on the right when this PC wrote it and on the left when
// the device did; only this PC's can be changed, and either can be deleted.
Item {
    id: root
    width: 600
    height: 300

    ChatNote {
        id: note
        width: root.width
        text: "Milk, eggs, bread"
        mine: true
        canEdit: true
        timestamp: 1700000000
    }

    SignalSpy { id: copySpy; target: note; signalName: "copyRequested" }
    SignalSpy { id: editSpy; target: note; signalName: "editRequested" }
    SignalSpy { id: deleteSpy; target: note; signalName: "deleteRequested" }

    TestCase {
        name: "ChatNote"
        when: windowShown

        function init() {
            note.mine = true
            note.canEdit = true
            note.dayLabel = ""
            copySpy.clear()
            editSpy.clear()
            deleteSpy.clear()
        }

        function test_this_pcs_notes_are_on_the_right_and_the_devices_on_the_left() {
            const bubble = findChild(note, "bubble")
            verify(bubble.x + bubble.width > note.width / 2)
            verify(bubble.x > note.width - bubble.width - 40, "against the right edge")
            note.mine = false
            note.canEdit = false
            tryVerify(() => bubble.x < 40, 1000, "against the left edge")
        }

        function test_only_a_note_that_can_be_changed_offers_edit() {
            verify(findChild(note, "editButton").visible)
            note.canEdit = false
            verify(!findChild(note, "editButton").visible)
            verify(findChild(note, "copyButton").visible)
            verify(findChild(note, "deleteButton").visible)
        }

        function test_its_buttons_ask_for_what_they_say() {
            findChild(note, "copyButton").clicked()
            findChild(note, "editButton").clicked()
            findChild(note, "deleteButton").clicked()
            compare(copySpy.count, 1)
            compare(editSpy.count, 1)
            compare(deleteSpy.count, 1)
        }

        function test_the_buttons_show_when_one_of_them_has_focus() {
            const actions = findChild(note, "actions")
            tryCompare(actions, "opacity", 0, 1000)
            findChild(note, "deleteButton").forceActiveFocus()
            tryCompare(actions, "opacity", 1, 1000)
        }

        function test_a_day_label_adds_room_above_the_note() {
            const plain = note.height
            note.dayLabel = "Today"
            verify(note.height > plain)
        }
    }
}
