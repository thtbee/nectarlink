// SPDX-License-Identifier: GPL-3.0-or-later
//! The release channels' feeds: one small JSON file per channel, which the
//! release and nightly workflows publish on the repository's `channels`
//! branch. Checking one is a conditional GET: while nothing changed, GitHub
//! answers 304 with no body, so checking often costs next to nothing.
//!
//! A feed only ever points at this repository's release downloads, and
//! lists each file's SHA-256, which the updaters check before installing.
//!
//! ```json
//! { "schema": 1, "channel": "nightly", "version": "0.1.5-nightly.7",
//!   "name": "Amber Clover", "published": "2026-11-03T10:17:00Z",
//!   "notes": "https://github.com/thtbee/nectarlink/releases/tag/v0.1.5-nightly.7",
//!   "files": [{ "name": "Nectarlink-0.1.5-nightly.7-x64-setup.exe",
//!               "url": "https://github.com/thtbee/nectarlink/releases/download/…",
//!               "sha256": "…", "size": 41234567 }] }
//! ```

use std::{collections::HashMap, sync::Mutex};

use serde::Deserialize;

use crate::{
    version::{self, Channel},
    win::http::{self, Fresh},
};

/// Where the feeds are: `<FEEDS><channel>.json`.
const FEEDS: &str = "https://raw.githubusercontent.com/thtbee/nectarlink/channels/";
/// The only place a feed's files may come from.
pub const RELEASE_DOWNLOADS: &str = "https://github.com/thtbee/nectarlink/releases/download/";
/// Release notes live on the repository's own pages.
const NOTES: &str = "https://github.com/thtbee/nectarlink/";
/// A feed is a few hundred bytes; anything near this is not one.
const MAX_FEED: usize = 64 * 1024;
const SCHEMA: u32 = 1;

/// A channel's newest build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Feed {
    pub schema: u32,
    pub channel: String,
    pub version: String,
    /// A nightly's name ("Amber Clover"), or a release's.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub published: Option<String>,
    pub notes: String,
    pub files: Vec<FeedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FeedFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

/// The last copy of each feed and its ETag, for conditional checks.
static CACHE: Mutex<Option<HashMap<Channel, (String, Feed)>>> = Mutex::new(None);

/// A file name an updater may save under: plain characters only, never a path.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

/// Checks a feed's content; files that don't pass are dropped.
fn validated(mut feed: Feed, channel: Channel) -> Result<Feed, String> {
    if feed.schema != SCHEMA {
        return Err(format!("feed schema {} (this version reads {SCHEMA})", feed.schema));
    }
    if Channel::parse(&feed.channel) != Some(channel) {
        return Err(format!("the {} feed says it's {}", channel.as_str(), feed.channel));
    }
    if version::compare(&feed.version, "0.0.0").is_none() || feed.version.len() > 64 {
        return Err(format!("not a version: {}", feed.version));
    }
    // A stable feed lists only releases, a nightly feed only nightlies.
    if version::channel_of(&feed.version) != channel {
        return Err(format!("{} isn't a {} build", feed.version, channel.as_str()));
    }
    if !feed.notes.starts_with(NOTES) {
        feed.notes = format!("{NOTES}releases");
    }
    feed.name = feed.name.filter(|n| n.len() <= 64 && !n.chars().any(char::is_control));
    feed.files.retain(|f| {
        is_plain_name(&f.name)
            && f.url.starts_with(RELEASE_DOWNLOADS)
            && !f.url[RELEASE_DOWNLOADS.len()..].contains("..")
            && f.sha256.len() == 64
            && f.sha256.chars().all(|c| c.is_ascii_hexdigit())
    });
    for file in &mut feed.files {
        file.sha256.make_ascii_lowercase();
    }
    Ok(feed)
}

