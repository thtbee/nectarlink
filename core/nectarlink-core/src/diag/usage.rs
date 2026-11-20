// SPDX-License-Identifier: MPL-2.0
//! The daily count (`tools/report-worker`, `/v1/ping`): once a UTC day,
//! each install says its version, platform, OS and phone model or graphics,
//! and how its features did the day before. No ID, nothing personal: the
//! service adds it to daily totals and throws it away.

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{health, send};

/// Zero bits the daily count's proof needs: a few milliseconds.
pub const PROOF_BITS: u32 = 16;
const FILE: &str = "count.json";

/// What an install says about itself in the daily count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Install {
    pub app: String,
    /// `stable`, `nightly` or `dev`.
    pub channel: String,
    /// `windows`, `linux`, `macos` or `android`.
    pub platform: String,
    /// `Windows 11`, `Android 16`.
    pub os: String,
    /// A phone's maker and model, or a PC's graphics.
    pub hardware: String,
}

#[derive(Serialize)]
struct Wire<'a> {
    v: u32,
    app: &'a str,
    channel: &'a str,
    platform: &'a str,
    os: String,
    hardware: String,
    features: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Sent {
    day: String,
}

/// The day's count to POST to `{REPORT_SERVICE}/v1/ping`, with its proof
/// (`X-Proof`), or `None` when today's went out already or there's no
/// diagnostics folder. Includes yesterday's feature health from `dir`.
pub fn due(dir: &Path, install: &Install, now: u64) -> Option<(String, String)> {
    let today = day_of(now);
    let sent: Sent =
        fs::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if sent.day == today {
        return None;
    }
    let yesterday = day_of(now.saturating_sub(86_400));
    let features = health::days(dir)
        .into_iter()
        .find(|d| d.day == yesterday)
        .map(|d| {
            d.counts
                .into_iter()
                .take(16)
                .map(|(f, codes)| (f, codes.into_iter().take(16).collect()))
                .collect()
        })
        .unwrap_or_default();
    let wire = Wire {
        v: 1,
        app: &install.app,
        channel: &install.channel,
        platform: &install.platform,
        os: plain(&install.os),
        hardware: plain(&install.hardware),
        features,
    };
    let body = serde_json::to_string(&wire).ok()?;
    let hash: String = Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let proof = send::prove(&hash, now, PROOF_BITS);
    Some((body, proof))
}

/// Notes that today's count went out (or was refused for good).
pub fn sent(dir: &Path, now: u64) {
    let _ = fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec(&Sent { day: day_of(now) }) {
        let _ = fs::write(dir.join(FILE), json);
    }
}

/// What the service takes: letters, digits, spaces and `._()+/-`, at most 60.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ._()+/-".contains(*c))
        .take(60)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn day_of(secs: u64) -> String {
    let (y, m, d) = super::layer::civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> Install {
        Install {
            app: "0.1.6".into(),
            channel: "stable".into(),
            platform: "android".into(),
            os: "Android 16 (API 36)".into(),
            hardware: "Google Pixel 9 \"Pro\" <x>".into(),
        }
    }

    #[test]
    fn once_a_day_with_yesterdays_health() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_795_000_000;
        let (body, proof) = due(dir.path(), &install(), now).unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["platform"], "android");
        assert_eq!(json["os"], "Android 16 (API 36)");
        assert_eq!(json["hardware"], "Google Pixel 9 Pro x");
        assert_eq!(json["features"], serde_json::json!({}));
        assert!(json.get("id").is_none());
        let hash: String = Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let (t, n) = proof.split_once('.').unwrap();
        let digest = Sha256::digest(format!("{hash}|{t}|{n}").as_bytes());
        assert!(digest[0] == 0 && digest[1] == 0);

        sent(dir.path(), now);
        assert!(due(dir.path(), &install(), now + 60).is_none());
        assert!(due(dir.path(), &install(), now + 86_400).is_some());
    }
}
