// SPDX-License-Identifier: MPL-2.0
//! What's new (#461): the release notes of this build, read from the
//! CHANGELOG.md it was built from, so they match it exactly and need no
//! network. The top section is always this build's: the next release's notes
//! on a Nightly, its own on a release.
//!
//! Each entry has an ID made from its text, and the IDs already shown are
//! saved, by each app in its own data folder. After an update with something
//! new, a dot on the What's new button says so, and the panel lists only what
//! changed since the last look, one short line per change (its bold lead),
//! Nightly to Nightly too, where the version's section only grows.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::Serialize;

const CHANGELOG: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"));

/// How many releases before this one the sheet offers.
#[cfg(test)]
const EARLIER: usize = 4;
/// How many shown entries are remembered: far more than a release has.
pub const SEEN_LIMIT: usize = 2000;
/// Where the full notes are.
const RELEASES_URL: &str = "https://github.com/thtbee/nectarlink/releases";
/// Points per group; the rest is "and N more".
const MAX_POINTS: usize = 5;
/// How many entries of the last update are kept to show again.
const BATCH_LIMIT: usize = 400;
/// A point longer than this is cut at a word.
const MAX_POINT_CHARS: usize = 90;

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
    /// Not shown before on this device.
    #[serde(rename = "new")]
    pub unseen: bool,
    /// The markdown after the bold lead, for an entry without one.
    #[serde(skip)]
    pub text: String,
    /// The ID from the whole text, as before: kept to carry over what was
    /// seen.
    #[serde(skip)]
    pub legacy_id: String,
}

/// What this device has seen, in `whats-new.json`.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Seen {
    #[serde(default)]
    seen: Vec<String>,
    /// Updated with something new since the panel was last opened: the dot shows.
    #[serde(default)]
    pending: bool,
    /// What the last update brought, to show again after the dot is gone.
    #[serde(default)]
    batch: Vec<String>,
    /// The newest release when the panel was last opened.
    #[serde(default)]
    version: String,
    /// Which IDs `seen` holds: 0 the whole entry's text, 2 its point.
    #[serde(default)]
    ids: u8,
}

/// IDs from an entry's point, so a credit or a reworded line doesn't make an
/// old entry new again.
const IDS: u8 = 2;

impl Seen {
    fn path(dir: &Path) -> PathBuf {
        dir.join("whats-new.json")
    }

    fn load(dir: &Path) -> Self {
        let mut seen: Self = std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if seen.ids < IDS {
            seen.upgrade(&parse(CHANGELOG));
        }
        seen
    }

    /// Carries what was seen over from whole-text IDs to point IDs. An entry
    /// edited since it was seen can't be matched and counts as unseen once.
    fn upgrade(&mut self, releases: &[Release]) {
        let renamed: std::collections::HashMap<&str, &str> = releases
            .iter()
            .flat_map(|r| &r.sections)
            .flat_map(|s| &s.entries)
            .map(|e| (e.legacy_id.as_str(), e.id.as_str()))
            .collect();
        let map = |ids: &mut Vec<String>| {
            let mut out: Vec<String> = Vec::new();
            for id in ids.drain(..) {
                let id = renamed.get(id.as_str()).map_or(id, |new| (*new).to_owned());
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            *ids = out;
        };
        map(&mut self.seen);
        map(&mut self.batch);
        self.ids = IDS;
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

/// At start: after an update that brought something not seen, the dot
/// shows; on a new install everything counts as seen.
pub fn note_start(dir: &Path, updated: bool) {
    let mut seen = Seen::load(dir);
    // Most starts have nothing to note: then the changelog isn't parsed.
    if !updated && !(seen.seen.is_empty() && !seen.pending) {
        return;
    }
    let releases = parse(CHANGELOG);
    if updated {
        let known: HashSet<&String> = seen.seen.iter().collect();
        let unseen = ids_of(scope(&releases, &seen.version)).any(|id| !known.contains(id));
        if !unseen || seen.pending {
            return;
        }
        seen.pending = true;
    } else {
        seen.seen = ids_of(&releases[..releases.len().min(1)]).cloned().collect();
        seen.version = releases.first().map(|r| r.version.clone()).unwrap_or_default();
    }
    seen.save(dir);
}

/// Whether the dot shows.
pub fn pending(dir: &Path) -> bool {
    Seen::load(dir).pending
}

/// One group of points: "New", "Changed", "Fixed"...
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub title: String,
    pub items: Vec<String>,
    pub more: usize,
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub version: String,
    /// "new" (since the last look), "highlights" (nothing new) or "empty".
    pub mode: &'static str,
    #[serde(rename = "newCount")]
    pub new_count: usize,
    pub groups: Vec<Group>,
    pub url: &'static str,
}

/// The releases since the one last seen: from the top down to `version`, or
/// only this build's when it isn't known.
fn scope<'a>(releases: &'a [Release], version: &str) -> &'a [Release] {
    let end = releases.iter().position(|r| !version.is_empty() && r.version == version).map_or(1, |i| i + 1);
    &releases[..end.min(releases.len())]
}

