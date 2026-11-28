// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A Deck tile's badge: the picture the user gave the tile (#490), rounded
// like the tile, or else its icon on the badge's colour. A picture that
// can't be shown falls back to the icon.
Rectangle {
    id: badge
    // A file: or data: URL of the tile's picture, or "".
    property string imageUrl: ""
    property string iconPath: ""
    property color iconColor: Theme.surfaceContent
    property real iconSize: 18
    readonly property bool showsPicture: imageUrl.length > 0 && picture.status !== Image.Error

    radius: Theme.radiusSm

    Icon {
        objectName: "deckBadgeIcon"
        anchors.centerIn: parent
        width: badge.iconSize
        height: badge.iconSize
        visible: !badge.showsPicture
        path: badge.iconPath
        color: badge.iconColor
    }

    RoundedImage {
        id: picture
        objectName: "deckBadgePicture"
        anchors.fill: parent
        visible: badge.showsPicture
        radius: badge.radius
        source: badge.imageUrl
        sourceSize: Qt.size(192, 192)
    }

    // A hairline keeps a picture's edge clear on light and dark tiles alike.
    Rectangle {
        anchors.fill: parent
        visible: badge.showsPicture
        radius: badge.radius
        color: "transparent"
        border.width: 1
        border.color: Theme.outlineVariant
    }
}
