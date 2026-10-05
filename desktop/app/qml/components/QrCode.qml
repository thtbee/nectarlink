// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// A QR code from Pairing.qrPath: vector modules on a white tile with the
// quiet zone scanners need. Always dark on light, in every theme, so it
// scans reliably; modules snap to whole pixels for crisp edges.
Rectangle {
    id: qr
    property string path
    property int modules: 21
    // Quiet zone around the code, in modules (the spec's minimum is 4; the
    // tile's own margin adds to it).
    readonly property int quietZone: 2

    readonly property int cell: Math.max(1, Math.floor(width / (modules + quietZone * 2)))

    radius: Theme.radiusLg
    color: "#FFFFFF"
    border.width: Theme.graphite ? 1 : 0
    border.color: Theme.outlineVariant

    Shape {
        width: qr.modules
        height: qr.modules
        x: Math.round((qr.width - qr.modules * qr.cell) / 2)
        y: Math.round((qr.height - qr.modules * qr.cell) / 2)
        scale: qr.cell
        transformOrigin: Item.TopLeft
        ShapePath {
            strokeWidth: -1
            fillColor: "#1A1918"
            PathSvg { path: qr.path }
        }
    }
}
