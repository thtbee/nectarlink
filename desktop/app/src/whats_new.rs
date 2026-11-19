// SPDX-License-Identifier: GPL-3.0-or-later
//! What's new (#461): the release notes of this build, read from the
//! CHANGELOG.md it was built from, so they match it exactly and need no
//! network. The top section is always this build's: the next release's notes
//! on a Nightly, its own on a release.
//!
//! Each entry has an ID made from its text, and the IDs already shown are
//! saved, so after an update the sheet can lead with what's new since the
//! last look, Nightly to Nightly too, where the version's section only grows.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::Serialize;

const CHANGELOG: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"));

/// How many releases before this one the sheet offers.
const EARLIER: usize = 4;
/// How many shown entries are remembered: far more than a release has.
pub const SEEN_LIMIT: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    /// "0.1.6", with its name when it has one ("0.1.0 \"Pollen\"").
    pub version: String,
    /// "2026-11-05", or "" for the release in the making.
    pub date: String,
    /// The paragraph under the heading, as rich text.
    pub summary: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// "New", "Changed", "Fixed"...
    pub title: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: String,
    /// The bold lead, as plain text (may be empty).
    pub title: String,
    /// The rest, as Qt rich text (bold, links).
    pub body: String,
    /// Not shown before on this PC.
    #[serde(rename = "new")]
    pub unseen: bool,
}

/// What this PC has seen, in `whats-new.json`.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Seen {
    #[serde(default)]
    seen: Vec<String>,
    /// Updated since the sheet was last closed: it opens by itself.
    #[serde(default)]
    pending: bool,
}

impl Seen {
    fn path(dir: &Path) -> PathBuf {
        dir.join("whats-new.json")
    }

    fn load(dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, dir: &Path) {
        let path = Self::path(dir);
        let tmp = path.with_extension("json.tmp");
        let written = serde_json::to_vec(self)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(&tmp, json))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            tracing::warn!(error = %e, "can't save what's new was seen");
        }
    }
}

/// At start: after an update the sheet opens by itself; on a new install
/// everything counts as seen, so only later changes are marked new.
pub fn note_start(dir: &Path, updated: bool) {
    let mut seen = Seen::load(dir);
    if updated {
        // Only with something not shown before: a Nightly that adds nothing stays quiet.
        let known: HashSet<&String> = seen.seen.iter().collect();
        let unseen = current_ids().iter().any(|id| !known.contains(id));
        if !unseen || seen.pending {
            return;
        }
        seen.pending = true;
    } else if seen.seen.is_empty() && !seen.pending {
        seen.seen = current_ids();
    } else {
        return;
    }
    seen.save(dir);
}

/// Whether the sheet should open by itself.
pub fn pending(dir: &Path) -> bool {
    Seen::load(dir).pending
}

/// The sheet as JSON: this build's release first, then the ones before it,
/// with how many entries are new here.
pub fn json(dir: &Path) -> String {
    let seen: HashSet<String> = Seen::load(dir).seen.into_iter().collect();
    let releases = releases(&seen);
    let unseen = releases
        .first()
        .map_or(0, |r| r.sections.iter().flat_map(|s| &s.entries).filter(|e| e.unseen).count());
    serde_json::json!({ "releases": releases, "newCount": unseen }).to_string()
}

/// The sheet was closed: everything in this build is seen.
pub fn mark_seen(dir: &Path) {
    let mut seen = Seen::load(dir);
    let mut ids: Vec<String> = current_ids();
    let current: HashSet<&String> = ids.iter().collect();
    // Keep older IDs too (a Nightly's section can lose an entry it shows
    // again later), newest first, up to the limit.
    let older: Vec<String> = seen.seen.drain(..).filter(|id| !current.contains(id)).collect();
    ids.extend(older);
    ids.truncate(SEEN_LIMIT);
    seen.seen = ids;
    seen.pending = false;
    seen.save(dir);
}

/// This build's release first, then the ones before it.
pub fn releases(seen: &HashSet<String>) -> Vec<Release> {
    let mut releases = parse(CHANGELOG);
    releases.truncate(EARLIER + 1);
    for entry in releases.iter_mut().flat_map(|r| &mut r.sections).flat_map(|s| &mut s.entries) {
        entry.unseen = !seen.contains(&entry.id);
    }
    releases
}

/// The IDs of this build's entries, to mark them all seen.
pub fn current_ids() -> Vec<String> {
    parse(CHANGELOG)
        .into_iter()
        .next()
        .map(|r| r.sections.into_iter().flat_map(|s| s.entries).map(|e| e.id).collect())
        .unwrap_or_default()
}

