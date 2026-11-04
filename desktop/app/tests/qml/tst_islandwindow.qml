// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The island says how transfers really ended: "complete" only for one that
// finished, "failed" for one that didn't, and nothing for a cancelled one.
Item {
    id: root
    width: 480
    height: 120

    IslandWindow {
        id: island
    }

    TestCase {
        name: "IslandWindow"
        when: windowShown

        function unfinished() {
            let n = 0
            for (let i = 0; i < TransferList.count; i++) {
                const s = TransferList.get(i).status
                if (s !== "done" && s !== "failed" && s !== "cancelled")
                    n++
            }
            return n
        }

        function start(id, title) {
            TransferList.append({
                transferId: id, title: title, progress: 0.5, rate: 1000, incoming: false, status: "running"
            })
            TransferList.active = unfinished()
            return TransferList.count - 1
        }

        function setStatus(row, status) {
            TransferList.setProperty(row, "status", status)
            TransferList.active = unfinished()
        }

        function init() {
            // Going active and back clears a banner and its timer from the last test.
            TransferList.active = 1
            TransferList.clear()
            island.transferJustFinished = false
            island.transferFailed = false
            island.finishedTransferTitle = ""
            TransferList.active = 0
        }

        function test_a_finished_transfer_says_complete() {
            setStatus(start("a", "holiday.mp4"), "done")
            compare(island.mode, "transfer")
            compare(island.primaryLabel, "Transfer complete")
            compare(island.secondaryLabel, "holiday.mp4")
        }

        function test_a_failed_transfer_says_failed() {
            setStatus(start("a", "holiday.mp4"), "failed")
            compare(island.mode, "transfer")
            compare(island.primaryLabel, "Transfer failed")
            compare(island.secondaryLabel, "holiday.mp4")
        }

        function test_a_cancelled_transfer_says_nothing() {
            setStatus(start("a", "holiday.mp4"), "cancelled")
            compare(island.mode, "idle")
        }

        function test_one_failure_in_a_batch_is_not_hidden_by_a_later_success() {
            const a = start("a", "bad.bin")
            const b = start("b", "good.bin")
            setStatus(a, "failed")
            compare(island.transferJustFinished, false, "the other is still going")
            setStatus(b, "done")
            compare(island.primaryLabel, "Transfer failed")
            compare(island.secondaryLabel, "bad.bin")
        }

        function test_a_paused_transfer_keeps_its_name() {
            const a = start("a", "holiday.mp4")
            setStatus(a, "paused")
            compare(island.mode, "transfer")
            compare(island.primaryLabel, "holiday.mp4")
            compare(island.activeTransferId, "a")
        }

        function test_a_new_transfer_replaces_a_failed_banner_and_starts_afresh() {
            setStatus(start("a", "bad.bin"), "failed")
            const b = start("b", "good.bin")
            compare(island.primaryLabel, "good.bin")
            compare(island.transferJustFinished, false)
            setStatus(b, "done")
            compare(island.primaryLabel, "Transfer complete")
            compare(island.secondaryLabel, "good.bin")
        }

        function test_an_old_failure_in_the_list_does_not_colour_a_new_transfer() {
            TransferList.append({
                transferId: "old", title: "old.bin", progress: 0, rate: 0, incoming: false, status: "failed"
            })
            setStatus(start("a", "holiday.mp4"), "done")
            compare(island.primaryLabel, "Transfer complete")
        }

        function test_the_banner_goes_away() {
            setStatus(start("a", "holiday.mp4"), "failed")
            tryCompare(island, "mode", "idle", 5000)
            compare(island.transferFailed, false)
        }
    }
}
