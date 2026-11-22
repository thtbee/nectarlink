// SPDX-License-Identifier: GPL-3.0-or-later
//! A phone app on this PC's desktop (Windows) or app menu (Linux), #370: a
//! shortcut that starts NectarLink with `--task open-app:<phone>:<package>`,
//! which the running copy (or this one) answers by opening the app in an app
//! window. The shortcut carries the app's own icon, from the icons NectarLink
//! keeps for the Apps page.

use std::path::{Path, PathBuf};

use nectarlink_core::DeviceId;

/// The task a shortcut passes, before `<phone>:<package>`.
pub const TASK_PREFIX: &str = "open-app:";

/// The task that opens `pkg` on `device`.
pub fn task(device: &DeviceId, pkg: &str) -> String {
    format!("{TASK_PREFIX}{device}:{pkg}")
}

/// The phone and package a task names, if it's one of these and well formed.
pub fn parse_task(action: &str) -> Option<(DeviceId, String)> {
    let rest = action.strip_prefix(TASK_PREFIX)?;
    let (device, pkg) = rest.split_once(':')?;
    let device: DeviceId = device.parse().ok()?;
    nectarlink_core::is_package_name(pkg).then(|| (device, pkg.to_owned()))
}

/// Whether this system gets such shortcuts.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// A name a file can take: no path or reserved characters, not empty.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn file_stem(label: &str, pkg: &str) -> String {
    let spaced: String = label
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = joined.trim_end_matches('.').trim();
    if cleaned.is_empty() { pkg.to_owned() } else { cleaned.chars().take(80).collect() }
}

/// How Windows starts an app window again from the taskbar (#465): the
/// command, and the app's icon as a resource (`path,0`) when there is one.
/// Without these, pinning an app window pinned NectarLink itself.
#[cfg(windows)]
pub fn relaunch(data_dir: &Path, device: DeviceId, pkg: &str) -> Option<(String, String)> {
    if !nectarlink_core::is_package_name(pkg) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let command = format!("\"{}\" {} {}", exe.display(), crate::win::jumplist::ARG, task(&device, pkg));
    let icon = crate::icons::existing(data_dir, pkg)
        .and_then(|png| imp::current_ico(data_dir, pkg, &png))
        .map(|ico| format!("{},0", ico.display()))
        .unwrap_or_default();
    Some((command, icon))
}

/// The shortcuts NectarLink made for `pkg` on `device` that are still there.
pub fn existing(device: DeviceId, pkg: &str) -> Vec<PathBuf> {
    if !nectarlink_core::is_package_name(pkg) {
        return Vec::new();
    }
    imp::existing(&task(&device, pkg), pkg)
}

/// Deletes the shortcuts for `pkg` on `device` (#370: Linux app menus have no
/// way to remove an entry); how many there were.
pub fn remove(device: DeviceId, pkg: &str) -> Result<usize, String> {
    let found = existing(device, pkg);
    for path in &found {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(found.len())
}

/// Whether `text` (a shortcut's arguments or command line) runs exactly
/// `task`, not one that merely starts with it.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn runs_task(text: &str, task: &str) -> bool {
    text.split_whitespace().any(|word| word.trim_matches('"') == task)
}

