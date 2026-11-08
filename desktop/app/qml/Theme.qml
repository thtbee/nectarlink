// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
import app.nectarlink

// The active theme: Bloom (Material You, seed presets, the default), Hive
// (honey on charcoal or wax), Glass, Graphite (paper and slate), or Fluent
// (Windows 11's own look), light or dark, resolved from Preferences and the system. Colors
// come from Tokens (generated from docs/design/tokens.json); Material's
// "onX" roles are named "xContent" because QML reserves the "on" prefix.
QtObject {
    id: theme

    readonly property bool hive: Preferences.theme === "hive"
    readonly property bool graphite: Preferences.theme === "graphite"
    readonly property bool glass: Preferences.theme === "glass" || Preferences.theme === "blurry"
    // Windows 11 (WinUI 3): Mica, layered cards, 4 px controls, the system accent.
    readonly property bool fluent: Preferences.theme === "fluent"
    // Windows' accent ({ light, dark }), or null elsewhere: Fluent then uses its own blue.
    readonly property var systemAccent: AppController.systemAccent !== ""
        ? JSON.parse(AppController.systemAccent) : null
    readonly property bool dark: Preferences.colorMode === "dark"
                                 || (Preferences.colorMode === "system" && AppController.systemDark)
    readonly property bool amoled: dark && Preferences.amoled && (graphite || Preferences.seed === "black")
    readonly property bool reduceMotion: AppController.reduceMotion

    // ---- Hive's comb ----
    readonly property var comb: Tokens.data.themes.hive.comb
    // Breathes and pulses: the user's setting, never with reduced motion or
    // battery saver (it holds still then).
    readonly property bool combMotion: hive && Preferences.combMotion && !reduceMotion && !AppController.powerSaver
    readonly property color combLine: Qt.rgba(primary.r, primary.g, primary.b,
                                              dark ? comb.lineAlphaDark : comb.lineAlphaLight)
    readonly property color combGlow: dark ? Qt.rgba(1.0, 0.62, 0.18, comb.glowAlphaDark)
                                           : Qt.rgba(1.0, 0.72, 0.37, comb.glowAlphaLight)
    // One clock for every comb in the app (seconds), so two combs never draw
    // twice as often. The main window runs it while it's in front.
    property real combTime: 0
    property bool combClockRunning: false
    // A comb is on screen outside Hive (the invitation to try it).
    property bool combPreviewShown: false
    property double _combSince: 0
    property real _combBase: 0
    readonly property Timer _combClock: Timer {
        interval: theme.comb.frameMs
        repeat: true
        running: theme.combClockRunning
                 && (theme.combMotion || (theme.combPreviewShown && !theme.reduceMotion && !AppController.powerSaver))
        onRunningChanged: {
            if (running)
                theme._combSince = Date.now()
            else
                theme._combBase = theme.combTime
        }
        onTriggered: theme.combTime = theme._combBase + (Date.now() - theme._combSince) / 1000
    }

    readonly property var bloomSeeds: Tokens.data.themes.bloom.seeds
    // Colors made from the desktop wallpaper (or null until they're known),
    // in the shape of a preset seed.
    readonly property var wallpaperSeed: AppController.wallpaperColors !== ""
        ? JSON.parse(AppController.wallpaperColors) : null
    // Material You color scheme extracted from the currently playing media artwork.
    readonly property var artworkSeed: {
        if (MediaList.activeColors === "")
            return null
        try {
            return JSON.parse(MediaList.activeColors)
        } catch (e) {
            return null
        }
    }
    readonly property string artworkUrl: MediaList.activeArt
    readonly property bool isArtworkSeed: Preferences.seed === "artwork"
    readonly property bool artworkAmbientActive: false
    readonly property bool ambientBlurActive: glass
    // Material You color schemes from paired phones (`[{ id, key, name, label, hasAccent, scheme }]`).
    readonly property var phoneSeeds: AppController.phoneColors !== ""
        ? JSON.parse(AppController.phoneColors) : []
    readonly property bool isPhoneSeed: Preferences.seed === "phone" || Preferences.seed.startsWith("phone:")
    readonly property var activePhoneSeed: {
        if (!isPhoneSeed)
            return null
        const targetId = Preferences.seed.startsWith("phone:") ? Preferences.seed.slice(6) : ""
        for (let i = 0; i < phoneSeeds.length; i++) {
            const p = phoneSeeds[i]
            if (targetId !== "" && p.id === targetId && p.scheme)
                return p.scheme
        }
        for (let i = 0; i < phoneSeeds.length; i++) {
            if (phoneSeeds[i].scheme)
                return phoneSeeds[i].scheme
        }
        return null
    }
    function phonePalette(deviceId) {
        for (let i = 0; i < phoneSeeds.length; i++) {
            const p = phoneSeeds[i]
            if ((deviceId === "" || p.id === deviceId) && p.scheme)
                return p.scheme[dark ? "dark" : "light"]
        }
        return null
    }
    // "artwork", "wallpaper", "phone" / "phone:<id>", or a preset name; dynamic
    // seeds fall back to wallpaper or the default preset until their colors are known.
    readonly property string seed: isArtworkSeed
        ? (artworkSeed ? "artwork" : (wallpaperSeed ? "wallpaper" : Tokens.data.themes.bloom.defaultSeed))
        : isPhoneSeed
          ? (activePhoneSeed ? Preferences.seed : (wallpaperSeed ? "wallpaper" : Tokens.data.themes.bloom.defaultSeed))
          : Preferences.seed === "wallpaper"
            ? (wallpaperSeed ? "wallpaper" : Tokens.data.themes.bloom.defaultSeed)
            : (bloomSeeds[Preferences.seed] ? Preferences.seed : Tokens.data.themes.bloom.defaultSeed)
    readonly property var seedColors: seed === "artwork"
        ? artworkSeed
        : (seed === "phone" || seed.startsWith("phone:"))
          ? activePhoneSeed
          : (seed === "wallpaper" ? wallpaperSeed : bloomSeeds[seed])
    readonly property var palette: hive
        ? Tokens.data.themes.hive.variants[dark ? "comb" : "wax"]
        : (graphite
            ? Tokens.data.themes.graphite.variants[dark ? "slate" : "paper"]
            : (fluent
                ? Tokens.data.themes.fluent.variants[dark ? "dark" : "light"]
                : seedColors[dark ? "dark" : "light"]))
    readonly property var status: graphite || hive || fluent ? palette : Tokens.data.themes.bloom.status[dark ? "dark" : "light"]

    // ---- Color roles ----
    readonly property color primary: fluent && systemAccent ? systemAccent[dark ? "dark" : "light"] : palette.primary
    readonly property color primaryContent: palette.onPrimary
    // Fluent: a light wash of the accent, so it follows the system accent too.
    readonly property color primaryContainer: fluent
        ? Qt.tint(palette.surfaceContainer, Qt.rgba(primary.r, primary.g, primary.b, dark ? 0.18 : 0.12))
        : palette.primaryContainer
    readonly property color primaryContainerContent: fluent ? palette.onSurface : palette.onPrimaryContainer
    readonly property color secondaryContainer: palette.secondaryContainer
    readonly property color secondaryContainerContent: palette.onSecondaryContainer
    readonly property color surface: amoled ? "#000000" : palette.surface
    readonly property color surfaceLow: amoled ? "#050507" : palette.surfaceContainerLow
    readonly property color surfaceContainerLow: amoled ? "#050507" : palette.surfaceContainerLow
    readonly property color surfaceContainer: amoled ? "#0A0B0E" : palette.surfaceContainer
    readonly property color surfaceContainerHigh: amoled ? "#121318" : palette.surfaceContainerHigh
    readonly property color surfaceContainerHighest: amoled ? "#1A1C23" : palette.surfaceContainerHighest
    readonly property color surfaceContent: palette.onSurface
    readonly property color surfaceContentVariant: palette.onSurfaceVariant
    readonly property color outline: palette.outline
    readonly property color outlineVariant: amoled ? "#22242C" : palette.outlineVariant
    // Success never gets its own (green) color: it uses the theme's ink.
    readonly property color success: primary
    readonly property color warning: status.warning
    readonly property color error: status.error
    readonly property color errorContent: status.onError
    // A quiet error surface (the "sharing your screen" pill): the error hue
    // over the surface, with the surface's own ink so text stays readable.
    readonly property color errorContainer: Qt.tint(surfaceContainerHigh, Qt.rgba(error.r, error.g, error.b, dark ? 0.22 : 0.14))
    readonly property color errorContainerContent: surfaceContent

    // ---- Component colors (docs/design/assets/nl.css) ----
    // Hive: nearly opaque, so the comb shows through as texture without
    // ever crossing text.
    // Fluent: WinUI's card fill over the page layer (CardBackgroundFillColorDefault).
    readonly property color cardColor: fluent
        ? (dark ? Qt.rgba(1, 1, 1, 0.051) : Qt.rgba(1, 1, 1, 0.70))
        : hive
        ? (function() {
            const c = Qt.tint(surfaceContainer, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, dark ? 0.12 : 0.18))
            return Qt.rgba(c.r, c.g, c.b, dark ? 0.90 : 0.92)
        })()
        : graphite ? "transparent"
        : (glass
            ? Qt.tint(
                Qt.rgba(surfaceContainer.r, surfaceContainer.g, surfaceContainer.b, amoled ? 0.56 : (dark ? 0.36 : 0.52)),
                Qt.rgba(primary.r, primary.g, primary.b, dark ? 0.08 : 0.06)
            )
            : (artworkAmbientActive
                ? Qt.rgba(surfaceContainer.r, surfaceContainer.g, surfaceContainer.b, dark ? 0.74 : 0.80)
                : (amoled
                    ? Qt.tint(surfaceContainer, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, 0.08))
                    : Qt.tint(
                        surfaceContainer,
                        Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, isPhoneSeed ? (dark ? 0.28 : 0.36) : (dark ? 0.16 : 0.22))
                    ))))
    // Hive: a faint honey edge, like wax around a cell.
    readonly property color cardBorder: fluent
        ? (dark ? Qt.rgba(0, 0, 0, 0.10) : Qt.rgba(0, 0, 0, 0.059))
        : hive
        ? Qt.rgba(primary.r, primary.g, primary.b, dark ? 0.14 : 0.16)
        : graphite ? outlineVariant
        : (glass
            ? (dark ? Qt.rgba(1, 1, 1, 0.08) : Qt.rgba(1, 1, 1, 0.28))
            : (artworkAmbientActive
                ? Qt.rgba(outlineVariant.r, outlineVariant.g, outlineVariant.b, dark ? 0.36 : 0.45)
                : (amoled ? "#1D1F27" : "transparent")))
    readonly property color tileColor: fluent
        ? (dark ? Qt.rgba(1, 1, 1, 0.061) : Qt.rgba(1, 1, 1, 0.70))
        : hive
        ? Qt.tint(surfaceContainerHigh, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, dark ? 0.14 : 0.20))
        : graphite ? "transparent"
        : (glass
            ? Qt.tint(
                Qt.rgba(1, 1, 1, dark ? 0.08 : 0.48),
                Qt.rgba(primary.r, primary.g, primary.b, dark ? 0.08 : 0.06)
            )
            : (artworkAmbientActive
                ? Qt.rgba(surfaceContainerHigh.r, surfaceContainerHigh.g, surfaceContainerHigh.b, dark ? 0.76 : 0.82)
                : (amoled
                    ? Qt.tint(surfaceContainerHigh, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, 0.10))
                    : Qt.tint(
                        surfaceContainerHigh,
                        Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, isPhoneSeed ? (dark ? 0.32 : 0.42) : (dark ? 0.18 : 0.24))
                    ))))
    readonly property color heroColor: fluent ? cardColor
        : hive
        ? Qt.tint(surface, Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, dark ? 0.55 : 0.62))
        : graphite ? "transparent"
        : Qt.tint(
            ambientBlurActive
                ? Qt.rgba(surface.r, surface.g, surface.b, glass ? (amoled ? 0.54 : (dark ? 0.34 : 0.48)) : (dark ? 0.65 : 0.72))
                : (amoled ? "#090A0E" : surface),
            Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, glass ? (dark ? 0.30 : 0.38) : (artworkAmbientActive ? 0.52 : (amoled ? 0.32 : 0.68)))
        )
    readonly property color heroContent: graphite || fluent ? surfaceContent : primaryContainerContent
    // Hive's rail is the comb itself; Fluent's sits straight on Mica.
    readonly property color railColor: graphite || hive || fluent ? "transparent"
        : (glass
            ? Qt.rgba(surfaceLow.r, surfaceLow.g, surfaceLow.b, amoled ? 0.48 : (dark ? 0.28 : 0.42))
            : (artworkAmbientActive
                ? Qt.rgba(surfaceLow.r, surfaceLow.g, surfaceLow.b, dark ? 0.62 : 0.68)
                : (amoled
                    ? "#030305"
                    : Qt.tint(
                        surfaceLow,
                        Qt.rgba(primaryContainer.r, primaryContainer.g, primaryContainer.b, isPhoneSeed ? (dark ? 0.24 : 0.32) : (dark ? 0.14 : 0.18))
                    ))))
    readonly property color indicator: graphite ? surfaceContent
        : (hive ? primaryContainer : (fluent ? subtleFillSecondary : secondaryContainer))
    readonly property color indicatorContent: graphite ? surface
        : (hive ? primaryContainerContent : (fluent ? surfaceContent : secondaryContainerContent))

    // ---- Fluent (WinUI 3) fills and strokes ----
    // Subtle fills: hover, selection and press on borderless things (nav items, list rows).
    readonly property color subtleFillSecondary: dark ? Qt.rgba(1, 1, 1, 0.061) : Qt.rgba(0, 0, 0, 0.037)
    readonly property color subtleFillTertiary: dark ? Qt.rgba(1, 1, 1, 0.042) : Qt.rgba(0, 0, 0, 0.024)
    // Control fills: rest, hover, pressed, and the off toggle's knob track.
    readonly property color controlFill: dark ? Qt.rgba(1, 1, 1, 0.061) : Qt.rgba(1, 1, 1, 0.70)
    readonly property color controlFillHover: dark ? Qt.rgba(1, 1, 1, 0.084) : Qt.rgba(0.976, 0.976, 0.976, 0.50)
    readonly property color controlFillPressed: dark ? Qt.rgba(1, 1, 1, 0.033) : Qt.rgba(0.976, 0.976, 0.976, 0.30)
    readonly property color controlAltFill: dark ? Qt.rgba(0, 0, 0, 0.10) : Qt.rgba(0, 0, 0, 0.024)
    // Control strokes: the hairline, and the darker edge at the bottom of a raised control.
    readonly property color controlStroke: dark ? Qt.rgba(1, 1, 1, 0.07) : Qt.rgba(0, 0, 0, 0.059)
    readonly property color controlStrokeBottom: dark ? Qt.rgba(1, 1, 1, 0.035) : Qt.rgba(0, 0, 0, 0.16)
    // The strong stroke and knob of an off toggle or an unchecked box.
    readonly property color controlStrong: dark ? Qt.rgba(1, 1, 1, 0.545) : Qt.rgba(0, 0, 0, 0.447)
    // The page layer over Mica, with its stroke.
    readonly property color layerFill: dark ? Qt.rgba(0.227, 0.227, 0.227, 0.30) : Qt.rgba(1, 1, 1, 0.50)
    readonly property color layerStroke: dark ? Qt.rgba(0, 0, 0, 0.10) : Qt.rgba(0, 0, 0, 0.059)
    // Hive: the honey halo around a primary control on hover.
    readonly property color glow: Qt.rgba(primary.r, primary.g, primary.b, dark ? 0.30 : 0.22)
    readonly property color buttonShadow: graphite ? palette.buttonShadow : "transparent"
    readonly property color scrim: Qt.rgba(0, 0, 0, dark ? 0.5 : 0.32)

    // ---- Shape ----
    readonly property var shape: graphite ? Tokens.data.themes.graphite.shape
        : (hive ? Tokens.data.themes.hive.shape
            : (fluent ? Tokens.data.themes.fluent.shape : Tokens.data.themes.bloom.shape))
    readonly property int radiusXs: shape.xs
    readonly property int radiusSm: shape.sm
    readonly property int radiusMd: shape.md
    readonly property int radiusLg: shape.lg
    readonly property int radiusXl: shape.xl
    // "Pill": half the height of whatever uses it, in Bloom. Graphite and
    // Fluent use their own small corner instead.
    function pill(height) { return graphite || fluent ? shape.full : height / 2 }

    // ---- Type ----
    // Bundled with the app (assets/fonts).
    // Fluent uses Windows' own Segoe UI Variable where it's installed.
    function firstFont(names, fallback) {
        const have = Qt.fontFamilies()
        for (let i = 0; i < names.length; i++) {
            if (have.indexOf(names[i]) >= 0)
                return names[i]
        }
        return fallback
    }
    readonly property string fluentText: firstFont(["Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI"], "Figtree")
    readonly property string fluentDisplay: firstFont(["Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI"], "Figtree")
    readonly property string fontUi: fluent ? fluentText : "Figtree"
    readonly property string fontDisplay: graphite ? "Instrument Serif" : (fluent ? fluentDisplay : "Figtree")
    readonly property string fontMono: "Space Mono"
    // Caption button glyphs: Segoe Fluent Icons ships with Windows 11 only;
    // Windows 10's Segoe MDL2 Assets has the same code points.
    readonly property string captionIconFont: Qt.fontFamilies().indexOf("Segoe Fluent Icons") >= 0
        ? "Segoe Fluent Icons" : "Segoe MDL2 Assets"
    readonly property var typography: Tokens.data.typography
    readonly property var labelStyle: graphite ? Tokens.data.themes.graphite.label
        : (hive ? Tokens.data.themes.hive.label
            : (fluent ? Tokens.data.themes.fluent.label : Tokens.data.themes.bloom.label))

    // ---- Layout and spacing ----
    readonly property var layout: Tokens.data.layout.desktop
    readonly property int railWidth: layout.railWidth
    readonly property int topBarHeight: layout.topBarHeight
    readonly property int contentPadding: layout.contentPadding
    readonly property int gutter: layout.gutter
    function space(step) { return Tokens.data.spacing.scale[step] }

    // Focus rings show for keyboard focus only, not after a click (TapHandler
    // doesn't take activeFocus on click; activeFocus on a control comes from Tab).
    function focusVisible(item) {
        return item !== null && item !== undefined && item.activeFocus
            && (item.focusReason === undefined
                || item.focusReason === Qt.TabFocusReason
                || item.focusReason === Qt.BacktabFocusReason)
    }

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
    // Fluent controls don't move or shrink: they change fill.
    readonly property real hoverLift: reduceMotion || fluent ? 0 : Tokens.data.motion.hoverLift
    readonly property real pressScale: reduceMotion || fluent ? 1 : Tokens.data.motion.pressScale
    // Windows uses the arrow over controls; the hand is for links.
    readonly property int controlCursor: fluent ? Qt.ArrowCursor : Qt.PointingHandCursor
}
