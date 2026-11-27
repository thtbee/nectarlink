// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust Messages in QML tests: what a test sets is what the
// Messages page shows, and what the page asks is kept in `asked`.
QtObject {
    property string device: ""
    property string status: "ready"
    property string threads: "[]"
    property string thread: ""
    property string messages: "[]"
    property bool more: false
    property bool loadingOlder: false
    property bool sending: false
    property string composeTo: ""
    property string composeName: ""
    property string appFilter: ""
    property string availableApps: "[]"
    property bool hasChatThreads: false
    property bool currentIsChat: false
    property string currentAppName: "SMS"
    property bool currentCanReply: true
    property bool currentIsGroup: false
    property bool hasAttachment: false
    property string attachmentPreview: ""
    property string attachmentName: ""
    property string attachmentSize: ""
    property string attachmentError: ""
    property string sims: "[]"
    property string sim: ""
    property string focusMessage: ""

    // What the page asked, as [name, argument] pairs.
    property var asked: []
    function ask(name, arg) { asked = asked.concat([[name, arg === undefined ? "" : arg]]) }
    function lastAsk(name) {
        for (let i = asked.length - 1; i >= 0; i--) {
            if (asked[i][0] === name)
                return asked[i][1]
        }
        return undefined
    }

    function open(device) {}
    function openThread(id) { ask("openThread", id); thread = id }
    function closeThread() { ask("closeThread"); thread = "" }
    function setOnScreen(on) {}
    function loadOlder() { ask("loadOlder") }
    function send(body) { ask("send", body) }
    function sendTo(to, body) { ask("sendTo", to) }
    function refresh() { ask("refresh") }
    function copy(text) { ask("copy", text) }
    function filterByApp(app) { appFilter = app }
    function markThreadsUnread(ids) { ask("markThreadsUnread", ids) }
    function deleteThreads(ids) { ask("deleteThreads", ids) }
    function deleteMessages(ids) { ask("deleteMessages", ids) }
    function markOpenUnread() { ask("markOpenUnread") }
    function attachFile(path) { return false }
    function pasteClipboardImage() { return false }
    function clearAttachment() {}
    function saveDraft(key, text) {}
    function draft(key) { return "" }
    function chooseSim(sim) {}
    function startChat(device, number, name) {}
}
