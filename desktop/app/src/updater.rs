// SPDX-License-Identifier: GPL-3.0-or-later
//! Updates for an installed copy: the latest GitHub release is checked at
//! startup and once a day (unless turned off). When it's newer, the user is
//! told and can update: the installer is downloaded, checked against the
//! release's SHA-256 sums, and run silently; it closes this copy, installs
//! and opens the new one.

use std::{
    path::PathBuf,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    core_host,
    state::Changes,
    win::{autostart, http, toast},
};

const LATEST: &str = "https://api.github.com/repos/thtbee/nectarlink/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(24 * 3600);
const MAX_INSTALLER: usize = 400 * 1024 * 1024;
/// The toast "device" for update notices.
pub const TOAST_GROUP: &str = "update";
pub const ACTION_UPDATE: &str = "update";

/// A newer release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes_url: String,
    installer_url: String,
    installer_name: String,
    sums_url: String,
}

static AUTO: AtomicBool = AtomicBool::new(true);
static CHECKING: AtomicBool = AtomicBool::new(false);
static FOUND: Mutex<Option<Update>> = Mutex::new(None);

fn found() -> MutexGuard<'static, Option<Update>> {
    FOUND.lock().unwrap_or_else(|e| e.into_inner())
}

/// The update found, if any.
pub fn available() -> Option<Update> {
    found().clone()
}

pub fn set_auto(on: bool) {
    AUTO.store(on, Ordering::Relaxed);
}

/// Whether this copy updates itself (an installed one, not a build run
/// from its folder).
pub fn can_update() -> bool {
    std::env::current_exe().is_ok_and(|exe| autostart::is_installed(&exe))
}

/// Checks soon after startup, then daily, while automatic checks are on.
pub fn start() {
    if !can_update() {
        return;
    }
    let spawned = std::thread::Builder::new().name("updates".into()).spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if AUTO.load(Ordering::Relaxed) {
                check_now(false);
            }
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "won't check for updates");
    }
}

/// Checks now (blocking). `told`: the user asked, so say "up to date" too.
/// Returns what to tell the user.
pub fn check_now(told: bool) -> Option<String> {
    if CHECKING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let result = latest();
    CHECKING.store(false, Ordering::Release);
    match result {
        Ok(Some(update)) => {
            let new = found().as_ref() != Some(&update);
            *found() = Some(update.clone());
            core_host::host().hub.update(|_| Changes::UPDATE);
            if new {
                announce(&update);
            }
            Some(format!("Nectarlink {} is available.", update.version))
        }
        Ok(None) => told.then(|| "Nectarlink is up to date.".to_owned()),
        Err(e) => {
            tracing::info!(error = %e, "couldn't check for updates");
            told.then(|| "Couldn't check for updates. Check your internet connection.".to_owned())
        }
    }
}

fn announce(update: &Update) {
    toast::show(toast::Toast {
        device: TOAST_GROUP.into(),
        key: update.version.clone(),
        title: format!("Nectarlink {} is available", update.version),
        body: "Update now, or later from Settings.".into(),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: vec![(ACTION_UPDATE.into(), "Update".into())],
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn latest() -> Result<Option<Update>, String> {
    let body = http::get(LATEST, 1024 * 1024)?;
    let release: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(newer_release(release, env!("CARGO_PKG_VERSION")))
}

const INSTALLER_SUFFIX: &str =
    if cfg!(target_arch = "aarch64") { "-arm64-setup.exe" } else { "-x64-setup.exe" };

/// The release as an update, if it's a newer published one with an
/// installer for this architecture and its sums.
fn newer_release(release: Release, current: &str) -> Option<Update> {
    newer_release_for(release, current, INSTALLER_SUFFIX)
}

fn newer_release_for(release: Release, current: &str, suffix: &str) -> Option<Update> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = release.tag_name.trim_start_matches('v').to_owned();
    if !is_newer(&version, current) {
        return None;
    }
    let installer = release.assets.iter().find(|a| a.name.ends_with(suffix))?;
    let sums = release.assets.iter().find(|a| a.name == "SHA256SUMS.txt")?;
    Some(Update {
        version,
        notes_url: release.html_url,
        installer_url: installer.browser_download_url.clone(),
        installer_name: installer.name.clone(),
        sums_url: sums.browser_download_url.clone(),
    })
}

/// A version's numbers, and its pre-release part.
type Version<'a> = ((u64, u64, u64), Option<&'a str>);

/// `1.2.3` style versions, with an optional `-pre` part (a release is
/// newer than its pre-releases).
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<Version<'_>> {
        let (numbers, pre) = match v.split_once('-') {
            Some((n, p)) => (n, Some(p)),
            None => (v, None),
        };
        let mut parts = numbers.split('.').map(str::parse::<u64>);
        let version = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next().unwrap_or(Ok(0)).ok()?);
        Some((version, pre))
    }
    let (Some((a, a_pre)), Some((b, b_pre))) = (parse(candidate), parse(current)) else { return false };
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

