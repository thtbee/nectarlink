// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// Report a problem: the area starts at the guess, the report is made from
// what was written, parts can be left out, and Open on GitHub copies the
// report first.
Item {
    id: root
    width: 800
    height: 900

    ReportSheet { id: sheet }

    TestCase {
        name: "ReportSheet"
        when: windowShown

        function init() {
            Report.guessed = "files"
            Report.lastPrepare = null
            Report.githubOpened = 0
            Report.sends = 0
            Report.sendState = ""
            Report.sentIssue = 0
            AppController.copied = ""
            sheet.open()
            wait(0)
        }
        function cleanup() { sheet.close() }

        function test_opens_on_the_guessed_area() {
            compare(sheet.step, 0)
            compare(sheet.area, "files")
            compare(sheet.parts, 31)
        }

        function test_next_makes_the_report_from_what_was_written() {
            sheet.area = "audio"
            sheet.makeReport()
            compare(Report.lastPrepare.area, "audio")
            compare(Report.lastPrepare.parts, 31)
            verify(Report.lastPrepare.justNow)
        }

        function test_leaving_a_part_out_makes_it_again() {
            sheet.step = 1
            sheet.makeReport()
            verify(Report.preview.indexOf("log") >= 0)
            sheet.togglePart(16)
            compare(Report.lastPrepare.parts, 15)
            verify(Report.preview.indexOf("log") < 0)
            sheet.togglePart(16)
            compare(Report.lastPrepare.parts, 31)
        }

        function test_send_files_it_once() {
            sheet.step = 1
            sheet.makeReport()
            const send = findChild(sheet, "reportSend")
            verify(send.enabled)
            send.clicked()
            compare(Report.sends, 1)
            compare(Report.sentIssue, 412)
            verify(!send.enabled)
        }

        function test_reopening_starts_over() {
            sheet.step = 1
            sheet.area = "calls"
            sheet.close()
            sheet.open()
            compare(sheet.step, 0)
            compare(sheet.area, "files")
        }
    }
}