/// The channel's feed. `Ok(None)`: none is published (yet). Errors are
/// network trouble or a feed that isn't one.
pub fn fetch(channel: Channel) -> Result<Option<Feed>, String> {
    let url = format!("{FEEDS}{}.json", channel.as_str());
    let cached =
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(&channel).cloned());
    match http::get_fresh(&url, MAX_FEED, cached.as_ref().map(|(etag, _)| etag.as_str())) {
        Ok(Fresh::NotModified) => Ok(cached.map(|(_, feed)| feed)),
        Ok(Fresh::Changed { body, etag }) => {
            let feed: Feed = serde_json::from_slice(&body).map_err(|e| format!("feed: {e}"))?;
            let feed = validated(feed, channel)?;
            let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            let cache = cache.get_or_insert_default();
            match etag {
                Some(etag) => cache.insert(channel, (etag, feed.clone())),
                None => cache.remove(&channel),
            };
            Ok(Some(feed))
        }
        Err(e) if e == "HTTP 404" => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(channel: &str, version: &str) -> Feed {
        Feed {
            schema: 1,
            channel: channel.into(),
            version: version.into(),
            name: Some("Amber Clover".into()),
            published: None,
            notes: "https://github.com/thtbee/nectarlink/releases/tag/v1".into(),
            files: vec![FeedFile {
                name: "Nectarlink-0.1.5-nightly.7-x64-setup.exe".into(),
                url: format!("{RELEASE_DOWNLOADS}v0.1.5-nightly.7/Nectarlink-0.1.5-nightly.7-x64-setup.exe"),
                sha256: "A".repeat(64),
                size: 1,
            }],
        }
    }

    #[test]
    fn a_good_feed_passes_with_its_hash_lowercased() {
        let checked = validated(feed("nightly", "0.1.5-nightly.7"), Channel::Nightly).unwrap();
        assert_eq!(checked.files.len(), 1);
        assert_eq!(checked.files[0].sha256, "a".repeat(64));
        assert_eq!(checked.name.as_deref(), Some("Amber Clover"));
    }

    #[test]
    fn feeds_must_match_their_channel_and_schema() {
        assert!(validated(feed("stable", "0.1.5-nightly.7"), Channel::Nightly).is_err());
        assert!(validated(feed("nightly", "0.1.5"), Channel::Nightly).is_err());
        assert!(validated(feed("stable", "0.1.5-nightly.7"), Channel::Stable).is_err());
        assert!(validated(feed("stable", "0.1.5"), Channel::Stable).is_ok());
        let mut newer = feed("stable", "0.1.5");
        newer.schema = 2;
        assert!(validated(newer, Channel::Stable).is_err());
        assert!(validated(feed("stable", "latest"), Channel::Stable).is_err());
    }

    #[test]
    fn files_from_elsewhere_or_with_odd_names_are_dropped() {
        let mut f = feed("nightly", "0.1.5-nightly.7");
        let good = f.files[0].clone();
        f.files = vec![
            FeedFile { url: "https://evil.example/Nectarlink-setup.exe".into(), ..good.clone() },
            FeedFile { url: format!("{RELEASE_DOWNLOADS}../../other/x.exe"), ..good.clone() },
            FeedFile { name: "..\\setup.exe".into(), ..good.clone() },
            FeedFile { sha256: "xyz".into(), ..good.clone() },
            good,
        ];
        f.notes = "https://evil.example/notes".into();
        let checked = validated(f, Channel::Nightly).unwrap();
        assert_eq!(checked.files.len(), 1);
        assert!(checked.notes.starts_with(NOTES));
    }

    #[test]
    fn real_json_parses() {
        let json = br#"{"schema":1,"channel":"nightly","version":"0.1.5-nightly.7","name":"Amber Clover",
            "published":"2026-11-03T10:17:00Z","notes":"https://github.com/thtbee/nectarlink/releases/tag/v0.1.5-nightly.7",
            "files":[{"name":"Nectarlink-0.1.5-nightly.7-android.apk",
            "url":"https://github.com/thtbee/nectarlink/releases/download/v0.1.5-nightly.7/Nectarlink-0.1.5-nightly.7-android.apk",
            "sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":12}],"later":"ignored"}"#;
        let parsed: Feed = serde_json::from_slice(json).unwrap();
        assert_eq!(validated(parsed, Channel::Nightly).unwrap().files.len(), 1);
    }
}
