// SPDX-License-Identifier: GPL-3.0-or-later
// Linux equivalent of Jump List tasks via freedesktop.org `.desktop` Desktop Actions
// (`~/.local/share/applications/nectarlink.desktop` + hicolor app icon).
//
// Desktop environments (GNOME Dash, KDE Plasma Task Manager, Cinnamon, XFCE) expose
// `[Desktop Action <id>]` entries in the launcher right-click menu, providing full
// parity with Windows taskbar Jump List tasks.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The command-line flag Jump List tasks pass back to NectarLink.
pub const ARG: &str = "--task";

/// Jump-list tasks exposed on the launcher / dock right-click menu.
/// Kept in parity with the Windows Jump List (`win/jumplist.rs`) and
/// `bridge::app::NectarLinkApp::dispatch_jump_task`.
pub const TASKS: &[(&str, &str, &str)] = &[
    ("send-files", "Send files to phone", "Open the file picker to send files to your phone"),
    ("send-clipboard", "Send clipboard to phone", "Send what's currently copied on this PC to your phone"),
    ("find-phone", "Find my phone", "Make your paired phone ring"),
    ("take-photo", "Take photo with phone", "Capture a photo with your phone camera and paste it here"),
    ("open-timeline", "Open Timeline", "Open the chronological activity feed in NectarLink"),
];

fn applications_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("applications"))
}

fn hicolor_icon_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("icons/hicolor/256x256/apps/nectarlink.png"))
}

/// Register the application `.desktop` entry and 256x256 PNG icon in the user's XDG data dir.
///
/// Only installs into the user's real `~/.local/share/applications` when running as the
/// primary profile (`custom_data_dir.is_none()`), matching Windows Jump List registration.
pub fn register(custom_data_dir: Option<&Path>) {
    if custom_data_dir.is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(apps_dir) = applications_dir() else {
        return;
    };
    let icon_path = hicolor_icon_path().unwrap_or_else(|| PathBuf::from("nectarlink"));
    if icon_path.is_absolute() && !icon_path.exists() {
        let _ = super::icon::write_png(&icon_path, 128);
    }
    if let Err(e) = write_desktop_file_to(&apps_dir, &exe, &icon_path, custom_data_dir) {
        tracing::debug!(error = %e, "couldn't update the desktop launcher actions");
    }
}

/// Write `nectarlink.desktop` into `dir` pointing at `exe` and `icon_path`.
/// Extracted so unit tests can verify `.desktop` generation inside a temp directory.
pub fn write_desktop_file_to(
    dir: &Path,
    exe: &Path,
    icon_path: &Path,
    custom_data_dir: Option<&Path>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let desktop_path = dir.join("nectarlink.desktop");
    let exe_str = exe.to_string_lossy();
    let icon_str = icon_path.to_string_lossy();

    let actions_list: Vec<&str> = TASKS.iter().map(|(id, _, _)| *id).collect();
    let mut content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.1\n\
         Name=NectarLink\n\
         GenericName=Phone Companion\n\
         Comment=Connect your Android phone and Linux PC\n\
         Exec=\"{exe_str}\" %U\n\
         Icon={icon_str}\n\
         Terminal=false\n\
         Categories=Utility;FileTransfer;\n\
         MimeType=x-scheme-handler/nectarlink;\n\
         StartupWMClass=NectarLink\n\
         Actions={};\n",
        actions_list.join(";")
    );

    for &(action, title, _comment) in TASKS {
        let args = build_task_arguments(action, custom_data_dir);
        content.push_str(&format!(
            "\n[Desktop Action {action}]\n\
             Name={title}\n\
             Exec=\"{exe_str}\" {args}\n"
        ));
    }

    std::fs::write(&desktop_path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&desktop_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(desktop_path)
}

/// Remove the user-level `.desktop` file and icon installed by [`register`].
pub fn unregister() {
    if let Some(dir) = applications_dir() {
        let _ = std::fs::remove_file(dir.join("nectarlink.desktop"));
    }
    if let Some(icon) = hicolor_icon_path() {
        let _ = std::fs::remove_file(icon);
    }
}

/// Build the command-line argument string for a jump-list task.
/// Kept public so unit tests can verify `--data-dir` propagation without
/// touching the desktop environment.
pub fn build_task_arguments(action: &str, custom_data_dir: Option<&Path>) -> String {
    match custom_data_dir {
        Some(dir) => format!("--data-dir \"{}\" {ARG} {action}", dir.display()),
        None => format!("{ARG} {action}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_omits_data_dir() {
        assert_eq!(build_task_arguments("send-files", None), "--task send-files");
    }

    #[test]
    fn custom_profile_quotes_data_dir() {
        let p = Path::new("/home/Alice Smith/.local/share/NectarLink Dev");
        let args = build_task_arguments("find-phone", Some(p));
        assert_eq!(args, "--data-dir \"/home/Alice Smith/.local/share/NectarLink Dev\" --task find-phone");
    }

    #[test]
    fn all_defined_tasks_have_non_empty_metadata() {
        assert_eq!(TASKS.len(), 5);
        let ids: Vec<&str> = TASKS.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids, vec!["send-files", "send-clipboard", "find-phone", "take-photo", "open-timeline"]);
        for &(action, title, tip) in TASKS {
            assert!(!action.is_empty());
            assert!(!title.is_empty());
            assert!(!tip.is_empty());
        }
    }

    #[test]
    fn write_desktop_file_generates_valid_actions_and_sections() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exe = Path::new("/usr/bin/nectarlink-desktop");
        let icon = Path::new("/home/user/.local/share/icons/hicolor/256x256/apps/nectarlink.png");

        let desktop_path = write_desktop_file_to(tmp.path(), exe, icon, None).expect("write desktop file");
        assert!(desktop_path.exists());

        let text = std::fs::read_to_string(&desktop_path).expect("read desktop file");
        assert!(text.contains("[Desktop Entry]"));
        assert!(text.contains("MimeType=x-scheme-handler/nectarlink;"));
        assert!(text.contains("Actions=send-files;send-clipboard;find-phone;take-photo;open-timeline;"));
        assert!(text.contains(&format!("Icon={}", icon.display())));

        for &(action, title, _) in TASKS {
            assert!(text.contains(&format!("[Desktop Action {action}]")), "missing section for {action}");
            assert!(text.contains(&format!("Name={title}")), "missing title for {action}");
            assert!(
                text.contains(&format!("Exec=\"/usr/bin/nectarlink-desktop\" --task {action}")),
                "missing Exec for {action}"
            );
        }
    }
}
