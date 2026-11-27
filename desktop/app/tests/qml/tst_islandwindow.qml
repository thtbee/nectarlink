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

    // A stand-in for a button, for the hover-hint service.
    Item {
        id: tipOwner
        parent: island.contentItem
        width: 10
        height: 10
        visible: false
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

        function player(device, id, title) {
            MediaList.append({
                deviceId: device, playerId: id, app: id, title: title, artist: "", art: "",
                playing: true, canPlay: true, canPause: true, canNext: true, canPrevious: true,
                canSeek: true, duration: 0, position: 0, positionAt: 0
            })
        }

        function test_a_pc_player_shows_without_a_phone_and_switches_with_a_phones() {
            MediaList.clear()
            island.mediaPick = ""
            player("local", "spotify", "On the PC")
            // This fork keeps a PC-only player on the Island even when "This
            // PC's players too" is off: it has to work with no phone at all.
            compare(island.mode, "media", "a PC-only player shows without a phone")
            compare(island.primaryLabel, "On the PC")
            Preferences.nectarIslandPcMedia = true
            compare(island.mode, "media")
            compare(island.primaryLabel, "On the PC")
            player("phone1", "music", "On the phone")
            compare(island.mediaSources, 2)
            compare(island.primaryLabel, "On the phone", "a phone's comes first")
            island.nextMediaSource()
            compare(island.primaryLabel, "On the PC")
            island.nextMediaSource()
            compare(island.primaryLabel, "On the phone")
            // The one picked stops: the other shows.
            island.nextMediaSource()
            MediaList.setProperty(0, "playing", false)
            compare(island.primaryLabel, "On the phone")
            // With a phone playing, the PC's player only joins when asked.
            MediaList.setProperty(0, "playing", true)
            Preferences.nectarIslandPcMedia = false
            compare(island.mediaSources, 1, "the PC's players stay out while a phone plays")
            MediaList.clear()
        }

        function test_it_sits_left_center_or_right() {
            const center = island.x
            Preferences.nectarIslandPosition = "left"
            verify(island.x < center)
            Preferences.nectarIslandPosition = "right"
            verify(island.x > center)
            Preferences.nectarIslandPosition = "center"
            compare(island.x, center)
        }

        function test_the_collapsed_island_has_one_fixed_shape() {
            island.closePanel()
            Preferences.nectarIslandOpacity = 100
            tryCompare(island, "windowRoomy", false, 1000)
            compare(island.width, 340)
            compare(island.height, 46)
            compare(island.compactPillHeight, 46)
            compare(island.expandedHeight, 368)
        }

        function test_the_window_does_not_shrink_in_the_middle_of_the_collapse() {
            const savedReduceMotion = AppController.reduceMotion
            AppController.reduceMotion = false
            island.panelHovered = true
            island.panelShown = true
            island.panelPinned = true
            tryCompare(island, "windowRoomy", true, 1000)
            island.closePanel()
            compare(island.expanded, false)
            wait(120)
            compare(island.windowRoomy, true, "the native window waits for the pill")
            tryCompare(island, "windowRoomy", false, 1000)
            AppController.reduceMotion = savedReduceMotion
        }

        function test_opacity_is_clamped_to_a_usable_range() {
            Preferences.nectarIslandOpacity = 100
            compare(island.contentOpacity, 1)
            Preferences.nectarIslandOpacity = 55
            compare(island.contentOpacity, 0.55)
            Preferences.nectarIslandOpacity = 5
            compare(island.contentOpacity, 0.3)
            Preferences.nectarIslandOpacity = 100
        }

        function test_the_settings_page_opens_inside_the_island_and_closes_with_its_own_x() {
            island.closePanel()
            island.openIslandSettings()
            compare(island.islandSettingsOpen, true)
            compare(island.panelShown, true)
            compare(island.expanded, true)
            island.closeIslandSettings()
            compare(island.islandSettingsOpen, false)
            compare(island.panelShown, true, "closing settings leaves the card open")
            island.closePanel()
        }

        function test_click_through_still_opens_on_the_cursor() {
            if (Qt.platform.os !== "windows")
                skip("cursor polling for click-through is Windows-only")
            island.closePanel()
            island.panelSuppressed = false
            Preferences.nectarIslandClickThrough = true
            compare(island.clickThroughActive, true)
            island.clickThroughHovered = true
            compare(island.pointerOverIsland, true)
            compare(island.expanded, true)
            tryCompare(island, "unlockButtonVisible", true, 1000)
            island.turnOffClickThrough()
            compare(island.clickThroughActive, false)
            compare(island.unlockButtonVisible, false)
            island.clickThroughHovered = false
            compare(island.pointerOverIsland, false)
            tryCompare(island, "panelShown", false, 1000)
            compare(island.expanded, false)
            Preferences.nectarIslandClickThrough = false
            island.closePanel()
        }

        function test_a_hover_hint_waits_then_shows_and_hides() {
            island.showTip("A hint", tipOwner, Qt.point(4, 4))
            compare(island.tipText, "", "the hint waits before it appears")
            tryCompare(island, "tipText", "A hint", 1000)
            island.moveTip(tipOwner, Qt.point(30, 20))
            compare(island.tipText, "A hint")
            island.hideTip(tipOwner)
            compare(island.tipText, "")
        }

        function test_opening_settings_keeps_the_island_size() {
            island.panelHovered = true
            island.panelShown = true
            island.panelPinned = true
            wait(350)
            const before = island.height
            island.openIslandSettings()
            island.panelPinned = true
            wait(450)
            compare(island.islandSettingsOpen, true)
            compare(island.height, before, "settings must not resize the open Island")
            island.closeIslandSettings()
            island.closePanel()
        }

        function test_lyrics_need_a_confident_match() {
            const songs = [
                { name: "Some Song", artists: [], duration: 90000 },
                { name: "Real Song", artists: [{ name: "Real Artist" }], duration: 200000 }
            ]
            compare(island.pickLyricSong(songs, "Unrelated Video", "", 0), null,
                "a video title must not borrow a song's lyrics")
            const found = island.pickLyricSong(songs, "Real Song", "Real Artist", 200500)
            compare(found ? found.name : "", "Real Song")
        }

        function test_a_notification_says_which_app_it_is_from() {
            NotificationList.latestApp = "Signal"
            NotificationList.latestTitle = "Sam"
            NotificationList.count = 1
            compare(island.mode, "notification")
            compare(island.primaryLabel, "Sam")
            compare(island.secondaryLabel, "Signal")
            NotificationList.count = 3
            compare(island.secondaryLabel, "Signal · 3 notification(s)")
            NotificationList.count = 0
            NotificationList.latestApp = ""
            NotificationList.latestTitle = ""
            compare(island.mode, "idle")
        }

        function test_the_banner_goes_away() {
            setStatus(start("a", "holiday.mp4"), "failed")
            tryCompare(island, "mode", "idle", 5000)
            compare(island.transferFailed, false)
        }
    }
}
