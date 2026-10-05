// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// The 6-digit pairing code, one digit per tile, as both screens show it.
Row {
    id: digits
    property string code
    spacing: 8

    Accessible.role: Accessible.StaticText
    Accessible.name: code.split("").join(" ")

    Repeater {
        model: digits.code.split("")
        delegate: Rectangle {
            required property string modelData
            width: 44; height: 56
            radius: Theme.radiusMd
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHigh
            border.width: Theme.graphite ? 1 : 0
            border.color: Theme.outlineVariant
            Txt {
                anchors.centerIn: parent
                text: parent.modelData
                role: "code"
                size: 26
                font.letterSpacing: 0
            }
        }
    }
}
