// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// An IconButton's tooltip shows under its button, in the window's top
// layer, wherever the button sits (it once showed at the window's top-left).
Item {
    id: root
    width: 400
    height: 300

    Item {
        x: 120
        y: 90
        width: 200
        height: 100
        IconButton {
            id: button
            x: 40
            y: 20
            iconPath: Icons.pause
            label: "Pause"
        }
    }

    TestCase {
        name: "IconButton"
        when: windowShown

        function tooltip() {
            const layer = root.Window.contentItem
            for (let i = 0; i < layer.children.length; i++) {
                const child = layer.children[i]
                if (child.hasOwnProperty("shown") && child.hasOwnProperty("reposition"))
                    return child
            }
            return null
        }

        function test_tooltip_shows_under_its_button() {
            mouseMove(button, button.width / 2, button.height / 2)
            let tip = null
            tryVerify(() => { tip = tooltip(); return tip !== null && tip.visible && tip.opacity === 1 },
                      3000, "the tooltip shows")
            const under = button.mapToItem(root.Window.contentItem, button.width / 2, button.height)
            verify(Math.abs(tip.x + tip.width / 2 - under.x) <= 1, "centered on the button: " + tip.x)
            verify(tip.y >= under.y && tip.y <= under.y + 12, "just under the button: " + tip.y)
            mouseMove(root, 5, 5)
            tryVerify(() => !tip.visible, 3000, "the tooltip hides when the pointer leaves")
        }

        // The small dot for something new shows only while `dot` is set.
        function test_dot_shows_only_when_set() {
            function dotItem() {
                for (let i = 0; i < button.children.length; i++) {
                    const child = button.children[i]
                    if (child.width === 6 && child.height === 6)
                        return child
                }
                return null
            }
            const d = dotItem()
            verify(d !== null, "the button has a dot")
            button.dot = false
            tryVerify(() => !d.visible, 2000, "no dot by default")
            button.dot = true
            tryVerify(() => d.visible, 2000, "the dot shows when set")
            compare(button.Accessible.description, "New changes")
            button.dot = false
            tryVerify(() => !d.visible, 2000, "and goes again")
        }
    }
}
