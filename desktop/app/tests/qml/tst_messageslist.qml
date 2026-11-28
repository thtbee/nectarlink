// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The Messages page keeps its lists steady while the phone's are read again:
// choosing a conversation (which reads it) or a new text doesn't move the
// list, and opening a conversation doesn't redraw its messages as they load.
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

    function threadRows(count, unreadFrom) {
        const out = []
        for (let i = 0; i < count; i++) {
            out.push({
                id: "t" + i, title: "Person " + i, addresses: "+1555" + i, snippet: "hello " + i,
                date: 1000000 - i, unread: i >= unreadFrom ? 2 : 0, group: false, photo: "",
                isChat: false, app: "", appName: "SMS", appIcon: "", canReply: true, lastSender: "", active: true
            })
        }
        return out
    }

    // Oldest first, as the page gets them.
    function messageRows(count, firstId) {
        const out = []
        for (let i = 0; i < count; i++) {
            out.push({
                id: "m" + (firstId + i), body: "text " + (firstId + i), date: 5000000 + (firstId + i) * 60000,
                outgoing: i % 3 === 0, status: "", images: [], audios: [], attachments: 0, sender: null, code: null
            })
        }
        return out
    }

    TestCase {
        name: "MessagesList"
        when: windowShown

        function init() {
            Messages.thread = ""
            Messages.messages = "[]"
            Messages.threads = JSON.stringify(root.threadRows(40, 20))
            wait(50)
        }

        // Reading a conversation takes it off the unread list; the list stays where it was.
        function test_choosing_a_conversation_low_in_the_list_does_not_scroll_the_list() {
            const list = root.find(page, "threadList")
            verify(list)
            compare(list.count, 40)
            list.contentY = 900
            wait(30)
            const y = list.contentY
            const row = list.itemAtIndex(Math.floor(y / 72) + 3)
            verify(row, "a row in view")
            const id = row.thread.id
            // What the phone's reading does to the list: that one is read, and a newer text moves another to the top.
            const next = root.threadRows(40, 20)
            next[id.substring(1) * 1].unread = 0
            next[39].date = 2000000
            Messages.threads = JSON.stringify(next.sort((a, b) => b.date - a.date))
            wait(50)
            compare(list.contentY, y)
            compare(list.count, 40)
            // The row that was read is the same item, only changed.
            const same = list.itemAtIndex(Math.floor(y / 72) + 4)
            verify(same)
        }

        function test_a_selected_conversation_and_the_rows_are_not_built_again() {
            const list = root.find(page, "threadList")
            list.contentY = 500
            wait(30)
            const first = list.itemAtIndex(Math.floor(list.contentY / 72) + 2)
            verify(first)
            Messages.threads = JSON.stringify(root.threadRows(40, 10))
            wait(50)
            compare(list.itemAtIndex(Math.floor(list.contentY / 72) + 2), first)
        }

        function test_a_conversation_opens_at_its_newest_message_and_stays_there() {
            Messages.messages = JSON.stringify(root.messageRows(100, 0))
            Messages.thread = "t3"
            wait(100)
            const list = root.find(page, "messageList")
            verify(list)
            compare(list.count, 100)
            const newest = list.itemAtIndex(0)
            verify(newest, "the newest bubble")
            const bottom = list.contentItem.mapToItem(root, 0, newest.y + newest.height).y
            const view = list.mapToItem(root, 0, list.height).y
            verify(Math.abs(bottom + 8 - view) < 4, "the newest is at the bottom of the view")

            // Pictures arriving, a refresh, and a new text: no bubble is built again.
            const created = []
            for (let i = 0; i < 6; i++)
                created.push(list.itemAtIndex(i))
            const rows = root.messageRows(100, 0)
            rows[99].images = ["file:///nowhere.jpg"]
            rows[98].body = "changed"
            Messages.messages = JSON.stringify(rows)
            wait(50)
            for (let i = 0; i < 6; i++)
                compare(list.itemAtIndex(i + 0), created[i])
            const more = root.messageRows(101, 0)
            Messages.messages = JSON.stringify(more)
            wait(50)
            compare(list.count, 101)
            const latest = list.itemAtIndex(0)
            const latestBottom = list.contentItem.mapToItem(root, 0, latest.y + latest.height).y
            verify(Math.abs(latestBottom + 8 - view) < 4, "a new text shows at the bottom")
        }

        function test_older_messages_loaded_do_not_move_what_is_in_view() {
            Messages.messages = JSON.stringify(root.messageRows(60, 40))
            Messages.thread = "t4"
            wait(100)
            const list = root.find(page, "messageList")
            list.contentY -= 400
            wait(30)
            const probe = list.itemAtIndex(list.indexAt(100, list.contentY + 100))
            verify(probe)
            const before = list.contentItem.mapToItem(root, 0, probe.y).y
            const id = probe.message.id
            Messages.messages = JSON.stringify(root.messageRows(100, 0))
            wait(100)
            const after = list.itemAtIndex(list.indexAt(100, list.contentY + 100))
            verify(after)
            fuzzyCompare(list.contentItem.mapToItem(root, 0, list.itemAtIndex(index_of(list, id)).y).y, before, 1)
        }

        function index_of(list, id) {
            for (let i = 0; i < list.count; i++) {
                const item = list.itemAtIndex(i)
                if (item && item.message.id === id)
                    return i
            }
            return -1
        }

        function test_switching_conversations_opens_the_other_at_its_newest() {
            Messages.messages = JSON.stringify(root.messageRows(100, 0))
            Messages.thread = "t1"
            wait(100)
            const list = root.find(page, "messageList")
            list.contentY -= 1500
            wait(30)
            Messages.messages = JSON.stringify(root.messageRows(80, 1000))
            Messages.thread = "t2"
            wait(100)
            const newest = list.itemAtIndex(0)
            verify(newest)
            compare(newest.message.id, "m1079")
            const bottom = list.contentItem.mapToItem(root, 0, newest.y + newest.height).y
            const view = list.mapToItem(root, 0, list.height).y
            verify(Math.abs(bottom + 8 - view) < 4)
        }

        // A clicked notification's text is brought into view.
        function test_a_message_brought_into_view_by_a_notification_is_shown() {
            Messages.messages = JSON.stringify(root.messageRows(100, 0))
            Messages.thread = "t3"
            wait(100)
            const list = root.find(page, "messageList")
            Messages.focusMessage = "m12"
            wait(100)
            const item = list.itemAtIndex(index_of(list, "m12"))
            verify(item, "its bubble is built")
            const top = list.contentItem.mapToItem(list, 0, item.y).y
            verify(top >= -1 && top + item.height <= list.height + 1, "and inside the view")
            Messages.focusMessage = ""
        }
    }
}
