// SPDX-License-Identifier: GPL-3.0-or-later
//! Repository automation. Run as `cargo xtask <task>`.
//!
//! - `tokens`: generates the desktop and Android themes
//!   (`desktop/app/qml/Tokens.qml`, `android/.../ui/theme/Tokens.kt`) from
//!   `docs/design/tokens.json`, the single source of truth.
//! - `tokens --check`: fails if the generated files are out of date (CI).
//! - `notices <out>`: the third-party notices shipped with the Windows app.

mod notices;

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

const TOKENS: &str = "docs/design/tokens.json";
const QML_OUT: &str = "desktop/app/qml/Tokens.qml";
const KOTLIN_OUT: &str = "android/app/src/main/kotlin/app/nectarlink/android/ui/theme/Tokens.kt";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["tokens"] => tokens(false),
        ["tokens", "--check"] => tokens(true),
        ["notices", rest @ ..] => notices::run(rest),
        _ => {
            eprintln!("usage: cargo xtask tokens [--check] | notices <out> [--target <triple>]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    // tools/xtask → repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("xtask lives two levels deep").to_owned()
}

fn tokens(check: bool) -> Result<()> {
    let root = repo_root();
    let source = fs::read_to_string(root.join(TOKENS)).with_context(|| format!("reading {TOKENS}"))?;
    let tokens: Value = serde_json::from_str(&source).with_context(|| format!("parsing {TOKENS}"))?;
    validate(&tokens)?;
    let outputs = [(QML_OUT, render_qml(&tokens)?), (KOTLIN_OUT, render_kotlin(&tokens)?)];

    let mut stale = Vec::new();
    for (path, content) in outputs {
        let out = root.join(path);
        if check {
            if fs::read_to_string(&out).unwrap_or_default() != content {
                stale.push(path);
            }
        } else {
            if let Some(dir) = out.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&out, content).with_context(|| format!("writing {path}"))?;
            println!("wrote {path}");
        }
    }
    if !stale.is_empty() {
        bail!("out of date: {}; run `cargo xtask tokens`", stale.join(", "));
    }
    if check {
        println!("generated theme files are up to date");
    }
    Ok(())
}

/// Color roles every palette must define (Material You names).
const ROLES: &[&str] = &[
    "primary",
    "onPrimary",
    "primaryContainer",
    "onPrimaryContainer",
    "secondaryContainer",
    "onSecondaryContainer",
    "surface",
    "surfaceContainerLow",
    "surfaceContainer",
    "surfaceContainerHigh",
    "surfaceContainerHighest",
    "onSurface",
    "onSurfaceVariant",
    "outline",
    "outlineVariant",
];

/// Catches mistakes in the token file before they reach a theme.
fn validate(tokens: &Value) -> Result<()> {
    let themes = tokens.get("themes").context("missing \"themes\"")?;
    let status = themes.pointer("/bloom/status").context("missing bloom status")?;
    let seeds = themes.pointer("/bloom/seeds").and_then(Value::as_object).context("missing bloom seeds")?;
    for (seed, def) in seeds {
        for mode in ["light", "dark"] {
            let name = format!("bloom.{seed}.{mode}");
            check_palette(def.get(mode), &name)?;
            check_contrast(def.get(mode), status.get(mode), &name)?;
        }
    }
    let variants = themes
        .pointer("/graphite/variants")
        .and_then(Value::as_object)
        .context("missing graphite variants")?;
    for (variant, palette) in variants {
        let name = format!("graphite.{variant}");
        check_palette(Some(palette), &name)?;
        check_contrast(Some(palette), Some(palette), &name)?;
    }
    Ok(())
}

fn check_palette(palette: Option<&Value>, name: &str) -> Result<()> {
    let palette = palette.and_then(Value::as_object).with_context(|| format!("{name}: missing palette"))?;
    for role in ROLES {
        let color =
            palette.get(*role).and_then(Value::as_str).with_context(|| format!("{name}: missing {role}"))?;
        if !is_hex_color(color) {
            bail!("{name}.{role}: {color:?} is not a #RRGGBB color");
        }
    }
    Ok(())
}

fn parse_rgb(hex: &str) -> Result<(u8, u8, u8)> {
    if !is_hex_color(hex) {
        bail!("{hex:?} is not a #RRGGBB color");
    }
    let r = u8::from_str_radix(&hex[1..3], 16)?;
    let g = u8::from_str_radix(&hex[3..5], 16)?;
    let b = u8::from_str_radix(&hex[5..7], 16)?;
    Ok((r, g, b))
}

fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let chan = |c: u8| {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * chan(r) + 0.7152 * chan(g) + 0.0722 * chan(b)
}

