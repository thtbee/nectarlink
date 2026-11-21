// SPDX-License-Identifier: MPL-2.0
//! Crashes already fixed: `.github/known-crashes.json` on `main`, a list of
//! `{ "sig": "<12 hex>", "issue": 482, "fixed_in": "0.1.7" }`. The crash
//! card checks its crash against it and, when a newer version fixes it,
//! offers the update instead of a report. Only the version is read from
//! the file, and only when it's plain digits: no text from it is shown.

use serde::Deserialize;

/// Where the list is.
pub const KNOWN_URL: &str =
    "https://raw.githubusercontent.com/thtbee/nectarlink/main/.github/known-crashes.json";
/// The list's size limit.
pub const MAX_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
struct Entry {
    sig: String,
    #[serde(default)]
    fixed_in: Option<String>,
}

/// The version that fixes crash `sig`, when the list names one newer than
/// `current`.
pub fn fixed_in(list: &[u8], sig: &str, current: &str) -> Option<String> {
    if list.len() > MAX_BYTES {
        return None;
    }
    let entries: Vec<Entry> = serde_json::from_slice(list).ok()?;
    let fixed = entries.into_iter().find(|e| e.sig == sig)?.fixed_in?;
    let wanted = parse(&fixed)?;
    if fixed.contains('-') {
        return None;
    }
    let (have, pre) = match current.split_once('-') {
        Some((core, _)) => (parse(core)?, true),
        None => (parse(current)?, false),
    };
    // A nightly of the fixed version comes before it, so it doesn't have the fix for sure.
    (wanted > have || (wanted == have && pre)).then_some(fixed)
}

fn parse(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let mut next = || parts.next()?.parse::<u32>().ok();
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &[u8] =
        br#"[{"sig":"b3af6c6e31db","issue":482,"fixed_in":"0.1.7"},{"sig":"aaaaaaaaaaaa","issue":9},{"sig":"cccccccccccc","fixed_in":"<b>soon</b>"}]"#;

    #[test]
    fn a_newer_fix_is_offered() {
        assert_eq!(fixed_in(LIST, "b3af6c6e31db", "0.1.6"), Some("0.1.7".into()));
        assert_eq!(fixed_in(LIST, "b3af6c6e31db", "0.1.7-nightly.2"), Some("0.1.7".into()));
        assert_eq!(fixed_in(LIST, "b3af6c6e31db", "0.1.7"), None);
        assert_eq!(fixed_in(LIST, "b3af6c6e31db", "0.2.0"), None);
    }

    #[test]
    fn anything_else_is_nothing() {
        assert_eq!(fixed_in(LIST, "aaaaaaaaaaaa", "0.1.6"), None);
        assert_eq!(fixed_in(LIST, "cccccccccccc", "0.1.6"), None);
        assert_eq!(fixed_in(LIST, "dddddddddddd", "0.1.6"), None);
        assert_eq!(fixed_in(b"not json", "b3af6c6e31db", "0.1.6"), None);
        assert_eq!(fixed_in(&[b' '; MAX_BYTES + 1], "b3af6c6e31db", "0.1.6"), None);
    }
}