/// The SHA-256 a sums file lists for `name` (`<hex>  <name>` lines).
fn listed_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads, checks and runs the installer (blocking). Returns what went
/// wrong, in words for the user.
pub fn install() -> Result<(), String> {
    let update = available().ok_or("There's no update to install.")?;
    let failed = |e: String| {
        tracing::warn!(error = e, "update failed");
        "The update couldn't be downloaded. Try again later.".to_owned()
    };
    let sums = http::get(&update.sums_url, 64 * 1024).map_err(failed)?;
    let expected = listed_hash(&String::from_utf8_lossy(&sums), &update.installer_name)
        .ok_or("The update's checksum is missing, so it wasn't installed.")?;
    let installer = http::get(&update.installer_url, MAX_INSTALLER).map_err(failed)?;
    let actual = Sha256::digest(&installer).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if actual != expected {
        tracing::warn!("update checksum mismatch");
        return Err("The download was damaged, so it wasn't installed. Try again later.".into());
    }
    let dir = std::env::temp_dir().join("Nectarlink update");
    let path: PathBuf = dir.join(&update.installer_name);
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, &installer))
        .map_err(|e| failed(e.to_string()))?;
    // Silent, then opens the new version (/RUN); Windows asks for
    // permission first. The installer closes this copy itself.
    crate::win::shell::run(&path, "/S /RUN").map_err(failed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("0.1.0", "0.0.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(!is_newer("0.0.1", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: "https://github.com/thtbee/nectarlink/releases/tag/v9".into(),
            draft: false,
            prerelease: false,
            assets: vec![
                Asset {
                    name: "Nectarlink-9.0.0-x64-setup.exe".into(),
                    browser_download_url: "https://x/setup".into(),
                },
                Asset {
                    name: "Nectarlink-9.0.0-arm64-setup.exe".into(),
                    browser_download_url: "https://x/setup-arm64".into(),
                },
                Asset { name: "SHA256SUMS.txt".into(), browser_download_url: "https://x/sums".into() },
            ],
        }
    }

    #[test]
    fn releases_become_updates_when_newer_and_complete() {
        let x64 = newer_release_for(release("v9.0.0"), "0.0.1", "-x64-setup.exe").unwrap();
        assert_eq!((x64.version.as_str(), x64.installer_url.as_str()), ("9.0.0", "https://x/setup"));
        let arm64 = newer_release_for(release("v9.0.0"), "0.0.1", "-arm64-setup.exe").unwrap();
        assert_eq!(
            (arm64.installer_name.as_str(), arm64.installer_url.as_str()),
            ("Nectarlink-9.0.0-arm64-setup.exe", "https://x/setup-arm64")
        );
        assert!(newer_release(release("v0.0.1"), "0.0.1").is_none());
        let mut draft = release("v9.0.0");
        draft.prerelease = true;
        assert!(newer_release(draft, "0.0.1").is_none());
        let mut no_sums = release("v9.0.0");
        no_sums.assets.pop();
        assert!(newer_release(no_sums, "0.0.1").is_none());
    }

    #[test]
    fn finds_the_installer_hash() {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  Nectarlink-9.0.0-x64-setup.exe\n{}  other.txt\n", "b".repeat(64));
        assert_eq!(listed_hash(&sums, "Nectarlink-9.0.0-x64-setup.exe"), Some(hash));
        assert_eq!(listed_hash(&sums, "missing.exe"), None);
        assert_eq!(listed_hash(&format!("{}  *x.exe", "C".repeat(64)), "x.exe"), Some("c".repeat(64)));
    }
}