fn ids_of(releases: &[Release]) -> impl Iterator<Item = &String> {
    releases.iter().flat_map(|r| &r.sections).flat_map(|s| &s.entries).map(|e| &e.id)
}

/// An entry as one short line: its bold lead, or else its first sentence
/// without credits or issue links.
fn point(entry: &Entry) -> Option<String> {
    let mut line = if entry.title.is_empty() {
        let mut text = plain(&entry.text);
        if let Some(at) = text.find(" Thanks to ") {
            text.truncate(at);
        }
        if let Some(at) = text.find(". ") {
            text.truncate(at);
        }
        drop_references(&text)
    } else {
        entry.title.clone()
    };
    line = line.trim().trim_end_matches('.').trim_end().to_owned();
    if line.chars().count() > MAX_POINT_CHARS {
        let cut: String = line.chars().take(MAX_POINT_CHARS - 2).collect();
        let at = cut.rfind(' ').unwrap_or(cut.len());
        line = format!("{}…", cut[..at].trim_end_matches([',', ';', ':', ' ']));
    }
    (!line.is_empty()).then_some(line)
}

/// Removes "(#123)", "(@name)" and the like.
fn drop_references(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')').map(|c| open + c) else { break };
        let inner = &rest[open + 1..close];
        out.push_str(&rest[..open]);
        if inner.contains('#') || inner.contains('@') {
            // Also the space before it.
            while out.ends_with(' ') {
                out.pop();
            }
        } else {
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Groups `entries` by section, in the order New, Changed, Fixed, then others.
fn groups<'a>(entries: impl Iterator<Item = (&'a str, &'a Entry)>) -> Vec<Group> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (section, entry) in entries {
        let Some(line) = point(entry) else { continue };
        match groups.iter_mut().find(|(title, _)| title == section) {
            Some((_, items)) => {
                if !items.contains(&line) {
                    items.push(line);
                }
            }
            None => groups.push((section.to_owned(), vec![line])),
        }
    }
    let rank = |title: &str| match title {
        "New" => 0,
        "Changed" => 1,
        "Fixed" => 2,
        _ => 3,
    };
    groups.sort_by_key(|(title, _)| rank(title));
    groups
        .into_iter()
        .map(|(title, mut items)| {
            let more = items.len().saturating_sub(MAX_POINTS);
            items.truncate(MAX_POINTS);
            Group { title, items, more }
        })
        .collect()
}

/// What the panel shows: what's new since the last look, or what the last
/// update brought, or this version's first points when nothing is new.
pub fn summary(dir: &Path) -> Summary {
    let seen = Seen::load(dir);
    let releases = parse(CHANGELOG);
    let version = releases.first().map(|r| r.version.clone()).unwrap_or_default();
    let known: HashSet<&String> = seen.seen.iter().collect();
    let batch: HashSet<&String> = seen.batch.iter().collect();
    let in_scope = || {
        scope(&releases, &seen.version)
            .iter()
            .flat_map(|r| &r.sections)
            .flat_map(|s| s.entries.iter().map(move |e| (s.title.as_str(), e)))
    };
    let new_count = in_scope().filter(|(_, e)| !known.contains(&e.id)).count();
    let (mode, groups) = if new_count > 0 {
        ("new", groups(in_scope().filter(|(_, e)| !known.contains(&e.id))))
    } else if in_scope().any(|(_, e)| batch.contains(&e.id)) {
        ("new", groups(in_scope().filter(|(_, e)| batch.contains(&e.id))))
    } else if let Some(current) = releases.first() {
        // Nothing new: the first points of this version, under "In this version".
        let first = current.sections.iter().find(|s| s.title == "New").or_else(|| current.sections.first());
        let groups =
            first.map(|s| groups(s.entries.iter().map(|e| (s.title.as_str(), e)))).unwrap_or_default();
        ("highlights", groups)
    } else {
        ("empty", Vec::new())
    };
    let mode = if groups.is_empty() { "empty" } else { mode };
    Summary { version, mode, new_count, groups, url: RELEASES_URL }
}

