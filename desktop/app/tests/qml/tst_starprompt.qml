// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The star prompt shows right after an update or after some use on a new install,
// and starring or closing it ends it for good.
Item {
    id: root
    width: 640
    height: 200

    Component {
        id: promptComponent
        StarPrompt {
            minuteMs: 10
            minutesOfUse: 3
            updatedWaitMs: 10
        }
    }

    SignalSpy {
        id: supportedSpy
        signalName: "supported"
    }

    SignalSpy {
        id: starredSpy
        signalName: "starred"
    }

    TestCase {
        name: "StarPrompt"
        when: windowShown

        function init() {
            Preferences.starPromptDone = false
            Preferences.starPromptUpdated = false
            Preferences.starPromptMinutes = 0
        }

        function make(eligible) {
            const prompt = createTemporaryObject(promptComponent, root, { eligible: eligible })
            starredSpy.clear()
            starredSpy.target = prompt
            return prompt
        }

        function waitUntilOpen(prompt) {
            tryCompare(prompt, "phase", "open")
            const star = findChild(prompt, "starButton")
            tryVerify(() => star.enabled, 5000)
            return star
        }

        function test_it_waits_until_it_may_appear() {
            const prompt = make(false)
            wait(60)
            compare(prompt.phase, "waiting")
            verify(!prompt.visible)
            prompt.eligible = true
            tryCompare(prompt, "phase", "open")
            verify(prompt.visible)
        }

        function test_a_new_install_waits_for_enough_use() {
            const prompt = make(true)
            tryVerify(() => Preferences.starPromptMinutes >= 1)
            prompt.eligible = false
            const counted = Preferences.starPromptMinutes
            wait(60)
            compare(Preferences.starPromptMinutes, counted)
            prompt.eligible = true
            tryCompare(prompt, "phase", "open")
            compare(Preferences.starPromptMinutes, 3)
        }

        function test_use_adds_up_across_sessions() {
            Preferences.starPromptMinutes = 2
            const prompt = make(true)
            tryCompare(prompt, "phase", "open")
            compare(Preferences.starPromptMinutes, 3)
        }

        function test_someone_who_updated_is_asked_without_counting() {
            Preferences.starPromptUpdated = true
            const prompt = make(true)
            tryCompare(prompt, "phase", "open")
            compare(Preferences.starPromptMinutes, 0)
        }

        function test_an_update_doesnt_bring_back_an_answer() {
            Preferences.starPromptDone = true
            Preferences.starPromptUpdated = true
            const prompt = make(true)
            wait(60)
            compare(prompt.phase, "waiting")
            verify(!prompt.visible)
        }

        function test_it_never_shows_once_answered() {
            Preferences.starPromptDone = true
            const prompt = make(true)
            wait(60)
            compare(prompt.phase, "waiting")
            verify(!prompt.visible)
        }

        function test_starring_opens_the_repo_and_ends_it() {
            const prompt = make(true)
            const star = waitUntilOpen(prompt)
            mouseClick(star)
            compare(starredSpy.count, 1)
            verify(Preferences.starPromptDone)
            compare(prompt.phase, "thanks")
            tryCompare(prompt, "phase", "gone", 6000)
            verify(!prompt.visible)
        }

        function test_support_opens_the_support_page_and_ends_it() {
            const prompt = make(true)
            supportedSpy.clear()
            supportedSpy.target = prompt
            waitUntilOpen(prompt)
            mouseClick(findChild(prompt, "supportButton"))
            compare(supportedSpy.count, 1)
            compare(starredSpy.count, 0)
            verify(Preferences.starPromptDone)
            compare(prompt.phase, "thanks")
            tryCompare(prompt, "phase", "gone", 6000)
        }

        function test_closing_ends_it_without_opening_anything() {
            const prompt = make(true)
            waitUntilOpen(prompt)
            mouseClick(findChild(prompt, "closeButton"))
            compare(starredSpy.count, 0)
            verify(Preferences.starPromptDone)
            tryCompare(prompt, "phase", "gone", 6000)
            verify(!prompt.visible)
        }

        function test_a_second_press_does_nothing() {
            const prompt = make(true)
            waitUntilOpen(prompt)
            prompt.star()
            prompt.star()
            prompt.dismiss()
            compare(starredSpy.count, 1)
        }
    }
}
