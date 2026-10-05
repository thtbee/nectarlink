pragma Singleton
import QtQuick

// Bloom Light, Honey seed (docs/design/tokens.json). The real app generates
// this from the tokens file and swaps palettes for dark mode and other seeds.
QtObject {
    readonly property bool dark: false

    readonly property color primary: "#8A5100"
    readonly property color primaryContent: "#FFFFFF"
    readonly property color primaryContainer: "#FFDDB8"
    readonly property color primaryContainerContent: "#2C1600"
    readonly property color secondaryContainer: "#F3E0CB"
    readonly property color secondaryContainerContent: "#261A0C"
    readonly property color surface: "#FFF8F3"
    readonly property color surfaceLow: "#FBF2EB"
    readonly property color surfaceContainer: "#F5ECE4"
    readonly property color surfaceContainerHigh: "#EFE6DE"
    readonly property color surfaceContainerHighest: "#E9E1D9"
    readonly property color surfaceContent: "#1F1B16"
    readonly property color surfaceContentVariant: "#51453A"
    readonly property color outline: "#837468"
    readonly property color outlineVariant: "#D5C3B5"
    readonly property color hero: Qt.tint(surface, Qt.rgba(1.0, 0.867, 0.722, 0.62))

    readonly property int radiusXl: 28
    readonly property int radiusLg: 20
    readonly property int radiusMd: 16
    readonly property int radiusSm: 12

    readonly property string font: "Segoe UI Variable Text"
    readonly property string displayFont: "Segoe UI Variable Display"

    // Motion: springs from tokens.json (gentle / standard / snappy).
    readonly property real springGentle: 2.2
    readonly property real springStandard: 3.8
    readonly property real dampingStandard: 0.3
}
