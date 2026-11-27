// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// Selecting, marking unread and deleting conversations and messages on the Messages
// page: Ctrl+click toggles, Shift+click selects a range, a toolbar acts on the
// selection, Esc cancels, right-click opens a menu, and deleting asks first.
Item {
    id: root
    width: 900
    height: 600

    MessagesPage {
        id: page
        anchors.fill: parent
        deviceId: "phone"
        deviceName: "Pixel"
    }

    function find(item, name) {
        if (item.objectName === name)
            return item
        const kids = item.children
        for (let i = 0; i < kids.length; i++) {
            const found = find(kids[i], name)
            if (found)
                return found
        }
        return null
    }

    function threadRows(count) {
        const out = []
        for (let i = 0; i < count; i++) {
            out.push({
                id: "t" + i, title: "Person " + i, addresses: "+1555" + i, snippet: "hello " + i,
                date: 1000000 - i, unread: 0, group: false, photo: "", isChat: false, app: "",
                appName: "SMS", appIcon: "", canReply: true, lastSender: "", active: true
            })
        }
        return out
    }

    // Oldest first, as the page gets them.
    function messageRows(count) {
        const out = []
        for (let i = 0; i < count; i++) {
            out.push({
                id: "m" + i, body: "text " + i, date: 5000000 + i * 60000, outgoing: i % 2 === 0,
                status: "", images: [], audios: [], attachments: 0, sender: null, code: null
            })
        }
        return out
    }

    TestCase {
        name: "MessagesManage"
        when: windowShown

        function row(index) {
            return root.find(page, "threadList").itemAtIndex(index)
        }
        function bubbleOf(id) {
            const list = root.find(page, "messageList")
            for (let i = 0; i < list.count; i++) {
                const item = list.itemAtIndex(i)
                if (item && item.message.id === id)
                    return item
            }
            return null
        }
        function click(item, button, modifiers) {
            mouseClick(item, item.width / 2, item.height / 2, button || Qt.LeftButton, modifiers || Qt.NoModifier)
        }

        function init() {
            Messages.asked = []
            Messages.thread = ""
            Messages.messages = "[]"
            Messages.currentIsChat = false
            Messages.threads = JSON.stringify(root.threadRows(10))
            wait(50)
            page.closeChat()
            keyClick(Qt.Key_Escape)
            keyClick(Qt.Key_Escape)
            // A sheet closed by the last test fades out first, and holds the clicks until it has.
            tryVerify(() => !root.find(page, "deleteSheet").visible && !root.find(page, "imageSheet").visible)
        }

        function test_a_plain_click_opens_and_ctrl_click_selects() {
            click(row(2))
            compare(Messages.lastAsk("openThread"), "t2")
            Messages.closeThread()
            compare(page.selectedThreads.length, 0)
            click(row(3), Qt.LeftButton, Qt.ControlModifier)
            compare(page.selectedThreads.join(), "t3")
            compare(Messages.lastAsk("openThread"), "t2", "selecting doesn't open")
            // Now any click toggles.
            click(row(5))
            compare(page.selectedThreads.join(), "t3,t5")
            click(row(3), Qt.LeftButton, Qt.ControlModifier)
            compare(page.selectedThreads.join(), "t5")
        }

        function test_shift_click_selects_a_range() {
            click(row(2), Qt.LeftButton, Qt.ControlModifier)
            click(row(6), Qt.LeftButton, Qt.ShiftModifier)
            compare(page.selectedThreads.join(), "t2,t3,t4,t5,t6")
        }

        function test_escape_cancels_the_selection_and_the_button_starts_one() {
            click(root.find(page, "selectThreads"))
            click(row(1))
            click(row(4))
            compare(page.selectedThreads.join(), "t1,t4")
            keyClick(Qt.Key_Escape)
            compare(page.selectedThreads.length, 0)
            // Not selecting any more: a click opens.
            click(row(1))
            compare(Messages.lastAsk("openThread"), "t1")
        }

        function test_the_toolbar_marks_unread_and_deletes_the_selection() {
            click(row(1), Qt.LeftButton, Qt.ControlModifier)
            click(row(2), Qt.LeftButton, Qt.ShiftModifier)
            click(root.find(page, "unreadSelected"))
            compare(Messages.lastAsk("markThreadsUnread"), '["t1","t2"]')
            compare(page.selectedThreads.length, 0, "the selection is done with")

            click(row(5), Qt.LeftButton, Qt.ControlModifier)
            click(row(6), Qt.LeftButton, Qt.ControlModifier)
            click(root.find(page, "deleteSelected"))
            const sheet = root.find(page, "deleteSheet")
            verify(sheet.opened, "asks first")
            compare(Messages.lastAsk("deleteThreads"), undefined)
            click(root.find(page, "confirmDelete"))
            compare(Messages.lastAsk("deleteThreads"), '["t5","t6"]')
            verify(!sheet.opened)
            compare(page.selectedThreads.length, 0)
        }

        // On this PC it is the conversation that is marked unread, however many messages are chosen.
        function test_selected_messages_are_marked_unread_by_count() {
            Messages.messages = JSON.stringify(root.messageRows(8))
            Messages.thread = "t1"
            wait(100)
            click(bubbleOf("m2"), Qt.LeftButton, Qt.ControlModifier)
            click(bubbleOf("m4"), Qt.LeftButton, Qt.ControlModifier)
            const labels = page.messageEntries(page.selectedMessages, true).map(e => e.label)
            verify(labels.some(l => l.indexOf("unread") >= 0 && l.indexOf("conversation") < 0), labels.join("|"))
            click(root.find(page, "unreadSelectedMessages"))
            verify(Messages.lastAsk("markOpenUnread") !== undefined)
            compare(page.selectedMessages.length, 0)
        }

        function test_right_click_opens_a_menu_for_that_conversation() {
            click(row(2), Qt.RightButton)
            const menu = root.find(page, "contextMenu")
            verify(menu.opened)
            compare(menu.entries.map(e => e.key).join(), "select,unread,delete")
            page.menuChosen("unread")
            compare(Messages.lastAsk("markThreadsUnread"), '["t2"]')
            menu.close()
            // On one that is part of a selection it's for the whole selection.
            click(row(1), Qt.LeftButton, Qt.ControlModifier)
            click(row(3), Qt.LeftButton, Qt.ControlModifier)
            click(row(3), Qt.RightButton)
            verify(menu.opened)
            compare(menu.entries.map(e => e.key).join(), "unread,delete", "already selecting")
            page.menuChosen("delete")
            compare(page.pendingDelete.ids.join(), "t1,t3")
            root.find(page, "deleteSheet").close()
            menu.close()
        }

        function test_the_open_conversation_has_its_own_menu() {
            Messages.messages = JSON.stringify(root.messageRows(5))
            Messages.thread = "t1"
            wait(80)
            click(root.find(page, "moreButton"))
            const menu = root.find(page, "contextMenu")
            verify(menu.opened)
            compare(menu.entries.map(e => e.key).join(), "select,unread,delete")
            page.menuChosen("unread")
            compare(Messages.lastAsk("markThreadsUnread"), '["t1"]')
            menu.close()
        }

        function test_messages_are_selected_copied_and_deleted() {
            Messages.messages = JSON.stringify(root.messageRows(8))
            Messages.thread = "t1"
            wait(100)
            click(bubbleOf("m2"), Qt.LeftButton, Qt.ControlModifier)
            compare(page.selectedMessages.join(), "m2")
            click(bubbleOf("m5"), Qt.LeftButton, Qt.ShiftModifier)
            compare(page.selectedMessages.join(), "m2,m3,m4,m5")
            // While selecting, a plain click toggles.
            click(bubbleOf("m3"))
            compare(page.selectedMessages.join(), "m2,m4,m5")

            click(root.find(page, "copySelectedMessages"))
            compare(Messages.lastAsk("copy"), "text 2\ntext 4\ntext 5")
            compare(page.selectedMessages.length, 0)

            click(bubbleOf("m6"), Qt.LeftButton, Qt.ControlModifier)
            keyClick(Qt.Key_Escape)
            compare(page.selectedMessages.length, 0)
            compare(Messages.thread, "t1", "Escape cancels the selection first")

            click(bubbleOf("m1"), Qt.LeftButton, Qt.ControlModifier)
            click(bubbleOf("m7"), Qt.LeftButton, Qt.ControlModifier)
            click(root.find(page, "deleteSelectedMessages"))
            verify(root.find(page, "deleteSheet").opened)
            click(root.find(page, "confirmDelete"))
            compare(Messages.lastAsk("deleteMessages"), '["m1","m7"]')
        }

        function test_right_click_on_a_message_opens_its_menu() {
            Messages.messages = JSON.stringify(root.messageRows(4))
            Messages.thread = "t1"
            wait(100)
            click(bubbleOf("m1"), Qt.RightButton)
            const menu = root.find(page, "contextMenu")
            verify(menu.opened)
            compare(menu.entries.map(e => e.key).join(), "select,copy,unread,delete")
            page.menuChosen("copy")
            compare(Messages.lastAsk("copy"), "text 1")
            menu.close()
        }

        function test_a_conversation_marked_unread_is_left_for_the_list() {
            Messages.messages = JSON.stringify(root.messageRows(4))
            Messages.thread = "t1"
            wait(100)
            click(bubbleOf("m1"), Qt.RightButton)
            page.menuChosen("unread")
            verify(Messages.lastAsk("markOpenUnread") !== undefined)
            root.find(page, "contextMenu").close()
        }

        // The entry of an open menu, by its key, to click it as a user does.
        function menuEntry(key) {
            const menu = root.find(page, "contextMenu")
            const walk = (item) => {
                if (item.modelData && item.modelData.key === key && item.width > 0)
                    return item
                for (let i = 0; i < item.children.length; i++) {
                    const found = walk(item.children[i])
                    if (found)
                        return found
                }
                return null
            }
            return walk(menu)
        }

        // Oldest first: a picture message in the middle.
        function pictureMessages() {
            const rows = root.messageRows(3)
            rows[1].images = ["file:///cache/sms-part-9.jpg"]
            rows[1].body = ""
            return rows
        }

        // Right-click on the picture of a message, then a click on a menu entry: the entry's
        // action and nothing else, in particular not the picture opening.
        function chooseOnPicture(key, rows) {
            Messages.messages = JSON.stringify(rows || pictureMessages())
            Messages.thread = "t1"
            wait(100)
            const sheet = root.find(page, "imageSheet")
            const picture = bubbleOf("m1")
            verify(picture !== null)
            // On the picture: the first message of the day has no separator, so it is at the top.
            mouseClick(picture, 90, 90, Qt.RightButton)
            const menu = root.find(page, "contextMenu")
            verify(menu.opened, "the menu opens on a right-click")
            verify(!sheet.opened, "a right-click doesn't open the picture")
            // Laid out: the entries are stacked by the next frame.
            wait(50)
            const entry = menuEntry(key)
            verify(entry !== null, key)
            click(entry)
            tryVerify(() => !menu.opened, 1000, key + " closes the menu")
            wait(50)
            verify(!sheet.opened, key + " must not open the picture")
        }

        function test_select_on_a_picture_message_only_selects() {
            chooseOnPicture("select")
            compare(page.selectedMessages.join(), "m1")
            click(root.find(page, "cancelMessageSelection"))
            compare(page.selectedMessages.length, 0)
        }

        function test_mark_unread_on_a_picture_message_only_marks_unread() {
            chooseOnPicture("unread")
            verify(Messages.lastAsk("markOpenUnread") !== undefined)
        }

        function test_delete_on_a_picture_message_only_asks_to_delete() {
            chooseOnPicture("delete")
            verify(root.find(page, "deleteSheet").opened)
            compare(page.pendingDelete.ids.join(), "m1")
            root.find(page, "deleteSheet").close()
        }

        function test_copy_on_a_picture_message_with_text_only_copies() {
            const rows = pictureMessages()
            rows[1].body = "look"
            chooseOnPicture("copy", rows)
            compare(Messages.lastAsk("copy"), "look")
        }

        function test_a_click_outside_the_menu_only_closes_it() {
            Messages.messages = JSON.stringify(pictureMessages())
            Messages.thread = "t1"
            wait(100)
            const picture = bubbleOf("m1")
            mouseClick(picture, 90, 90, Qt.RightButton)
            const menu = root.find(page, "contextMenu")
            verify(menu.opened)
            wait(50)
            // On the picture again, away from the menu.
            const entry = menuEntry("select")
            const away = picture.mapFromItem(entry, 0, 0).x > 150 ? 40 : 240
            mouseClick(picture, away, 60)
            tryVerify(() => !menu.opened)
            wait(50)
            verify(!root.find(page, "imageSheet").opened, "the click that closes the menu opens nothing")
        }

        // The photo of a conversation in the list: a menu entry over it doesn't open it.
        function test_a_menu_entry_over_a_conversations_photo_does_not_open_it() {
            const rows = root.threadRows(10)
            rows[2].photo = "file:///cache/photo.jpg"
            Messages.threads = JSON.stringify(rows)
            wait(50)
            const r = row(2)
            mouseClick(r, 28, r.height / 2, Qt.RightButton)
            const menu = root.find(page, "contextMenu")
            verify(menu.opened)
            wait(50)
            click(menuEntry("select"))
            tryVerify(() => !menu.opened)
            wait(50)
            compare(Messages.lastAsk("openThread"), undefined, "choosing doesn't open the conversation")
            compare(page.selectedThreads.join(), "t2")
        }

        function test_a_picture_can_be_copied_saved_and_shared() {
            const sheet = root.find(page, "imageSheet")
            sheet.openFor("file:///cache/sms-part-1.jpg")
            verify(sheet.opened)
            click(root.find(page, "copyImage"))
            compare(Messages.lastAsk("copyImage"), "file:///cache/sms-part-1.jpg")
            verify(root.find(page, "saveImage").visible)
            // Where there's a share sheet: Windows and macOS only.
            const share = root.find(page, "shareImage")
            compare(share.visible, Qt.platform.os === "windows" || Qt.platform.os === "osx")
            if (share.visible) {
                click(share)
                compare(Messages.lastAsk("shareImage"), "file:///cache/sms-part-1.jpg")
            }
            sheet.close()
        }
    }
}
