// SPDX-License-Identifier: GPL-3.0-or-later
// Linux file-manager "Send to" integration and `.lnk`-compatible shortcut metadata.
//
// `crate::send_to` reconciles per-device `<Name>.lnk` files in `send_to_dir()`.
// On Linux we store those canonical `.lnk` metadata files inside
// `~/.local/share/nectarlink/send-to/` and mirror them into the three major Linux
// file managers:
//   1. GNOME Files (Nautilus): `~/.local/share/nautilus/scripts/Send to <Name> (NectarLink)`
//   2. KDE Dolphin (KIO ServiceMenus): `~/.local/share/kio/servicemenus/nectarlink-send-<id>.desktop`
//   3. Cinnamon Nemo / XFCE Thunar (File Manager Actions):
//      `~/.local/share/file-manager/actions/nectarlink-send-<id>.desktop`

#![allow(dead_code)]

use std::io::Result;
use std::path::{Path, PathBuf};

/// What a shortcut starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub target: PathBuf,
    pub arguments: String,
    /// The tooltip.
    pub description: String,
}

fn xdg_data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// Write a shortcut entry to `path`, and if `path` lives in the active `send_to_dir()`,
/// synchronize GNOME Files (Nautilus), KDE Dolphin, and Nemo/Thunar context-menu entries.
pub fn write(path: &Path, shortcut: &Shortcut) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let target = shortcut.target.to_string_lossy();

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=NectarLink\n\
         Comment={}\n\
         Exec=\"{}\" {}\n\
         Icon=nectarlink\n\
         Terminal=false\n\
         X-NectarLink-Target={}\n\
         X-NectarLink-Args={}\n\
         X-NectarLink-Comment={}\n",
        shortcut.description, target, shortcut.arguments, target, shortcut.arguments, shortcut.description,
    );
    std::fs::write(path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(path, perms);
        }
    }

    if let (Some(parent), Some(real_send_to)) = (path.parent(), send_to_dir())
        && parent == real_send_to
    {
        let _ = sync_file_manager_menus(parent);
    }

    Ok(())
}

/// The arguments a shortcut passes, or `None` if it can't be read.
pub fn arguments(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("X-NectarLink-Args=") {
            return Some(rest.to_string());
        }
    }
    None
}

/// Read a full [`Shortcut`] metadata record from `path`.
fn read_shortcut(path: &Path) -> Option<Shortcut> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut target = None;
    let mut args = None;
    let mut comment = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("X-NectarLink-Target=") {
            target = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("X-NectarLink-Args=") {
            args = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("X-NectarLink-Comment=") {
            comment = rest.to_string();
        }
    }
    Some(Shortcut { target: target?, arguments: args?, description: comment })
}

/// Resolve the user's Startup / Autostart folder (`~/.config/autostart`).
pub fn startup_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart"))
}

/// Resolve the user's canonical SendTo directory (`~/.local/share/nectarlink/send-to`).
/// Ensures the directory exists so `send_to::reconcile` can enumerate it cleanly.
pub fn send_to_dir() -> Option<PathBuf> {
    let dir = xdg_data_home()?.join("nectarlink/send-to");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Synchronize all NectarLink `.lnk` entries in `send_to_dir` into the user's
/// file-manager context menus (Nautilus scripts, KDE KIO ServiceMenus, Nemo/Thunar actions).
pub fn sync_file_manager_menus(send_to_dir: &Path) -> Result<()> {
    let Some(data_home) = xdg_data_home() else {
        return Ok(());
    };
    let entries = collect_shortcuts(send_to_dir)?;
    write_file_manager_integrations(&data_home, &entries)
}

fn collect_shortcuts(send_to_dir: &Path) -> Result<Vec<(PathBuf, Shortcut)>> {
    let mut out = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(send_to_dir) else {
        return Ok(out);
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            continue;
        }
        if let Some(sc) = read_shortcut(&path)
            && sc.arguments.starts_with("--send-to ")
        {
            out.push((path, sc));
        }
    }
    Ok(out)
}

fn extract_device_id(args: &str) -> String {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    for pair in tokens.windows(2) {
        if pair[0] == "--send-to" {
            let sanitized: String = pair[1]
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
                .collect();
            if !sanitized.is_empty() {
                return sanitized;
            }
        }
    }
    "default".to_string()
}

fn format_menu_label(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Phone").trim();
    let with_prefix = if stem.starts_with("Send to ") { stem.to_string() } else { format!("Send to {stem}") };
    if with_prefix.ends_with("(NectarLink)") || with_prefix.contains("(NectarLink ") {
        with_prefix
    } else {
        format!("{with_prefix} (NectarLink)")
    }
}

