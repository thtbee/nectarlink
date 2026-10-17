// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// An image clipped to its bounds.
Item {
    id: root
    property alias source: image.source
    property alias status: image.status
    property alias fillMode: image.fillMode
    property alias sourceSize: image.sourceSize
    property alias implicitImageWidth: image.implicitWidth
    property alias implicitImageHeight: image.implicitHeight
    property real radius: Theme.radiusMd
    clip: true

    Image {
        id: image
        anchors.fill: parent
        asynchronous: true
        mipmap: true
        smooth: true
        fillMode: Image.PreserveAspectCrop
        opacity: status === Image.Ready ? 1 : 0
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeNormal } }
    }
}
