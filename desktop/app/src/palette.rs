// SPDX-License-Identifier: GPL-3.0-or-later
//! Bloom's colors from a seed color, built like Material You: the most
//! suitable color in the wallpaper becomes the seed, and the seed spans
//! tonal palettes that every color role takes a tone from, in a light and a
//! dark version. The presets in docs/design/tokens.json are built the same
//! way, so a wallpaper theme looks like one of them.

use material_colors::{
    color::Rgb,
    hct::Hct,
    palette::TonalPalette,
    quantize::{Quantizer, QuantizerCelebi},
    score::Score,
};

/// How many colors an image is reduced to before picking a seed (Android's
/// value).
const QUANTIZE_COLORS: usize = 128;

/// The best seed color in an image, or `None` when it has no color worth
/// building a theme on (black and white, grey, near-empty).
pub fn seed_from_pixels(pixels: &[Rgb]) -> Option<Rgb> {
    if pixels.is_empty() {
        return None;
    }
    let quantized = QuantizerCelebi::quantize(pixels, QUANTIZE_COLORS);
    // A sentinel fallback tells "nothing suitable" apart from a real pick:
    // pure black has no chroma, so scoring can never choose it.
    let sentinel = Rgb::new(0, 0, 0);
    let ranked = Score::score(&quantized.color_to_count, Some(1), Some(sentinel), Some(true));
    ranked.first().copied().filter(|&c| c != sentinel)
}

#[derive(Clone, Copy)]
enum Key {
    Primary,
    Secondary,
    Neutral,
    NeutralVariant,
}

/// Bloom's color roles (names as in docs/design/tokens.json) and the tone
/// each takes from its palette, in light and dark: the Material 3 baseline
/// tones, except that Bloom's low surface sits at the bottom in dark mode,
/// so the navigation rail recedes.
const ROLES: [(&str, Key, i32, i32); 15] = [
    ("primary", Key::Primary, 40, 80),
    ("onPrimary", Key::Primary, 100, 20),
    ("primaryContainer", Key::Primary, 90, 30),
    ("onPrimaryContainer", Key::Primary, 10, 90),
    ("secondaryContainer", Key::Secondary, 90, 30),
    ("onSecondaryContainer", Key::Secondary, 10, 90),
    ("surface", Key::Neutral, 98, 6),
    ("surfaceContainerLow", Key::Neutral, 96, 4),
    ("surfaceContainer", Key::Neutral, 94, 12),
    ("surfaceContainerHigh", Key::Neutral, 92, 17),
    ("surfaceContainerHighest", Key::Neutral, 90, 22),
    ("onSurface", Key::Neutral, 10, 90),
    ("onSurfaceVariant", Key::NeutralVariant, 30, 80),
    ("outline", Key::NeutralVariant, 50, 60),
    ("outlineVariant", Key::NeutralVariant, 80, 30),
];

/// One Bloom palette: role name to "#RRGGBB".
pub type Palette = serde_json::Map<String, serde_json::Value>;

/// The palettes a seed spans, with Android's Tonal Spot chromas: calm
/// accents in the seed's hue, and surfaces and text barely tinted by it. A
/// loud wallpaper never makes a loud app.
fn tonal_palettes(seed: Rgb) -> [TonalPalette; 4] {
    let hue = Hct::new(seed).get_hue();
    [
        TonalPalette::of(hue, 36.0),
        TonalPalette::of(hue, 16.0),
        TonalPalette::of(hue, 6.0),
        TonalPalette::of(hue, 8.0),
    ]
}

fn palette(palettes: &[TonalPalette; 4], dark: bool) -> Palette {
    ROLES
        .iter()
        .map(|&(name, key, light_tone, dark_tone)| {
            let color = palettes[key as usize].tone(if dark { dark_tone } else { light_tone });
            (name.to_owned(), format!("#{:06X}", color.as_u32()).into())
        })
        .collect()
}