/// Write Nautilus, KDE Dolphin KIO ServiceMenu, and Nemo/Thunar file-manager action files
/// under `root_data_dir` (e.g. `~/.local/share`) and prune stale NectarLink entries.
pub fn write_file_manager_integrations(
    root_data_dir: &Path,
    shortcuts: &[(PathBuf, Shortcut)],
) -> Result<()> {
    let nautilus_dir = root_data_dir.join("nautilus/scripts");
    let kio_dir = root_data_dir.join("kio/servicemenus");
    let actions_dir = root_data_dir.join("file-manager/actions");

    std::fs::create_dir_all(&nautilus_dir)?;
    std::fs::create_dir_all(&kio_dir)?;
    std::fs::create_dir_all(&actions_dir)?;

    let mut desired_nautilus = std::collections::HashSet::new();
    let mut desired_desktop = std::collections::HashSet::new();

    for (lnk_path, sc) in shortcuts {
        let label = format_menu_label(lnk_path);
        let dev_id = extract_device_id(&sc.arguments);
        let desktop_filename = format!("nectarlink-send-{dev_id}.desktop");

        desired_nautilus.insert(label.clone());
        desired_desktop.insert(desktop_filename.clone());

        let target = sc.target.to_string_lossy();
        let args = &sc.arguments;

        // 1. GNOME Files (Nautilus) executable script
        let nautilus_path = nautilus_dir.join(&label);
        let script = format!(
            "#!/usr/bin/env sh\n\
             # Generated by NectarLink — {comment}\n\
             if [ -n \"${{NAUTILUS_SCRIPT_SELECTED_FILE_PATHS:-}}\" ]; then\n\
                 IFS='\n'\n\
                 set -f\n\
                 exec \"{target}\" {args} $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n\
             else\n\
                 exec \"{target}\" {args} \"$@\"\n\
             fi\n",
            comment = sc.description,
        );
        std::fs::write(&nautilus_path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&nautilus_path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&nautilus_path, perms);
            }
        }

        // 2. KDE Dolphin (KIO ServiceMenu)
        let kio_path = kio_dir.join(&desktop_filename);
        let kio_content = format!(
            "[Desktop Entry]\n\
             Type=Service\n\
             MimeType=all/allfiles;\n\
             Actions=NectarLinkSend_{dev_id};\n\
             X-KDE-Priority=TopLevel\n\
             \n\
             [Desktop Action NectarLinkSend_{dev_id}]\n\
             Name={label}\n\
             Icon=nectarlink\n\
             Exec=\"{target}\" {args} %F\n"
        );
        std::fs::write(&kio_path, kio_content)?;

        // 3. Nemo / Thunar (DES-EMA File Manager Action)
        let action_path = actions_dir.join(&desktop_filename);
        let action_content = format!(
            "[Desktop Entry]\n\
             Type=Action\n\
             Name={label}\n\
             Tooltip={comment}\n\
             Icon=nectarlink\n\
             Profiles=profile-zero;\n\
             \n\
             [X-Action-Profile profile-zero]\n\
             MimeTypes=all/allfiles;\n\
             Exec=\"{target}\" {args} %F\n\
             Name=Default profile\n",
            comment = sc.description,
        );
        std::fs::write(&action_path, action_content)?;
    }

    // Prune stale Nautilus scripts generated by NectarLink
    if let Ok(rd) = std::fs::read_dir(&nautilus_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                let is_nectarlink = name.ends_with("(NectarLink)") || name.contains("(NectarLink ");
                if is_nectarlink && !desired_nautilus.contains(name) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    // Prune stale KDE ServiceMenus and File Manager Actions prefixed with `nectarlink-send-`
    for dir in [&kio_dir, &actions_dir] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str())
                    && name.starts_with("nectarlink-send-")
                    && name.ends_with(".desktop")
                    && !desired_desktop.contains(name)
                {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Pixel 9 é.lnk");
        let shortcut = Shortcut {
            target: std::env::current_exe().unwrap(),
            arguments: r#"--send-to abc --data-dir "/tmp/some dir""#.into(),
            description: "Send to Pixel".into(),
        };
        write(&path, &shortcut).unwrap();
        assert_eq!(arguments(&path).as_deref(), Some(shortcut.arguments.as_str()));
        // Replacing works too.
        write(&path, &Shortcut { arguments: "--send-to def".into(), ..shortcut }).unwrap();
        assert_eq!(arguments(&path).as_deref(), Some("--send-to def"));
        assert_eq!(arguments(&dir.path().join("missing.lnk")), None);
    }

    #[test]
    fn file_manager_integrations_create_and_prune_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let send_to = tmp.path().join("nectarlink/send-to");
        std::fs::create_dir_all(&send_to).unwrap();

        let lnk1 = send_to.join("Pixel 9.lnk");
        let sc1 = Shortcut {
            target: PathBuf::from("/usr/bin/nectarlink-desktop"),
            arguments: "--send-to pixel9".to_string(),
            description: "Send to Pixel 9".to_string(),
        };
        let lnk2 = send_to.join("Galaxy S25 (NectarLink).lnk");
        let sc2 = Shortcut {
            target: PathBuf::from("/usr/bin/nectarlink-desktop"),
            arguments: "--send-to s25".to_string(),
            description: "Send to Galaxy S25".to_string(),
        };

        write_file_manager_integrations(tmp.path(), &[(lnk1.clone(), sc1.clone()), (lnk2.clone(), sc2)])
            .expect("write integrations");

        let nautilus_p1 = tmp.path().join("nautilus/scripts/Send to Pixel 9 (NectarLink)");
        let nautilus_p2 = tmp.path().join("nautilus/scripts/Send to Galaxy S25 (NectarLink)");
        let kio_p1 = tmp.path().join("kio/servicemenus/nectarlink-send-pixel9.desktop");
        let kio_p2 = tmp.path().join("kio/servicemenus/nectarlink-send-s25.desktop");
        let act_p1 = tmp.path().join("file-manager/actions/nectarlink-send-pixel9.desktop");

        assert!(nautilus_p1.exists());
        assert!(nautilus_p2.exists());
        assert!(kio_p1.exists());
        assert!(kio_p2.exists());
        assert!(act_p1.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&nautilus_p1).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0, "Nautilus script must be executable");
        }

        let kio_text = std::fs::read_to_string(&kio_p1).unwrap();
        assert!(kio_text.contains("Exec=\"/usr/bin/nectarlink-desktop\" --send-to pixel9 %F"));

        // Now prune Galaxy S25 and verify its files are removed while Pixel 9 remains.
        write_file_manager_integrations(tmp.path(), &[(lnk1, sc1)]).expect("prune integrations");
        assert!(nautilus_p1.exists());
        assert!(!nautilus_p2.exists());
        assert!(kio_p1.exists());
        assert!(!kio_p2.exists());
    }
}