/// The panel as JSON.
pub fn json(dir: &Path) -> String {
    serde_json::to_string(&summary(dir)).unwrap_or_else(|_| "{}".into())
}

/// The panel opened: what it shows stays as the last update's, the dot goes.
pub fn mark_seen(dir: &Path) {
    let mut seen = Seen::load(dir);
    let releases = parse(CHANGELOG);
    let in_scope: Vec<String> = ids_of(scope(&releases, &seen.version)).cloned().collect();
    let known: HashSet<&String> = seen.seen.iter().collect();
    let unseen: Vec<String> = in_scope.iter().filter(|id| !known.contains(id)).cloned().collect();
    if !unseen.is_empty() {
        seen.batch = unseen;
        seen.batch.truncate(BATCH_LIMIT);
    }
    // Keep older IDs too (a Nightly's section can lose an entry it shows
    // again later), newest first, up to the limit.
    let current: HashSet<&String> = in_scope.iter().collect();
    let older: Vec<String> = seen.seen.drain(..).filter(|id| !current.contains(id)).collect();
    let mut ids = in_scope.clone();
    ids.extend(older);
    ids.truncate(SEEN_LIMIT);
    seen.seen = ids;
    seen.version = releases.first().map(|r| r.version.clone()).unwrap_or_default();
    seen.pending = false;
    seen.save(dir);
}

/// This build's release first, then the ones before it.
#[cfg(test)]
pub fn releases(seen: &HashSet<String>) -> Vec<Release> {
    let mut releases = parse(CHANGELOG);
    releases.truncate(EARLIER + 1);
    for entry in releases.iter_mut().flat_map(|r| &mut r.sections).flat_map(|s| &mut s.entries) {
        entry.unseen = !seen.contains(&entry.id);
    }
    releases
}