fn contrast_ratio(fg: &str, bg: &str) -> Result<f64> {
    let l1 = relative_luminance(parse_rgb(fg)?);
    let l2 = relative_luminance(parse_rgb(bg)?);
    let (bright, dark) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    Ok((bright + 0.05) / (dark + 0.05))
}

/// Enforces WCAG 2.1 AA (>= 4.5:1) contrast for all text and status pairs.
fn check_contrast(palette: Option<&Value>, status: Option<&Value>, name: &str) -> Result<()> {
    let p = palette.and_then(Value::as_object).with_context(|| format!("{name}: missing palette"))?;
    let s = status.and_then(Value::as_object).with_context(|| format!("{name}: missing status"))?;
    let get_p = |k: &str| p.get(k).and_then(Value::as_str).with_context(|| format!("{name}: missing {k}"));
    let get_s = |k: &str| s.get(k).and_then(Value::as_str).with_context(|| format!("{name}: missing {k}"));

    let surfaces = [
        "surface",
        "surfaceContainerLow",
        "surfaceContainer",
        "surfaceContainerHigh",
        "surfaceContainerHighest",
    ];
    for bg_key in surfaces {
        let bg = get_p(bg_key)?;
        for fg_key in ["onSurface", "onSurfaceVariant", "primary"] {
            let fg = get_p(fg_key)?;
            let ratio = contrast_ratio(fg, bg)?;
            if ratio < 4.5 {
                bail!("{name}: {fg_key} ({fg}) on {bg_key} ({bg}) has contrast {ratio:.2}:1 (< 4.5:1)");
            }
        }
        for st_key in ["warning", "error"] {
            let fg = get_s(st_key)?;
            let ratio = contrast_ratio(fg, bg)?;
            if ratio < 4.5 {
                bail!("{name}: {st_key} ({fg}) on {bg_key} ({bg}) has contrast {ratio:.2}:1 (< 4.5:1)");
            }
        }
    }
    for (fg_key, bg_key) in [
        ("onPrimary", "primary"),
        ("onPrimaryContainer", "primaryContainer"),
        ("onSecondaryContainer", "secondaryContainer"),
    ] {
        let fg = get_p(fg_key)?;
        let bg = get_p(bg_key)?;
        let ratio = contrast_ratio(fg, bg)?;
        if ratio < 4.5 {
            bail!("{name}: {fg_key} ({fg}) on {bg_key} ({bg}) has contrast {ratio:.2}:1 (< 4.5:1)");
        }
    }
    let on_err = get_s("onError")?;
    let err = get_s("error")?;
    let err_ratio = contrast_ratio(on_err, err)?;
    if err_ratio < 4.5 {
        bail!("{name}: onError ({on_err}) on error ({err}) has contrast {err_ratio:.2}:1 (< 4.5:1)");
    }
    Ok(())
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Drops documentation keys (`$description`, `$note`, …).
fn strip_docs(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !k.starts_with('$'))
                .map(|(k, v)| (k.clone(), strip_docs(v)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_docs).collect()),
        other => other.clone(),
    }
}

fn render_qml(tokens: &Value) -> Result<String> {
    let data = serde_json::to_string_pretty(&strip_docs(tokens))?;
    let data = data.replace('\n', "\n    ");
    Ok(format!(
        "// SPDX-License-Identifier: GPL-3.0-or-later\n\
         // Generated from docs/design/tokens.json by `cargo xtask tokens`. Do not edit.\n\
         pragma Singleton\n\
         import QtQuick\n\
         \n\
         QtObject {{\n    \
         readonly property var data: ({data})\n\
         }}\n"
    ))
}

/// `#RRGGBB` as a Compose color literal.
fn kotlin_color(hex: &str) -> String {
    format!("Color(0xFF{})", hex.trim_start_matches('#').to_ascii_uppercase())
}

/// A `Palette(...)` expression, its lines indented by `indent` spaces.
fn kotlin_palette(palette: &Value, indent: usize) -> Result<String> {
    let pad = " ".repeat(indent);
    let mut lines = vec!["Palette(".to_owned()];
    for role in ROLES {
        let hex = palette.get(*role).and_then(Value::as_str).with_context(|| format!("missing {role}"))?;
        lines.push(format!("{pad}    {role} = {},", kotlin_color(hex)));
    }
    lines.push(format!("{pad})"));
    Ok(lines.join(
        "
",
    ))
}

