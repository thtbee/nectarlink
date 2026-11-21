// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The ask to join Discord comes only when an update armed it, counts as
// shown when it appears, and joining ends it for good.
Item {
    id: root
    width: 640
    height: 200

    Component {
        id: promptComponent
        DiscordPrompt { delayMs: 10 }
    }

    SignalSpy {
        id: joinedSpy
        signalName: "joined"
    }

    TestCase {
        name: "DiscordPrompt"
        when: windowShown

        function init() {
            Preferences.communityPromptDone = false
            Preferences.communityPromptDue = false
            Preferences.communityPromptCount = 0
        }

        function make(eligible) {
            const prompt = createTemporaryObject(promptComponent, root, { eligible: eligible })
            joinedSpy.clear()
            joinedSpy.target = prompt
            return prompt
        }

        function test_not_without_an_update() {
            const prompt = make(true)
            wait(80)
            verify(!prompt.open)
            compare(Preferences.communityPromptCount, 0)
        }

        function test_waits_until_it_may_appear() {
            Preferences.communityPromptDue = true
            const prompt = make(false)
            wait(80)
            verify(!prompt.open)
            compare(Preferences.communityPromptDue, true, "still armed for later")
        }

        function test_counts_when_shown() {
            Preferences.communityPromptDue = true
            const prompt = make(true)
            tryVerify(() => prompt.open)
            compare(Preferences.communityPromptDue, false)
            compare(Preferences.communityPromptCount, 1)
            mouseClick(findChild(prompt, "closeButton"))
            verify(!prompt.open)
            compare(Preferences.communityPromptDone, false, "dismissing isn't joining")
            compare(joinedSpy.count, 0)
        }

        function test_joining_ends_it() {
            Preferences.communityPromptDue = true
            const prompt = make(true)
            tryVerify(() => prompt.open)
            mouseClick(findChild(prompt, "joinButton"))
            compare(joinedSpy.count, 1)
            compare(Preferences.communityPromptDone, true)
            verify(!prompt.open)
        }
    }
}