fn parse(text: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    // The list item being read: its lines so far.
    let mut item: Option<Vec<String>> = None;
    let mut in_table_section = false;

    fn finish(releases: &mut [Release], item: &mut Option<Vec<String>>) {
        let Some(lines) = item.take() else { return };
        let Some(release) = releases.last_mut() else { return };
        let Some(section) = release.sections.last_mut() else { return };
        let text = lines.join(" ");
        let (title, rest) = split_lead(&text);
        let id = entry_id(&release.version, &section.title, &text);
        section.entries.push(Entry { id, title, body: rich(rest.trim()), unseen: false });
    }

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            finish(&mut releases, &mut item);
            let (version, date) = split_heading(heading);
            releases.push(Release { version, date, summary: String::new(), sections: Vec::new() });
            in_table_section = false;
            continue;
        }
        if let Some(title) = line.strip_prefix("### ") {
            finish(&mut releases, &mut item);
            let title = title.trim();
            // Download tables and the thanks paragraph aren't changes.
            in_table_section = matches!(title, "Downloads" | "Thanks");
            if !in_table_section && let Some(release) = releases.last_mut() {
                release.sections.push(Section { title: title.to_owned(), entries: Vec::new() });
            }
            continue;
        }
        let Some(before_sections) = releases.last().map(|r| r.sections.is_empty()) else { continue };
        if in_table_section {
            continue;
        }
        if let Some(first) = line.strip_prefix("- ") {
            finish(&mut releases, &mut item);
            item = Some(vec![first.trim().to_owned()]);
        } else if let Some(lines) = item.as_mut()
            && line.starts_with("  ")
            && !line.trim().is_empty()
        {
            lines.push(line.trim().to_owned());
        } else if line.trim().is_empty() {
            finish(&mut releases, &mut item);
        } else if before_sections {
            finish(&mut releases, &mut item);
            // The paragraph under a version's heading.
            if let Some(release) = releases.last_mut() {
                if !release.summary.is_empty() {
                    release.summary.push(' ');
                }
                release.summary.push_str(line.trim());
            }
        }
    }
    finish(&mut releases, &mut item);
    for release in &mut releases {
        release.summary = rich(&release.summary);
        release.sections.retain(|s| !s.entries.is_empty());
    }
    releases.retain(|r| !r.sections.is_empty() || !r.summary.is_empty());
    releases
}

/// "0.1.6 (unreleased)" to ("0.1.6", ""); "0.1.5 (2026-11-05)" to its date.
fn split_heading(heading: &str) -> (String, String) {
    let heading = heading.trim();
    match heading.rsplit_once(" (") {
        Some((version, rest)) => {
            let date = rest.trim_end_matches(')');
            let date = if date.eq_ignore_ascii_case("unreleased") { "" } else { date };
            (version.trim().to_owned(), date.to_owned())
        }
        None => (heading.to_owned(), String::new()),
    }
}

/// An entry's bold lead ("**Two PCs work together.** Pair a PC...") and the
/// rest. Entries without one have no title.
fn split_lead(text: &str) -> (String, &str) {
    if let Some(rest) = text.strip_prefix("**")
        && let Some(end) = rest.find("**")
    {
        let title = rest[..end].trim();
        let title = title.trim_end_matches([',', ':']).trim_end();
        return (plain(title), &rest[end + 2..]);
    }
    (String::new(), text)
}

