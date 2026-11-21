// SPDX-License-Identifier: GPL-3.0-or-later
//! The links the app opens live in `qml/Links.qml` and nowhere else (A3):
//! these tests pin them, and keep translations and other QML from bringing
//! an address of their own, which a translation PR could otherwise repoint.

use std::path::Path;

const LINKS: &str = include_str!("../qml/Links.qml");

fn dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_links_are_pinned() {
    for (key, url) in [
        ("discord", "https://discord.gg/AA2Dp8PYqf"),
        ("x", "https://x.com/thtbee_"),
        ("donate", "https://buymeacoffee.com/thtbee_"),
        ("repo", "https://github.com/thtbee/nectarlink"),
        ("releases", "https://github.com/thtbee/nectarlink/releases"),
    ] {
        let line = format!("readonly property string {key}: \"{url}\"");
        assert!(LINKS.contains(&line), "{key} must be {url}");
    }
}

/// Words that make a link, in a translation: each must be in its source too.
const LINKY: &[&str] = &["://", "www.", "href=", "discord.gg", "mailto:"];

#[test]
fn translations_bring_no_links_of_their_own() {
    let i18n = dir().join("i18n");
    let mut checked = 0;
    for entry in std::fs::read_dir(&i18n).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ts") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for message in text.split("<message").skip(1) {
            let between = |open: &str, close: &str| {
                message.split_once(open).and_then(|(_, rest)| rest.split_once(close)).map(|(inner, _)| inner)
            };
            let source = between("<source>", "</source>").unwrap_or_default();
            let Some(translation) = between("<translation", "</translation>") else { continue };
            let translation = translation.split_once('>').map_or("", |(_, t)| t);
            for word in LINKY {
                assert!(
                    translation.matches(word).count() <= source.matches(word).count(),
                    "{}: a translation adds {word:?}: {translation}",
                    path.display()
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 1000, "read the translations ({checked})");
}

#[test]
fn only_links_qml_opens_the_projects_addresses() {
    fn walk(dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "qml") && !path.ends_with("Links.qml") {
                let text = std::fs::read_to_string(&path).unwrap();
                if text.contains("openUrlExternally(\"http") || text.contains("openUrlExternally('http") {
                    found.push(path.display().to_string());
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(&dir().join("qml"), &mut found);
    assert!(found.is_empty(), "open these through Links.open: {found:?}");
}
