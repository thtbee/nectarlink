// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The call audio test card: labelled as a test, says which test ran and why,
// shows the microphone, the levels and the counters, words the hints, and
// starts and stops the test.
Item {
    id: root
    width: 600
    height: 1400

    CallAudioTestCard { id: card; width: 540 }

    function find(item, text) {
        if (item.text !== undefined && String(item.text).indexOf(text) >= 0 && item.visible)
            return item
        for (let i = 0; i < item.children.length; i++) {
            const found = find(item.children[i], text)
            if (found) return found
        }
        return null
    }

    readonly property var loopback: ({
        stage: "running", mode: "loopback", reason: "", elapsed_ms: 4000, remaining_ms: 56000,
        source: { name: "Studio Mic", native_rate: 48000, native_channels: 2, bluetooth: false, system_volume: 60, rate: 16000, channels: 1 },
        output: "Headphones",
        input: { rms: -24, peak: -12 }, loudest_db: -10, clipped: 0,
        returned: { rms: -25, peak: -13 },
        sent_chunks: 200, sent_bytes: 128000, dropped: 0, returned_chunks: 195, returned_bytes: 124800, late: 0, lost_ms: 0,
        latency_ms: { last: 90, average: 85, max: 140 }, hints: [], error: ""
    })

    TestCase {
        name: "CallAudioTestCard"
        when: windowShown

        function init() {
            PhoneCall.callAudioTest = "{}"
            PhoneCall.testStarts = 0
            PhoneCall.testStops = 0
        }

        function test_says_it_is_a_test_and_starts_one() {
            verify(root.find(card, "Not a real call"))
            verify(root.find(card, "Start test"))
            verify(!root.find(card, "Stop test"))
            verify(PhoneCall.testPolls > 0)
            const start = root.find(card, "Start test")
            mouseClick(start)
            compare(PhoneCall.testStarts, 1)
        }

        function test_shows_the_source_the_numbers_and_the_round_trip() {
            PhoneCall.callAudioTest = JSON.stringify(root.loopback)
            verify(card.active)
            verify(root.find(card, "Through your phone"))
            verify(root.find(card, "Studio Mic"))
            verify(root.find(card, "48 kHz, stereo"))
            verify(root.find(card, "16 kHz, mono, 16-bit"))
            verify(root.find(card, "Headphones"))
            verify(root.find(card, "200 chunks"))
            verify(root.find(card, "85 ms average"))
            verify(root.find(card, "Stop test"))
            verify(!root.find(card, "Start test"))
        }

        function test_a_local_test_says_why_it_stayed_on_the_pc() {
            const local = JSON.parse(JSON.stringify(root.loopback))
            local.mode = "local"; local.stage = "recording"; local.reason = "no_test"; local.latency_ms = null
            PhoneCall.callAudioTest = JSON.stringify(local)
            verify(root.find(card, "This PC only"))
            verify(root.find(card, "update it"))
            local.reason = "offline"
            PhoneCall.callAudioTest = JSON.stringify(local)
            verify(root.find(card, "no phone is connected"))
            verify(!root.find(card, "ms average"))
        }

        function test_words_the_hints_and_the_failure() {
            const bad = JSON.parse(JSON.stringify(root.loopback))
            bad.stage = "failed"; bad.error = "macOS keeps NectarLink off the microphone."
            bad.hints = ["quiet", "clipping", "bluetooth", "dropped", "link"]
            PhoneCall.callAudioTest = JSON.stringify(bad)
            verify(root.find(card, "almost silent"))
            verify(root.find(card, "cut off"))
            verify(root.find(card, "Bluetooth headset"))
            verify(root.find(card, "connection couldn't keep up"))
            verify(root.find(card, "macOS keeps NectarLink off"))
            verify(root.find(card, "Test again"))
        }

        function test_meter_readings_show_minus_infinity_at_the_floor() {
            compare(card.db(-90), "-∞")
            compare(card.db(-12.4), "-12")
        }
    }
}
