// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Effects
import app.nectarlink

// An image with rounded corners (a plain clip only cuts rectangles).
Item {
    id: root
    property alias source: image.source
    property alias status: image.status
    property alias fillMode: image.fillMode
    property alias sourceSize: image.sourceSize
    property alias implicitImageWidth: image.implicitWidth
    property alias implicitImageHeight: image.implicitHeight
    property real radius: Theme.radiusMd

    Image {
        id: image
        anchors.fill: parent
        asynchronous: true
        fillMode: Image.PreserveAspectCrop
        visible: false
        layer.enabled: true
    }
    Rectangle {
        id: mask
        anchors.fill: parent
        radius: root.radius
        visible: false
        layer.enabled: true
        layer.smooth: true
    }
    MultiEffect {
        anchors.fill: parent
        source: image
        maskEnabled: true
        maskSource: mask
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
        opacity: image.status === Image.Ready ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    }
}
