// SPDX-License-Identifier: GPL-3.0-or-later
//! Repository automation. Run as `cargo xtask <task>`.
//!
//! - `tokens`: generates the desktop theme (`desktop/app/qml/Tokens.qml`)
//!   from `docs/design/tokens.json`, the single source of truth.
//! - `tokens --check`: fails if the generated files are out of date (CI).

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

const TOKENS: &str = "docs/design/tokens.json";
const QML_OUT: &str = "desktop/app/qml/Tokens.qml";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["tokens"] => tokens(false),
        ["tokens", "--check"] => tokens(true),
        _ => {
            eprintln!("usage: cargo xtask tokens [--check]");
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
    let qml = render_qml(&tokens)?;

    let out = root.join(QML_OUT);
    if check {
        let current = fs::read_to_string(&out).unwrap_or_default();
        if current != qml {
            bail!("{QML_OUT} is out of date; run `cargo xtask tokens`");
        }
        println!("{QML_OUT} is up to date");
    } else {
        fs::write(&out, qml).with_context(|| format!("writing {QML_OUT}"))?;
        println!("wrote {QML_OUT}");
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
    let seeds = themes.pointer("/bloom/seeds").and_then(Value::as_object).context("missing bloom seeds")?;
    for (seed, def) in seeds {
        for mode in ["light", "dark"] {
            check_palette(def.get(mode), &format!("bloom.{seed}.{mode}"))?;
        }
    }
    let variants = themes
        .pointer("/graphite/variants")
        .and_then(Value::as_object)
        .context("missing graphite variants")?;
    for (variant, palette) in variants {
        check_palette(Some(palette), &format!("graphite.{variant}"))?;
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
    fn documentation_keys_are_dropped() {
        let v: Value = serde_json::json!({"$note": 1, "a": {"$x": 2, "b": [ {"$y": 3, "c": 4} ]}});
        assert_eq!(strip_docs(&v), serde_json::json!({"a": {"b": [{"c": 4}]}}));
    }
}
