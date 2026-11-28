// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// A Deck tile shows its picture instead of its icon (#490), and its icon
// again when the picture is gone or can't be read.
Item {
    id: root
    width: 120
    height: 120

    // A 2 by 2 orange PNG.
    readonly property string png: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4v4DhPwgzwBgAX4oKeUfxrOMAAAAASUVORK5CYII="

    DeckBadge {
        id: badge
        width: 34
        height: 34
        iconPath: Icons.play
    }

    TestCase {
        name: "DeckBadge"
        when: windowShown

        function icon() { return findChild(badge, "deckBadgeIcon") }
        function picture() { return findChild(badge, "deckBadgePicture") }

        function test_shows_the_icon_without_a_picture() {
            badge.imageUrl = ""
            verify(!badge.showsPicture)
            verify(icon().visible)
            verify(!picture().visible)
        }

        function test_shows_the_picture_instead_of_the_icon() {
            badge.imageUrl = root.png
            tryCompare(picture(), "status", Image.Ready)
            verify(badge.showsPicture)
            verify(picture().visible)
            verify(!icon().visible)
            badge.imageUrl = ""
            verify(icon().visible)
        }

        function test_falls_back_to_the_icon_when_the_picture_cant_be_read() {
            badge.imageUrl = "file:///no/such/deck-picture.png"
            tryCompare(badge, "showsPicture", false)
            verify(icon().visible)
            verify(!picture().visible)
        }
    }
}
