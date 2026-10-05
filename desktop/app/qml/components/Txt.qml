// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Text in one of the design's type roles: displayLarge, displaySmall,
// headline, title, body, bodySmall, label, caption, code.
Text {
    id: txt
    property string role: "body"
    // Secondary text uses the variant color.
    property bool muted: false

    readonly property var spec: Theme.typography[role] || Theme.typography.body
    readonly property bool isDisplay: role === "displayLarge" || role === "displaySmall" || role === "headline"
    // Graphite labels are mono, uppercase and tracked (docs/design/README.md).
    readonly property bool graphiteLabel: Theme.graphite && (role === "label" || role === "caption")

    // Pixel size; set this (not font.pixelSize) to resize, so the role's
    // tracking scales with it.
    property int size: Math.round(graphiteLabel ? Theme.labelStyle.size : spec.size)

    color: muted ? Theme.surfaceContentVariant : Theme.surfaceContent
    font.family: role === "code" || graphiteLabel ? Theme.fontMono : (isDisplay ? Theme.fontDisplay : Theme.fontUi)
    font.pixelSize: size
    font.weight: graphiteLabel ? Theme.labelStyle.weight : (isDisplay && Theme.graphite ? 400 : spec.weight)
    font.letterSpacing: (graphiteLabel ? Theme.labelStyle.tracking : spec.tracking) * size
    font.capitalization: graphiteLabel ? Font.AllUppercase : Font.MixedCase
    // Token line heights are CSS-style (a multiple of the font size), not
    // a multiple of the font's own line spacing.
    lineHeight: Math.round(size * spec.lineHeight)
    lineHeightMode: Text.FixedHeight
    textFormat: Text.PlainText
    elide: Text.ElideRight
    Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
}