/// A stable ID for an entry: the same text in the same release is the same
/// entry, however the changelog around it moves.
fn entry_id(version: &str, section: &str, text: &str) -> String {
    // FNV-1a: stable across Rust versions and builds, unlike `DefaultHasher`.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in version.bytes().chain([0]).chain(section.bytes()).chain([0]).chain(text.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Markdown's inline marks as plain text: `**`, backticks and link targets go.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        match link_at(&rest[start..]) {
            Some((label, _, len)) => {
                out.push_str(label);
                rest = &rest[start + len..];
            }
            None => {
                out.push('[');
                rest = &rest[start + 1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// `[label](url)` at the start of `text`: the label, the URL and its length.
fn link_at(text: &str) -> Option<(&str, &str, usize)> {
    let close = text.find("](")?;
    let label = &text[1..close];
    if label.contains('[') {
        return None;
    }
    let after = &text[close + 2..];
    let end = after.find(')')?;
    let url = &after[..end];
    if url.contains(char::is_whitespace) {
        return None;
    }
    Some((label, url, close + 2 + end + 1))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Markdown's inline marks as Qt rich text: bold, code (bold too) and links.
/// Anything else is escaped, so the text can't add markup of its own.
fn rich(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut bold = false;
    let mut code = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**") {
            out.push_str(if bold { "</b>" } else { "<b>" });
            bold = !bold;
            rest = after;
        } else if let Some(after) = rest.strip_prefix('`') {
            out.push_str(if code { "</b>" } else { "<b>" });
            code = !code;
            rest = after;
        } else if rest.starts_with('[')
            && let Some((label, url, len)) = link_at(rest)
            && (url.starts_with("https://") || url.starts_with("http://"))
        {
            // A URL from `link_at` has no whitespace; quotes and brackets are
            // kept out, and `&` stays as it is (StyledText doesn't decode attributes).
            let url = url.replace(['"', '<', '>'], "");
            out.push_str(&format!("<a href=\"{url}\">{}</a>", escape(&plain(label))));
            rest = &rest[len..];
        } else {
            let next = rest.char_indices().skip(1).map(|(i, _)| i).next().unwrap_or(rest.len());
            out.push_str(&escape(&rest[..next]));
            rest = &rest[next..];
        }
    }
    if code {
        out.push_str("</b>");
    }
    if bold {
        out.push_str("</b>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog

Intro.

## 0.1.6 (unreleased)

### New

- **Two PCs work together.** Pair a PC with another PC, the
  same way as a phone ([#432](https://github.com/thtbee/nectarlink/issues/432)).
- **Scroll with the air mouse.** Drag two fingers.

### Fixed

- Voice typing comes through in `Notepad`.

## 0.1.5 (2026-11-05)

The PC app on Linux, and the fixes for
what people reported.

### Downloads

| Download | For |
|---|---|
| x | y |

### New

- **Phone as microphone.** Use it in calls.

### Thanks

To [@someone](https://github.com/someone) for testing.

## 0.1.0 \"Pollen\" (2026-10-14)

### Notifications

- Mirrors notifications.
";

    #[test]
    fn reads_releases_sections_and_entries() {
        let releases = parse(SAMPLE);
        assert_eq!(releases.len(), 3);
        let next = &releases[0];
        assert_eq!((next.version.as_str(), next.date.as_str()), ("0.1.6", ""));
        assert_eq!(next.sections.len(), 2);
        assert_eq!(next.sections[0].title, "New");
        let first = &next.sections[0].entries[0];
        assert_eq!(first.title, "Two PCs work together.");
        assert_eq!(
            first.body,
            "Pair a PC with another PC, the same way as a phone \
             (<a href=\"https://github.com/thtbee/nectarlink/issues/432\">#432</a>)."
        );
        assert_eq!(next.sections[1].entries[0].title, "");
        assert_eq!(next.sections[1].entries[0].body, "Voice typing comes through in <b>Notepad</b>.");

        let last = &releases[1];
        assert_eq!((last.version.as_str(), last.date.as_str()), ("0.1.5", "2026-11-05"));
        assert_eq!(last.summary, "The PC app on Linux, and the fixes for what people reported.");
        assert_eq!(last.sections.len(), 1, "no Downloads or Thanks: {:?}", last.sections);
        assert_eq!(releases[2].version, "0.1.0 \"Pollen\"");
        assert_eq!(releases[2].sections[0].title, "Notifications");
    }

    #[test]
    fn the_same_entry_keeps_its_id_and_a_new_one_gets_its_own() {
        let before = parse(SAMPLE);
        let grown = SAMPLE.replace("### New\n\n- **Two", "### New\n\n- **Gamepad.** New.\n- **Two");
        let after = parse(&grown);
        let ids = |r: &[Release]| -> HashSet<String> {
            r[0].sections.iter().flat_map(|s| &s.entries).map(|e| e.id.clone()).collect()
        };
        let (old, new) = (ids(&before), ids(&after));
        assert!(old.is_subset(&new));
        assert_eq!(new.len(), old.len() + 1);
    }

    #[test]
    fn text_cant_add_markup_and_odd_links_stay_text() {
        assert_eq!(rich("a <script> & \"b\""), "a &lt;script&gt; &amp; &quot;b&quot;");
        assert_eq!(rich("[x](javascript:alert(1))"), "[x](javascript:alert(1))");
        assert_eq!(rich("**open"), "<b>open</b>");
        assert_eq!(plain("**Hi [there](https://x.y)** `now`"), "Hi there now");
    }

    #[test]
    fn an_update_opens_the_sheet_and_a_new_install_starts_with_all_seen() {
        let fresh = tempfile::tempdir().unwrap();
        note_start(fresh.path(), false);
        assert!(!pending(fresh.path()));
        assert!(json(fresh.path()).contains("\"newCount\":0"));
        // Updated to a build with nothing new: it stays closed.
        note_start(fresh.path(), true);
        assert!(!pending(fresh.path()));

        // Updated from a version that never showed these notes.
        let dir = tempfile::tempdir().unwrap();
        note_start(dir.path(), true);
        assert!(pending(dir.path()));
        mark_seen(dir.path());
        assert!(!pending(dir.path()));
        // A later start without an update changes nothing.
        note_start(dir.path(), false);
        assert!(!pending(dir.path()));
    }

    #[test]
    fn this_builds_changelog_reads() {
        let fresh = releases(&HashSet::new());
        assert!(!fresh.is_empty());
        assert!(fresh.len() <= EARLIER + 1);
        assert!(fresh[0].sections.iter().all(|s| !s.entries.is_empty()));
        assert!(fresh[0].sections.iter().flat_map(|s| &s.entries).all(|e| e.unseen));
        let ids: HashSet<String> = current_ids().into_iter().collect();
        assert!(releases(&ids)[0].sections.iter().flat_map(|s| &s.entries).all(|e| !e.unseen));
    }
}