/// Adds the shortcut for `pkg` (shown as `label`) on `device`; where it went.
pub fn add(data_dir: &Path, device: DeviceId, pkg: &str, label: &str) -> Result<PathBuf, String> {
    if !nectarlink_core::is_package_name(pkg) {
        return Err("not an app".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let icon = crate::icons::existing(data_dir, pkg);
    imp::add(data_dir, &exe, &task(&device, pkg), pkg, label, icon.as_deref())
}

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use crate::win::shortcut::{self, Shortcut};

    pub fn add(
        data_dir: &Path,
        exe: &Path,
        task: &str,
        pkg: &str,
        label: &str,
        icon: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let desktop = shortcut::desktop_dir().ok_or("no desktop folder")?;
        let path = desktop.join(format!("{}.lnk", super::file_stem(label, pkg)));
        let ico = icon.and_then(|png| write_ico(data_dir, pkg, png).ok());
        let link = Shortcut {
            target: exe.to_path_buf(),
            arguments: format!("{} {task}", crate::win::jumplist::ARG),
            description: label.to_owned(),
        };
        shortcut::write_with_icon(&path, &link, ico.as_deref()).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Desktop shortcuts that run `task`, whatever they were renamed to.
    pub fn existing(task: &str, _pkg: &str) -> Vec<PathBuf> {
        shortcut::desktop_dir().map(|desktop| existing_in(&desktop, task)).unwrap_or_default()
    }

    /// The shortcuts in `dir` that run `task`.
    pub fn existing_in(dir: &Path, task: &str) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
            .filter(|p| shortcut::arguments(p).is_some_and(|args| super::runs_task(&args, task)))
            .collect()
    }

    /// The `.ico` for `png`, written only when it's missing or older than
    /// the app's icon: the taskbar asks each time a window changes.
    pub(super) fn current_ico(data_dir: &Path, pkg: &str, png_path: &Path) -> Option<PathBuf> {
        let ico = data_dir.join("cache").join("shortcut-icons").join(format!("{pkg}.ico"));
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        match (modified(&ico), modified(png_path)) {
            (Some(ico_at), Some(png_at)) if ico_at >= png_at => Some(ico),
            _ => write_ico(data_dir, pkg, png_path).ok(),
        }
    }

    /// The app's PNG icon as a one-image `.ico` (an ICO may hold a PNG as
    /// is), which a shortcut needs.
    pub(super) fn write_ico(data_dir: &Path, pkg: &str, png_path: &Path) -> std::io::Result<PathBuf> {
        let png = std::fs::read(png_path)?;
        let ico = super::ico_from_png(&png)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a PNG"))?;
        let dir = data_dir.join("cache").join("shortcut-icons");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{pkg}.ico"));
        std::fs::write(&path, ico)?;
        Ok(path)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn add(
        _data_dir: &Path,
        exe: &Path,
        task: &str,
        pkg: &str,
        label: &str,
        icon: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let apps = dirs::data_dir().ok_or("no XDG data directory")?.join("applications");
        std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
        let path = apps.join(format!("nectarlink-app-{pkg}.desktop"));
        let launcher = crate::win::autostart::launcher(exe);
        let exec = crate::win::autostart::escape_desktop_exec_arg(&launcher.to_string_lossy());
        let single = |s: &str| s.replace(['\n', '\r'], " ");
        let icon_line = icon.map_or_else(|| "nectarlink".to_owned(), |p| single(&p.to_string_lossy()));
        let content = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{exec}\" {} {task}\nIcon={icon_line}\nTerminal=false\n",
            single(&super::file_stem(label, pkg)),
            crate::win::jumplist::ARG,
        );
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// The app menu entry for `pkg`, when it opens `task` (this phone's).
    pub fn existing(task: &str, pkg: &str) -> Vec<PathBuf> {
        let Some(apps) = dirs::data_dir().map(|d| d.join("applications")) else { return Vec::new() };
        let path = apps.join(format!("nectarlink-app-{pkg}.desktop"));
        let opens_task = std::fs::read_to_string(&path).is_ok_and(|text| {
            text.lines()
                .any(|line| line.strip_prefix("Exec=").is_some_and(|exec| super::runs_task(exec, task)))
        });
        if opens_task { vec![path] } else { Vec::new() }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn add(_: &Path, _: &Path, _: &str, _: &str, _: &str, _: Option<&Path>) -> Result<PathBuf, String> {
        Err("not on this system".into())
    }

    pub fn existing(_: &str, _: &str) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Wraps a PNG in a one-image ICO file. The size comes from the PNG's header;
/// 256 and up is written as 0, as ICO does.
#[cfg_attr(not(windows), allow(dead_code))]
fn ico_from_png(png: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !png.starts_with(SIGNATURE) || png.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    let side = |n: u32| if n >= 256 { 0 } else { n as u8 };
    let mut out = Vec::with_capacity(22 + png.len());
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    out.extend_from_slice(&[side(width), side(height), 0, 0]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&u32::try_from(png.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(png);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_round_trip_and_refuse_junk() {
        let device = DeviceId([7; 32]);
        let t = task(&device, "com.example.app");
        assert_eq!(parse_task(&t), Some((device, "com.example.app".to_owned())));
        assert_eq!(parse_task("open-app:nonsense"), None);
        assert_eq!(parse_task(&format!("{TASK_PREFIX}{device}:not a package")), None);
        assert_eq!(parse_task("send-files"), None);
    }

    #[test]
    fn a_shortcut_runs_its_own_task_only() {
        let device = DeviceId([7; 32]);
        let t = task(&device, "com.example.app");
        assert!(runs_task(&format!("--task {t}"), &t));
        assert!(runs_task(&format!("\"/opt/nectarlink\" --task \"{t}\""), &t));
        assert!(!runs_task(&format!("--task {t}.pro"), &t), "another app whose name starts the same");
        assert!(
            !runs_task(&format!("--task {}", task(&DeviceId([8; 32]), "com.example.app")), &t),
            "another phone"
        );
    }

    #[test]
    fn file_names_lose_what_windows_refuses() {
        assert_eq!(file_stem("Maps: Go/Now?", "com.maps"), "Maps Go Now");
        assert_eq!(file_stem("  ...  ", "com.x"), "com.x");
        assert_eq!(file_stem("Notes.", "com.notes"), "Notes");
    }

    #[test]
    fn an_ico_wraps_the_png_with_its_size() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&96u32.to_be_bytes());
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let ico = ico_from_png(&png).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 96);
        assert_eq!(ico[7], 0, "300 px is written as 0");
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize, png.len());
        assert_eq!(&ico[22..], &png[..]);
        assert!(ico_from_png(b"GIF89a").is_none());
    }

    /// Shortcuts are found by what they run, so renaming one on the desktop
    /// doesn't hide it from Remove (#370), and another app's stays.
    #[cfg(windows)]
    #[test]
    fn a_windows_shortcut_is_found_by_its_task() {
        use crate::win::shortcut::{self, Shortcut};
        let dir = std::env::temp_dir().join(format!("nl-shortcut-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let device = DeviceId([7; 32]);
        let exe = std::env::current_exe().unwrap();
        let mine = task(&device, "com.example.app");
        let other = task(&device, "com.example.other");
        for (name, t) in [("Renamed by me.lnk", &mine), ("Other.lnk", &other)] {
            let link = Shortcut {
                target: exe.clone(),
                arguments: format!("{} {t}", crate::win::jumplist::ARG),
                description: String::new(),
            };
            shortcut::write(&dir.join(name), &link).unwrap();
        }
        assert_eq!(imp::existing_in(&dir, &mine), vec![dir.join("Renamed by me.lnk")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real shortcut, in a temporary folder: it starts this app with the
    /// task, and carries the app's icon.
    #[cfg(windows)]
    #[test]
    fn a_windows_shortcut_opens_the_app_with_its_icon() {
        use crate::win::{image, shortcut};
        let dir = tempfile::tempdir().unwrap();
        let bitmap = image::Bitmap { width: 48, height: 48, bgra: vec![0x80; 48 * 48 * 4] };
        let ico = ico_from_png(&image::encode_png(&bitmap).unwrap()).unwrap();
        let ico_path = dir.path().join("app.ico");
        std::fs::write(&ico_path, ico).unwrap();
        let exe = std::env::current_exe().unwrap();
        let device = DeviceId([7; 32]);
        let link_path = dir.path().join("Maps.lnk");
        let link = shortcut::Shortcut {
            target: exe.clone(),
            arguments: format!("{} {}", crate::win::jumplist::ARG, task(&device, "com.maps")),
            description: "Maps".into(),
        };
        shortcut::write_with_icon(&link_path, &link, Some(&ico_path)).unwrap();
        assert_eq!(shortcut::target(&link_path).as_deref(), Some(exe.as_path()));
        let args = shortcut::arguments(&link_path).unwrap();
        let action = args.strip_prefix("--task ").unwrap();
        assert_eq!(parse_task(action), Some((device, "com.maps".to_owned())));
    }
}
