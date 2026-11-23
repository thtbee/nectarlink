// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Scrolls `flick` with the wheel or trackpad the way the system scrolls.
// Events that carry pixel distances (macOS passes them for every device, and
// for a wheel they come from the system's own scroll speed and acceleration)
// move the view by exactly those pixels. Events with only wheel notches
// (Windows, Linux) move it `notch` pixels a notch, eased. Qt Quick alone
// moves a view 72 px a notch, and on macOS drops the system's speed and
// acceleration for a mouse wheel, which feels slow beside other apps.
// Declare it beside the view, not inside it, which would attach the handler
// to the view's content instead.
Item {
    id: scroller
    required property Flickable flick
    // Pixels for one notch (120 units of angle) when no pixel distance comes.
    property real notch: 120
    // The wheel is moving the view.
    readonly property bool active: glide.running || rest.running
    // The wheel moved the view; `settled` when it has come to rest.
    signal moved()
    signal settled()

    function clamp(y) {
        const top = flick.originY - flick.topMargin
        const bottom = Math.max(top, flick.originY + flick.contentHeight + flick.bottomMargin - flick.height)
        return Math.max(top, Math.min(bottom, y))
    }

    WheelHandler {
        parent: scroller.flick
        target: null
        onWheel: (event) => {
            const flick = scroller.flick
            if (event.angleDelta.y === 0 && event.pixelDelta.y === 0) {
                event.accepted = false
                return
            }
            flick.cancelFlick()
            if (event.pixelDelta.y !== 0) {
                glide.stop()
                flick.contentY = scroller.clamp(flick.contentY - event.pixelDelta.y)
                rest.restart()
            } else {
                // From where the last notch is headed, so quick turns add up.
                const from = glide.running ? glide.to : flick.contentY
                const to = scroller.clamp(from - event.angleDelta.y / 120 * scroller.notch)
                glide.stop()
                glide.from = flick.contentY
                glide.to = to
                glide.duration = Theme.reduceMotion ? 0 : 140
                glide.start()
            }
            scroller.moved()
        }
    }

    // Pixel events come in streams: at rest when they stop.
    Timer {
        id: rest
        interval: 150
        onTriggered: scroller.settled()
    }

    NumberAnimation {
        id: glide
        target: scroller.flick
        property: "contentY"
        easing.type: Easing.OutCubic
        onFinished: scroller.settled()
    }
}
