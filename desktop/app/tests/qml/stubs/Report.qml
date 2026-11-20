// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust Report in QML tests: makes a report from what it's
// given at once, and remembers what was asked.
QtObject {
    property string preview: ""
    property bool busy: false
    property string guessed: "files"
    property var lastPrepare: null
    property int githubOpened: 0
    property string sendState: ""
    property int sentIssue: 0
    property int sends: 0
    property bool phoneLogWaiting: false
    property string crash: ""
    property string crashState: ""
    property string crashFixedIn: ""
    property int crashSends: 0
    function checkCrash() {}
    function crashSend() { crashSends++; crashState = "sent" }
    function crashDone(look) { crash = ""; crashState = ""; if (look) requested() }
    function crashOff() { crash = ""; crashState = "" }
    signal requested()
    function open() { requested() }
    function guessArea() { return guessed }
    function prepare(area, other, text, justNow, parts) {
        lastPrepare = { area: area, other: other, text: text, justNow: justNow, parts: parts }
        preview = "Area: " + area + "\n" + text + ((parts & 16) ? "\nlog" : "")
    }
    function looksLikeContact(text) { return text.indexOf("@") >= 0 }
    function openOnGithub() { githubOpened++; return true }
    function send() { sends++; sentIssue = 412; sendState = "filed" }
    function openSentIssue() { return sentIssue > 0 }
}
