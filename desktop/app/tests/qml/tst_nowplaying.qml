// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// This PC's players: with several, the card offers each to pick (#665).
Item {
    id: root
    width: 480
    height: 260

    NowPlaying {
        id: card
        width: 440
        deviceId: "local"
        playerId: "aimp"
        app: "AIMP"
        title: "Song"
        artist: ""
        playing: true
        duration: -1
        position: -1
        positionAt: 0
        art: ""
        canPlay: true
        canPause: true
        canNext: true
        canPrevious: true
        canSeek: false
    }

    SignalSpy { id: picked; target: card; signalName: "playerSelected" }

    function chips() {
        const found = []
        const walk = (item) => {
            for (let i = 0; i < item.children.length; i++) {
                const child = item.children[i]
                if (child.modelData !== undefined && child.modelData.id !== undefined)
                    found.push(child)
                walk(child)
            }
        }
        walk(card)
        return found
    }

    TestCase {
        name: "NowPlaying"
        when: windowShown

        function init() {
            picked.clear()
            card.deviceId = "local"
            card.pcPlayers = []
        }

        function test_one_player_has_no_picker() {
            card.pcPlayers = [{ id: "aimp", app: "AIMP", playing: true }]
            waitForRendering(card)
            compare(root.chips().length, 0)
        }

        function test_each_player_can_be_picked() {
            card.pcPlayers = [{ id: "spotify", app: "Spotify", playing: false }, { id: "aimp", app: "AIMP", playing: true }]
            waitForRendering(card)
            const chips = root.chips().filter(c => c.visible)
            compare(chips.length, 2)
            verify(chips[1].current, "the shown player is marked")
            verify(!chips[0].current)
            mouseClick(chips[0])
            compare(picked.count, 1)
            compare(picked.signalArguments[0][0], "spotify")
        }

        function test_a_phones_player_has_no_picker() {
            card.deviceId = "abc"
            card.pcPlayers = [{ id: "spotify", app: "Spotify", playing: false }, { id: "aimp", app: "AIMP", playing: true }]
            waitForRendering(card)
            compare(root.chips().filter(c => c.visible).length, 0)
        }
    }
}
