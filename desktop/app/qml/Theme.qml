// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
import app.nectarlink

// The active theme: Bloom (Material You, seed presets) or Graphite (paper and
// slate), light or dark, resolved from Preferences and the system. Colors
// come from Tokens (generated from docs/design/tokens.json); Material's
// "onX" roles are named "xContent" because QML reserves the "on" prefix.
QtObject {
    id: theme

    readonly property bool graphite: Preferences.theme === "graphite"
    readonly property bool dark: Preferences.colorMode === "dark"
                                 || (Preferences.colorMode === "system" && AppController.systemDark)
    readonly property bool reduceMotion: AppController.reduceMotion

    readonly property var bloomSeeds: Tokens.data.themes.bloom.seeds
    readonly property string seed: bloomSeeds[Preferences.seed] ? Preferences.seed : Tokens.data.themes.bloom.defaultSeed
    readonly property var palette: graphite
        ? Tokens.data.themes.graphite.variants[dark ? "slate" : "paper"]
        : bloomSeeds[seed][dark ? "dark" : "light"]
    readonly property var status: graphite ? palette : Tokens.data.themes.bloom.status[dark ? "dark" : "light"]

    // ---- Color roles ----
    readonly property color primary: palette.primary
    readonly property color primaryContent: palette.onPrimary
    readonly property color primaryContainer: palette.primaryContainer
    readonly property color primaryContainerContent: palette.onPrimaryContainer
    readonly property color secondaryContainer: palette.secondaryContainer
    readonly property color secondaryContainerContent: palette.onSecondaryContainer
    readonly property color surface: palette.surface
    readonly property color surfaceLow: palette.surfaceContainerLow
    readonly property color surfaceContainer: palette.surfaceContainer
    readonly property color surfaceContainerHigh: palette.surfaceContainerHigh
    readonly property color surfaceContainerHighest: palette.surfaceContainerHighest
    readonly property color surfaceContent: palette.onSurface
    readonly property color surfaceContentVariant: palette.onSurfaceVariant
    readonly property color outline: palette.outline
    readonly property color outlineVariant: palette.outlineVariant
    // Success never gets its own (green) color: it uses the theme's ink.
    readonly property color success: primary
    readonly property color warning: status.warning
    readonly property color error: status.error
    readonly property color errorContent: status.onError

    // ---- Component colors (docs/design/assets/nl.css) ----
    readonly property color cardColor: graphite ? "transparent" : surfaceContainer
    readonly property color cardBorder: graphite ? outlineVariant : "transparent"
    readonly property color tileColor: graphite ? "transparent" : surfaceContainerHigh
    readonly property color heroColor: graphite ? "transparent"
        : Qt.tint(surface, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, 0.62))
    readonly property color heroContent: graphite ? surfaceContent : primaryContainerContent
    readonly property color railColor: graphite ? "transparent" : surfaceLow
    readonly property color indicator: graphite ? surfaceContent : secondaryContainer
    readonly property color indicatorContent: graphite ? surface : secondaryContainerContent
    readonly property color buttonShadow: graphite ? palette.buttonShadow : "transparent"
    readonly property color scrim: Qt.rgba(0, 0, 0, dark ? 0.5 : 0.32)

    // ---- Shape ----
    readonly property var shape: graphite ? Tokens.data.themes.graphite.shape : Tokens.data.themes.bloom.shape
    readonly property int radiusXs: shape.xs
    readonly property int radiusSm: shape.sm
    readonly property int radiusMd: shape.md
    readonly property int radiusLg: shape.lg
    readonly property int radiusXl: shape.xl
    // "Pill": half the height of whatever uses it, in Bloom.
    function pill(height) { return graphite ? shape.full : height / 2 }

    // ---- Type ----
    // Bundled fonts (Figtree, Instrument Serif, Space Mono, Caveat) replace
    // these system fallbacks once added to the build.
    readonly property string fontUi: "Segoe UI Variable Text"
    readonly property string fontDisplay: graphite ? "Georgia" : "Segoe UI Variable Display"
    readonly property string fontMono: "Cascadia Mono"
    readonly property var typography: Tokens.data.typography
    readonly property var labelStyle: graphite ? Tokens.data.themes.graphite.label : Tokens.data.themes.bloom.label

    // ---- Layout and spacing ----
    readonly property var layout: Tokens.data.layout.desktop
    readonly property int railWidth: layout.railWidth
    readonly property int topBarHeight: layout.topBarHeight
    readonly property int contentPadding: layout.contentPadding
    readonly property int gutter: layout.gutter
    function space(step) { return Tokens.data.spacing.scale[step] }

    // ---- Motion ----
    // QML springs take a strength and a damping ratio; these are tuned to
    // match the token springs (gentle / standard / snappy).
    readonly property real springGentle: 2.4
    readonly property real dampingGentle: 0.36
    readonly property real springStandard: 3.6
    readonly property real dampingStandard: 0.32
    readonly property real springSnappy: 5.6
    readonly property real dampingSnappy: 0.34
    readonly property int fadeFast: Tokens.data.motion.fade.fast
    readonly property int fadeNormal: Tokens.data.motion.fade.normal
    readonly property real hoverLift: reduceMotion ? 0 : Tokens.data.motion.hoverLift
    readonly property real pressScale: reduceMotion ? 1 : Tokens.data.motion.pressScale
}
