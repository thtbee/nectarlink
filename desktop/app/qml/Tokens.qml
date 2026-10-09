// SPDX-License-Identifier: GPL-3.0-or-later
// Generated from docs/design/tokens.json by `cargo xtask tokens`. Do not edit.
pragma Singleton
import QtQuick

QtObject {
    readonly property var data: ({
      "elevation": {
        "bloom": {
          "level0": "none",
          "level1": "0 1px 2px rgba(0,0,0,.06)",
          "window": "0 1px 2px rgba(0,0,0,.06), 0 24px 60px -20px rgba(0,0,0,.18)"
        },
        "graphite": {
          "level0": "none",
          "level1": "none",
          "window": "0 30px 70px -30px rgba(0,0,0,.35)"
        }
      },
      "icons": {
        "set": "Material Symbols Rounded (Apache-2.0)",
        "size": 20,
        "stroke": "outlined, 400 weight; filled when selected"
      },
      "layout": {
        "desktop": {
          "contentPadding": 22,
          "gutter": 16,
          "minWindow": [
            880,
            600
          ],
          "railWidth": 80,
          "topBarHeight": 52
        },
        "phone": {
          "gutter": 10,
          "screenPadding": 16,
          "tabBarHeight": 64
        }
      },
      "motion": {
        "description": "Springs everywhere; durations are only for fades. Respect reduced-motion: replace movement with 120 ms fades.",
        "fade": {
          "fast": 120,
          "normal": 200
        },
        "hoverLift": 2,
        "pressScale": 0.97,
        "spring": {
          "gentle": {
            "damping": 26,
            "stiffness": 220,
            "use": "page and panel transitions"
          },
          "snappy": {
            "damping": 34,
            "stiffness": 620,
            "use": "toggles, chips, buttons, hover lift"
          },
          "standard": {
            "damping": 30,
            "stiffness": 380,
            "use": "cards, sheets, expanding a notification into a conversation"
          }
        }
      },
      "spacing": {
        "scale": [
          0,
          2,
          4,
          6,
          8,
          10,
          12,
          16,
          20,
          24,
          32,
          40,
          48,
          64
        ],
        "unit": 4
      },
      "themes": {
        "bloom": {
          "defaultSeed": "honey",
          "description": "Default. Minimal Material You: neutral surfaces, color only where it carries meaning.",
          "dynamicColor": true,
          "fonts": {
            "display": "Figtree",
            "label": "Figtree",
            "mono": "Space Mono",
            "ui": "Figtree"
          },
          "label": {
            "case": "none",
            "size": 12.5,
            "tracking": 0,
            "weight": 600
          },
          "seeds": {
            "honey": {
              "dark": {
                "onPrimary": "#4A2800",
                "onPrimaryContainer": "#FFDDB8",
                "onSecondaryContainer": "#F3E0CB",
                "onSurface": "#EBE1D9",
                "onSurfaceVariant": "#D5C3B5",
                "outline": "#9E8E81",
                "outlineVariant": "#51453A",
                "primary": "#FFB95F",
                "primaryContainer": "#693C00",
                "secondaryContainer": "#574330",
                "surface": "#17130E",
                "surfaceContainer": "#241F1A",
                "surfaceContainerHigh": "#2E2924",
                "surfaceContainerHighest": "#39342E",
                "surfaceContainerLow": "#120E0A"
              },
              "light": {
                "onPrimary": "#FFFFFF",
                "onPrimaryContainer": "#2C1600",
                "onSecondaryContainer": "#261A0C",
                "onSurface": "#1F1B16",
                "onSurfaceVariant": "#51453A",
                "outline": "#837468",
                "outlineVariant": "#D5C3B5",
                "primary": "#8A5100",
                "primaryContainer": "#FFDDB8",
                "secondaryContainer": "#F3E0CB",
                "surface": "#FFF8F3",
                "surfaceContainer": "#F5ECE4",
                "surfaceContainerHigh": "#EFE6DE",
                "surfaceContainerHighest": "#E9E1D9",
                "surfaceContainerLow": "#FBF2EB"
              },
              "seed": "#B7791F"
            },
            "lavender": {
              "dark": {
                "onPrimary": "#36275D",
                "onPrimaryContainer": "#E9DDFF",
                "onSecondaryContainer": "#E8DEF8",
                "onSurface": "#E6E0E9",
                "onSurfaceVariant": "#CAC4CF",
                "outline": "#948F99",
                "outlineVariant": "#49454E",
                "primary": "#CFBDFE",
                "primaryContainer": "#4D3D75",
                "secondaryContainer": "#4A4458",
                "surface": "#141218",
                "surfaceContainer": "#211F24",
                "surfaceContainerHigh": "#2B292F",
                "surfaceContainerHighest": "#36343A",
                "surfaceContainerLow": "#0F0D13"
              },
              "light": {
                "onPrimary": "#FFFFFF",
                "onPrimaryContainer": "#201047",
                "onSecondaryContainer": "#1D192B",
                "onSurface": "#1D1B20",
                "onSurfaceVariant": "#49454E",
                "outline": "#7A757F",
                "outlineVariant": "#CAC4CF",
                "primary": "#65558F",
                "primaryContainer": "#E9DDFF",
                "secondaryContainer": "#E8DEF8",
                "surface": "#FDF7FF",
                "surfaceContainer": "#F2ECF4",
                "surfaceContainerHigh": "#ECE6EE",
                "surfaceContainerHighest": "#E6E0E9",
                "surfaceContainerLow": "#F7F2FA"
              },
              "seed": "#7E6AA8"
            },
            "ocean": {
              "dark": {
                "onPrimary": "#003354",
                "onPrimaryContainer": "#CEE5FF",
                "onSecondaryContainer": "#D5E4F7",
                "onSurface": "#E0E2E8",
                "onSurfaceVariant": "#C2C7CF",
                "outline": "#8C9199",
                "outlineVariant": "#42474E",
                "primary": "#9BCBFB",
                "primaryContainer": "#0F4A73",
                "secondaryContainer": "#3B4858",
                "surface": "#101418",
                "surfaceContainer": "#1C2024",
                "surfaceContainerHigh": "#262A2F",
                "surfaceContainerHighest": "#31353A",
                "surfaceContainerLow": "#0B0F13"
              },
              "light": {
                "onPrimary": "#FFFFFF",
                "onPrimaryContainer": "#001D33",
                "onSecondaryContainer": "#0E1D2A",
                "onSurface": "#181C20",
                "onSurfaceVariant": "#42474E",
                "outline": "#72777F",
                "outlineVariant": "#C2C7CF",
                "primary": "#2F628C",
                "primaryContainer": "#CEE5FF",
                "secondaryContainer": "#D5E4F7",
                "surface": "#F7F9FF",
                "surfaceContainer": "#ECEEF4",
                "surfaceContainerHigh": "#E6E8EE",
                "surfaceContainerHighest": "#E0E2E8",
                "surfaceContainerLow": "#F1F4FA"
              },
              "seed": "#3D7DA8"
            },
            "sage": {
              "dark": {
                "onPrimary": "#0A390F",
                "onPrimaryContainer": "#BCF0B4",
                "onSecondaryContainer": "#D6E8CF",
                "onSurface": "#E0E4DA",
                "onSurfaceVariant": "#C2C8BD",
                "outline": "#8C9388",
                "outlineVariant": "#424940",
                "primary": "#A1D39A",
                "primaryContainer": "#234F24",
                "secondaryContainer": "#3B4B38",
                "surface": "#10140F",
                "surfaceContainer": "#1D211B",
                "surfaceContainerHigh": "#272B25",
                "surfaceContainerHighest": "#32362F",
                "surfaceContainerLow": "#0C100B"
              },
              "light": {
                "onPrimary": "#FFFFFF",
                "onPrimaryContainer": "#002204",
                "onSecondaryContainer": "#111F0F",
                "onSurface": "#191D17",
                "onSurfaceVariant": "#424940",
                "outline": "#72796F",
                "outlineVariant": "#C2C8BD",
                "primary": "#3B6939",
                "primaryContainer": "#BCF0B4",
                "secondaryContainer": "#D6E8CF",
                "surface": "#F7FBF1",
                "surfaceContainer": "#EBEFE5",
                "surfaceContainerHigh": "#E6E9E0",
                "surfaceContainerHighest": "#E0E4DA",
                "surfaceContainerLow": "#F1F5EB"
              },
              "seed": "#5B8C5A"
            }
          },
          "shape": {
            "full": 999,
            "lg": 20,
            "md": 16,
            "sm": 12,
            "xl": 28,
            "xs": 8
          },
          "status": {
            "dark": {
              "error": "#FFB4AB",
              "onError": "#690005",
              "success": "{primary}",
              "warning": "#F2C14E"
            },
            "light": {
              "error": "#BA1A1A",
              "onError": "#FFFFFF",
              "success": "{primary}",
              "warning": "#7D5200"
            }
          }
        },
        "graphite": {
          "description": "Built-in alternative, from github.com/thtbee/graphite. Monochrome ink on paper / chalk on slate; dynamic color off.",
          "dynamicColor": false,
          "fonts": {
            "display": "Instrument Serif",
            "label": "Space Mono",
            "mono": "Space Mono",
            "note": "Caveat",
            "ui": "Figtree"
          },
          "label": {
            "case": "upper",
            "size": 10.5,
            "tracking": 0.12,
            "weight": 400
          },
          "rules": [
            "Cards use 1px outlineVariant borders, no fills, no elevation.",
            "Primary buttons are ink-filled with the flat offset buttonShadow.",
            "Crop marks frame hero imagery; 'FIG. NN // ...' annotations label sections.",
            "At most one handwritten (Caveat) margin note per screen, only for delight moments.",
            "Never use Graphite's art effects in the app: no smudge physics, no gritty lettering, no scroll sound.",
            "Success and completed states use ink (Paper) or chalk (Slate), never green."
          ],
          "shape": {
            "full": 3,
            "lg": 4,
            "md": 4,
            "sm": 3,
            "xl": 6,
            "xs": 2
          },
          "variants": {
            "paper": {
              "buttonShadow": "3px 3px 0 #D5CFC2",
              "error": "#9E2B25",
              "grain": 0.22,
              "grid": "rgba(74,71,67,0.06)",
              "onError": "#F5F2EB",
              "onPrimary": "#F5F2EB",
              "onPrimaryContainer": "#1A1918",
              "onSecondaryContainer": "#1A1918",
              "onSurface": "#1A1918",
              "onSurfaceVariant": "#5E5A54",
              "outline": "#4A4743",
              "outlineVariant": "#D5CFC2",
              "primary": "#1A1918",
              "primaryContainer": "#E8E3D7",
              "secondaryContainer": "#E8E3D7",
              "success": "#1A1918",
              "surface": "#F5F2EB",
              "surfaceContainer": "#F5F2EB",
              "surfaceContainerHigh": "#EFEBE2",
              "surfaceContainerHighest": "#E8E3D7",
              "surfaceContainerLow": "#F1EDE4",
              "warning": "#705516"
            },
            "slate": {
              "buttonShadow": "3px 3px 0 #26272A",
              "error": "#F0A39C",
              "grain": 0.12,
              "grid": "rgba(200,208,216,0.035)",
              "onError": "#0D0D0E",
              "onPrimary": "#0D0D0E",
              "onPrimaryContainer": "#E8EAED",
              "onSecondaryContainer": "#E8EAED",
              "onSurface": "#E8EAED",
              "onSurfaceVariant": "#9A9CA1",
              "outline": "#7E8084",
              "outlineVariant": "#26272A",
              "primary": "#E8EAED",
              "primaryContainer": "#1A1A1D",
              "secondaryContainer": "#1A1A1D",
              "success": "#E8EAED",
              "surface": "#0D0D0E",
              "surfaceContainer": "#0D0D0E",
              "surfaceContainerHigh": "#141416",
              "surfaceContainerHighest": "#1A1A1D",
              "surfaceContainerLow": "#0B0B0C",
              "warning": "#E2C47A"
            }
          }
        }
      },
      "typography": {
        "body": {
          "lineHeight": 1.5,
          "size": 14,
          "tracking": 0,
          "weight": 400
        },
        "bodySmall": {
          "lineHeight": 1.45,
          "size": 12.5,
          "tracking": 0,
          "weight": 400
        },
        "caption": {
          "lineHeight": 1.3,
          "size": 11,
          "tracking": 0,
          "weight": 500
        },
        "code": {
          "font": "mono",
          "lineHeight": 1.3,
          "size": 13,
          "tracking": 0.14,
          "weight": 700
        },
        "displayLarge": {
          "lineHeight": 1.05,
          "size": 44,
          "tracking": -0.025,
          "weight": 650
        },
        "displaySmall": {
          "lineHeight": 1.1,
          "size": 30,
          "tracking": -0.02,
          "weight": 650
        },
        "headline": {
          "lineHeight": 1.2,
          "size": 22,
          "tracking": -0.015,
          "weight": 650
        },
        "label": {
          "lineHeight": 1.3,
          "size": 12.5,
          "tracking": 0,
          "weight": 600
        },
        "title": {
          "lineHeight": 1.35,
          "size": 16,
          "tracking": 0,
          "weight": 600
        }
      }
    })
}