fn render_kotlin(tokens: &Value) -> Result<String> {
    let bloom = tokens.pointer("/themes/bloom").context("missing bloom")?;
    let seeds = bloom.get("seeds").and_then(Value::as_object).context("missing seeds")?;
    let default_seed = bloom.get("defaultSeed").and_then(Value::as_str).context("missing defaultSeed")?;
    let variants = tokens.pointer("/themes/graphite/variants").context("missing graphite variants")?;
    let status = bloom.get("status").context("missing bloom status")?;
    let status_color = |mode: &str, key: &str| -> Result<String> {
        let hex = status
            .pointer(&format!("/{mode}/{key}"))
            .and_then(Value::as_str)
            .context("missing status color")?;
        Ok(kotlin_color(hex))
    };

    let mut out: Vec<String> = vec![
        "// SPDX-License-Identifier: GPL-3.0-or-later".into(),
        "// Generated from docs/design/tokens.json by `cargo xtask tokens`. Do not edit.".into(),
        "package app.nectarlink.android.ui.theme".into(),
        String::new(),
        "import androidx.compose.ui.graphics.Color".into(),
        String::new(),
        "/** Material color roles of one theme variant. */".into(),
        "data class Palette(".into(),
    ];
    out.extend(ROLES.iter().map(|r| format!("    val {r}: Color,")));
    out.extend([
        ")".into(),
        String::new(),
        "/** A Bloom color preset: its seed swatch and light and dark palettes. */".into(),
        "data class Seed(val color: Color, val light: Palette, val dark: Palette)".into(),
        String::new(),
        "object Tokens {".into(),
        format!("    const val DEFAULT_SEED = \"{default_seed}\""),
        String::new(),
        "    val bloomSeeds: Map<String, Seed> = linkedMapOf(".into(),
    ]);
    for (name, def) in seeds {
        let seed = def.get("seed").and_then(Value::as_str).context("missing seed color")?;
        out.push(format!("        \"{name}\" to Seed("));
        out.push(format!("            color = {},", kotlin_color(seed)));
        out.push(format!("            light = {},", kotlin_palette(&def["light"], 12)?));
        out.push(format!("            dark = {},", kotlin_palette(&def["dark"], 12)?));
        out.push("        ),".into());
    }
    out.push("    )".into());
    out.push(String::new());
    out.push(format!("    val graphitePaper = {}", kotlin_palette(&variants["paper"], 4)?));
    out.push(String::new());
    out.push(format!("    val graphiteSlate = {}", kotlin_palette(&variants["slate"], 4)?));
    out.push(String::new());
    out.push(format!("    val warningLight = {}", status_color("light", "warning")?));
    out.push(format!("    val warningDark = {}", status_color("dark", "warning")?));
    out.push(format!("    val errorLight = {}", status_color("light", "error")?));
    out.push(format!("    val errorDark = {}", status_color("dark", "error")?));
    out.push("}".into());
    out.push(String::new());
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_in_tokens_are_valid() {
        let source = fs::read_to_string(repo_root().join(TOKENS)).unwrap();
        validate(&serde_json::from_str(&source).unwrap()).unwrap();
    }

    #[test]
    fn rejects_bad_colors_and_missing_roles() {
        let mut palette = serde_json::Map::new();
        for role in ROLES {
            palette.insert((*role).into(), "#123456".into());
        }
        assert!(check_palette(Some(&Value::Object(palette.clone())), "ok").is_ok());
        palette.insert("primary".into(), "red".into());
        assert!(check_palette(Some(&Value::Object(palette.clone())), "bad").is_err());
        palette.remove("primary");
        assert!(check_palette(Some(&Value::Object(palette)), "missing").is_err());
    }

    #[test]
    fn kotlin_theme_has_every_seed() {
        let source = fs::read_to_string(repo_root().join(TOKENS)).unwrap();
        let tokens: Value = serde_json::from_str(&source).unwrap();
        let kotlin = render_kotlin(&tokens).unwrap();
        for seed in tokens.pointer("/themes/bloom/seeds").unwrap().as_object().unwrap().keys() {
            assert!(kotlin.contains(&format!("\"{seed}\" to Seed(")), "{seed}");
        }
        assert!(kotlin.contains("primary = Color(0xFF8A5100)"));
    }

    #[test]
    fn documentation_keys_are_dropped() {
        let v: Value = serde_json::json!({"$note": 1, "a": {"$x": 2, "b": [ {"$y": 3, "c": 4} ]}});
        assert_eq!(strip_docs(&v), serde_json::json!({"a": {"b": [{"c": 4}]}}));
    }

    #[test]
    fn rejects_low_contrast_palette() {
        let source = fs::read_to_string(repo_root().join(TOKENS)).unwrap();
        let mut tokens: Value = serde_json::from_str(&source).unwrap();
        tokens.pointer_mut("/themes/bloom/status/light").unwrap()["warning"] = "#9A6700".into();
        let err = validate(&tokens).unwrap_err().to_string();
        assert!(err.contains("warning"), "{err}");
    }
}