/// The IDs of this build's entries, to mark them all seen.
#[cfg(test)]
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
        let legacy_id = entry_id(&release.version, &section.title, &text);
        let mut entry = Entry {
            id: String::new(),
            title,
            body: rich(rest.trim()),
            unseen: false,
            text: rest.trim().to_owned(),
            legacy_id,
        };
        // Its point, so editing the rest (a credit, a reworded sentence)
        // keeps it the same entry; two with the same point stay apart.
        let key = point(&entry).unwrap_or_else(|| entry.text.clone());
        let mut id = entry_id(&release.version, &section.title, &key);
        let mut n = 1;
        while section.entries.iter().any(|e| e.id == id) {
            n += 1;
            id = entry_id(&release.version, &section.title, &format!("{key} #{n}"));
        }
        entry.id = id;
        section.entries.push(entry);
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
    fn a_credit_or_reworded_sentence_keeps_the_entry_seen() {
        let ids = |r: &[Release]| -> Vec<String> {
            r[0].sections.iter().flat_map(|s| &s.entries).map(|e| e.id.clone()).collect()
        };
        let lead = SAMPLE.find("- **").expect("an entry with a lead");
        let end = SAMPLE[lead..].find('\n').map_or(SAMPLE.len(), |i| lead + i);
        let edited =
            format!("{} Thanks to [@someone](https://github.com/someone).{}", &SAMPLE[..end], &SAMPLE[end..]);
        assert_eq!(ids(&parse(SAMPLE)), ids(&parse(&edited)));
    }

    #[test]
    fn what_was_seen_carries_over_to_the_new_ids() {
        let releases = parse(SAMPLE);
        let all: Vec<&Entry> = releases.iter().flat_map(|r| &r.sections).flat_map(|s| &s.entries).collect();
        let mut seen = Seen {
            seen: all.iter().map(|e| e.legacy_id.clone()).chain(["gone".to_owned()]).collect(),
            batch: vec![all[0].legacy_id.clone()],
            ..Seen::default()
        };
        seen.upgrade(&releases);
        assert_eq!(seen.ids, IDS);
        for e in &all {
            assert!(seen.seen.contains(&e.id), "{}", e.title);
        }
        assert!(seen.seen.contains(&"gone".to_owned()));
        assert_eq!(seen.batch, vec![all[0].id.clone()]);
    }

    #[test]
    fn text_cant_add_markup_and_odd_links_stay_text() {
        assert_eq!(rich("a <script> & \"b\""), "a &lt;script&gt; &amp; &quot;b&quot;");
        assert_eq!(rich("[x](javascript:alert(1))"), "[x](javascript:alert(1))");
        assert_eq!(rich("**open"), "<b>open</b>");
        assert_eq!(plain("**Hi [there](https://x.y)** `now`"), "Hi there now");
    }

    #[test]
    fn an_update_shows_the_dot_and_a_new_install_starts_with_all_seen() {
        let fresh = tempfile::tempdir().unwrap();
        note_start(fresh.path(), false);
        assert!(!pending(fresh.path()));
        let first = summary(fresh.path());
        assert_eq!((first.new_count, first.mode), (0, "highlights"));
        assert!(first.groups.iter().all(|g| g.items.len() <= MAX_POINTS));
        // Updated to a build with nothing new: no dot.
        note_start(fresh.path(), true);
        assert!(!pending(fresh.path()));

        // Updated from a version that never showed these notes.
        let dir = tempfile::tempdir().unwrap();
        note_start(dir.path(), true);
        assert!(pending(dir.path()));
        let before = summary(dir.path());
        assert_eq!(before.mode, "new");
        assert!(before.new_count > 0);
        mark_seen(dir.path());
        assert!(!pending(dir.path()));
        // Opened again: still what that update brought.
        let after = summary(dir.path());
        assert_eq!((after.mode, after.new_count), ("new", 0));
        assert_eq!(after.groups, before.groups);
        // A later start without an update changes nothing.
        note_start(dir.path(), false);
        assert!(!pending(dir.path()));
    }

    fn entry(title: &str, text: &str) -> Entry {
        Entry {
            id: String::new(),
            title: title.into(),
            body: String::new(),
            unseen: false,
            text: text.into(),
            legacy_id: String::new(),
        }
    }

    #[test]
    fn a_point_is_the_lead_or_a_short_first_sentence_without_credits() {
        assert_eq!(
            point(&entry("Two PCs work together.", "Pair a PC.")).as_deref(),
            Some("Two PCs work together")
        );
        assert_eq!(
            point(&entry("", "Voice typing works in Notepad ([#52](https://x.y/52)). It typed too fast."))
                .as_deref(),
            Some("Voice typing works in Notepad")
        );
        assert_eq!(
            point(&entry(
                "",
                "Send to opens faster Thanks to [@someone](https://x.y) ([#9](https://x.y/9))."
            ))
            .as_deref(),
            Some("Send to opens faster")
        );
        let long = "word ".repeat(40);
        let cut = point(&entry("", &long)).unwrap();
        assert!(cut.chars().count() <= MAX_POINT_CHARS && cut.ends_with('…'), "{cut}");
        assert_eq!(point(&entry("", "")), None);
    }

    #[test]
    fn groups_keep_five_points_and_count_the_rest() {
        let fixed: Vec<Entry> = (0..8).map(|i| entry(&format!("Fix {i}."), "")).collect();
        let new = [entry("Gamepad.", "")];
        let all = fixed.iter().map(|e| ("Fixed", e)).chain(new.iter().map(|e| ("New", e)));
        let g = groups(all);
        assert_eq!(g[0].title, "New");
        assert_eq!((g[1].title.as_str(), g[1].items.len(), g[1].more), ("Fixed", 5, 3));
    }

    #[test]
    fn the_scope_reaches_back_to_the_release_last_seen() {
        let releases = parse(SAMPLE);
        assert_eq!(scope(&releases, "").len(), 1);
        assert_eq!(scope(&releases, "0.1.5").len(), 2);
        assert_eq!(scope(&releases, "0.0.9").len(), 1, "an unknown version is this build's only");
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
