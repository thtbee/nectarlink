// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A file in the chat sits on the side of whoever sent it, shows its name and
// size, opens on a click (or shows its folder when it can't open), and offers
// Open, Show in folder and Remove from chat.
Item {
    id: root
    width: 600
    height: 300

    ChatAttachment {
        id: attachment
        width: root.width
        title: "report.pdf"
        size: "2 KB"
        mine: true
        canOpen: true
        canShowInFolder: true
        timestamp: 1700000000
    }

    SignalSpy { id: openSpy; target: attachment; signalName: "openRequested" }
    SignalSpy { id: folderSpy; target: attachment; signalName: "showInFolderRequested" }
    SignalSpy { id: removeSpy; target: attachment; signalName: "removeRequested" }

    TestCase {
        name: "ChatAttachment"
        when: windowShown

        function init() {
            attachment.mine = true
            attachment.kind = "file"
            attachment.title = "report.pdf"
            attachment.size = "2 KB"
            attachment.thumbUrl = ""
            attachment.canOpen = true
            attachment.canShowInFolder = true
            attachment.dayLabel = ""
            openSpy.clear()
            folderSpy.clear()
            removeSpy.clear()
        }

        function test_sent_files_are_on_the_right_and_received_ones_on_the_left() {
            const bubble = findChild(attachment, "bubble")
            verify(bubble.x > attachment.width - bubble.width - 40, "against the right edge")
            attachment.mine = false
            tryVerify(() => bubble.x < 40, 1000, "against the left edge")
        }

        function test_it_shows_its_name_and_size() {
            compare(findChild(attachment, "name").text, "report.pdf")
            compare(findChild(attachment, "size").text, "2 KB")
        }

        function test_without_an_image_it_shows_a_file_icon() {
            verify(findChild(attachment, "fileIcon").visible)
            verify(!findChild(attachment, "thumb").visible)
        }

        function test_an_image_on_this_pc_shows_as_a_thumbnail() {
            attachment.kind = "photo"
            attachment.thumbUrl = "data:image/png;base64,"
                + "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
            tryVerify(() => findChild(attachment, "thumb").visible, 2000)
            verify(!findChild(attachment, "fileIcon").visible)
        }

        function test_a_click_opens_it_or_shows_its_folder() {
            const bubble = findChild(attachment, "bubble")
            mouseClick(bubble)
            compare(openSpy.count, 1)
            compare(folderSpy.count, 0)
            attachment.canOpen = false
            mouseClick(bubble)
            compare(openSpy.count, 1)
            compare(folderSpy.count, 1)
        }

        function test_a_file_not_on_this_pc_says_so_and_offers_only_remove() {
            attachment.canOpen = false
            attachment.canShowInFolder = false
            verify(findChild(attachment, "size").text.indexOf("2 KB") === 0)
            verify(findChild(attachment, "size").text.length > "2 KB".length)
            verify(!findChild(attachment, "openButton").visible)
            verify(!findChild(attachment, "folderButton").visible)
            verify(findChild(attachment, "removeButton").visible)
            mouseClick(findChild(attachment, "bubble"))
            compare(openSpy.count + folderSpy.count, 0)
        }

        function test_its_buttons_ask_for_what_they_say() {
            findChild(attachment, "openButton").clicked()
            findChild(attachment, "folderButton").clicked()
            findChild(attachment, "removeButton").clicked()
            compare(openSpy.count, 1)
            compare(folderSpy.count, 1)
            compare(removeSpy.count, 1)
        }

        function test_the_buttons_show_when_it_has_focus() {
            const actions = findChild(attachment, "actions")
            tryCompare(actions, "opacity", 0, 1000)
            findChild(attachment, "bubble").forceActiveFocus()
            tryCompare(actions, "opacity", 1, 1000)
        }

        function test_enter_opens_it() {
            const bubble = findChild(attachment, "bubble")
            bubble.forceActiveFocus()
            keyClick(Qt.Key_Return)
            compare(openSpy.count, 1)
        }

        function test_a_day_label_adds_room_above_it() {
            const plain = attachment.height
            attachment.dayLabel = "Today"
            verify(attachment.height > plain)
        }
    }
}
