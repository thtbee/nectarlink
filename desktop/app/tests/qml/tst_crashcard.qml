// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The crash card: shows for a crash, sends it, opens Report a problem to
// look first, and goes away for good with "Don't ask about crashes".
Item {
    id: root
    width: 800
    height: 600

    CrashCard { id: card; eligible: true }

    TestCase {
        name: "CrashCard"
        when: windowShown

        function init() {
            Report.crash = "b3af6c6e31db"
            Report.crashState = ""
            Report.crashSends = 0
        }

        function test_shows_for_a_crash_only() {
            verify(card.visible)
            Report.crash = ""
            verify(!card.visible)
        }

        function test_send() {
            Report.crashSend()
            compare(Report.crashSends, 1)
            verify(card.sent)
        }

        function test_look_first_opens_report_a_problem() {
            let opened = 0
            const listener = () => opened++
            Report.requested.connect(listener)
            Report.crashDone(true)
            Report.requested.disconnect(listener)
            compare(opened, 1)
            verify(!card.visible)
        }

        function test_off() {
            Report.crashOff()
            verify(!card.visible)
        }
    }
}