/// `{ "seed": "#RRGGBB", "light": {...}, "dark": {...} }`, the shape of a
/// seed in docs/design/tokens.json.
pub fn scheme_json(seed: Rgb) -> serde_json::Value {
    let palettes = tonal_palettes(seed);
    serde_json::json!({
        "seed": format!("#{:06X}", seed.as_u32()),
        "light": palette(&palettes, false),
        "dark": palette(&palettes, true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: &str) -> Rgb {
        Rgb::from_u32(u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap())
    }

    /// Hue distance in degrees.
    fn hue_distance(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    /// The presets were tuned by hand, so they aren't reproduced to the
    /// last digit. What must match is their structure: every role at the
    /// same tone (so contrast is the same), in the seed's hue.
    #[test]
    fn has_the_structure_of_the_token_presets() {
        let tokens: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/design/tokens.json")).unwrap();
        let seeds = tokens["themes"]["bloom"]["seeds"].as_object().unwrap();
        assert!(!seeds.is_empty());
        let mut mismatches = Vec::new();
        for (name, preset) in seeds {
            let built = scheme_json(hex(preset["seed"].as_str().unwrap()));
            for mode in ["light", "dark"] {
                let roles = preset[mode].as_object().unwrap();
                assert_eq!(roles.len(), ROLES.len(), "{name} {mode}: roles changed in the tokens");
                for (role, value) in roles {
                    let ours = Hct::new(hex(built[mode][role].as_str().unwrap()));
                    let theirs = Hct::new(hex(value.as_str().unwrap()));
                    let tone = (ours.get_tone() - theirs.get_tone()).abs();
                    let chromatic = ours.get_chroma() > 10.0 && theirs.get_chroma() > 10.0;
                    let hue = if chromatic { hue_distance(ours.get_hue(), theirs.get_hue()) } else { 0.0 };
                    if tone > 2.0 || hue > 15.0 {
                        mismatches
                            .push(format!("{name} {mode} {role}: tone off by {tone:.1}, hue by {hue:.0}"));
                    }
                }
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[test]
    fn json_has_every_role_in_both_modes() {
        let scheme = scheme_json(Rgb::new(0x3D, 0x7D, 0xA8));
        assert_eq!(scheme["seed"], "#3D7DA8");
        for mode in ["light", "dark"] {
            for (role, ..) in ROLES {
                let value = scheme[mode][role].as_str().unwrap();
                assert!(value.len() == 7 && value.starts_with('#'), "{mode} {role}: {value}");
            }
        }
    }

    #[test]
    fn picks_the_dominant_color_of_an_image() {
        // Mostly a deep blue, some orange: the seed is the blue.
        let mut pixels = vec![Rgb::new(0x1E, 0x4F, 0xA8); 900];
        pixels.extend(vec![Rgb::new(0xE8, 0x8A, 0x2A); 100]);
        let seed = seed_from_pixels(&pixels).unwrap();
        let hue = Hct::new(seed).get_hue();
        assert!((240.0..290.0).contains(&hue), "hue {hue}");
    }

    #[test]
    fn grey_images_have_no_seed() {
        let mut pixels = vec![Rgb::new(0x80, 0x80, 0x80); 500];
        pixels.extend(vec![Rgb::new(0x20, 0x20, 0x20); 500]);
        assert_eq!(seed_from_pixels(&pixels), None);
        assert_eq!(seed_from_pixels(&[]), None);
    }

    #[test]
    fn extreme_phone_seeds_keep_calm_surfaces_and_wcag_contrast() {
        fn rel_lum(c: Rgb) -> f64 {
            let chan = |v: u8| {
                let s = f64::from(v) / 255.0;
                if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * chan(c.red) + 0.7152 * chan(c.green) + 0.0722 * chan(c.blue)
        }
        fn contrast(a: Rgb, b: Rgb) -> f64 {
            let (la, lb) = (rel_lum(a), rel_lum(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        }

        let extremes = [
            ("pure_red", Rgb::new(0xFF, 0x00, 0x00)),
            ("pure_green", Rgb::new(0x00, 0xFF, 0x00)),
            ("pure_blue", Rgb::new(0x00, 0x00, 0xFF)),
            ("near_grey", Rgb::new(0x7E, 0x80, 0x82)),
            ("saturated_yellow", Rgb::new(0xFF, 0xEE, 0x00)),
        ];

        for (label, seed) in extremes {
            let scheme = scheme_json(seed);
            for mode in ["light", "dark"] {
                let p = &scheme[mode];
                let pairs = [
                    ("onSurface", "surface", 7.0),
                    ("onSurface", "surfaceContainer", 7.0),
                    ("onSurfaceVariant", "surface", 4.5),
                    ("onSurfaceVariant", "surfaceContainer", 4.5),
                    ("onPrimary", "primary", 4.5),
                    ("onPrimaryContainer", "primaryContainer", 7.0),
                    ("onSecondaryContainer", "secondaryContainer", 7.0),
                ];
                for (fg_role, bg_role, min_ratio) in pairs {
                    let fg = hex(p[fg_role].as_str().unwrap());
                    let bg = hex(p[bg_role].as_str().unwrap());
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= min_ratio,
                        "{label} {mode} {fg_role}/{bg_role}: contrast {ratio:.2} < {min_ratio}"
                    );
                }
            }
        }
    }
}
