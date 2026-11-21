// SPDX-License-Identifier: GPL-3.0-or-later
//! Shell shortcuts (.lnk files), and where Explorer's "Send to" menu lives.

use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile},
        UI::Shell::{
            FOLDERID_Desktop, FOLDERID_PublicDesktop, FOLDERID_Recent, FOLDERID_SendTo, IShellLinkW,
            KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
        },
    },
    core::{HSTRING, Interface},
};

use super::with_com;

/// What a shortcut starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub target: PathBuf,
    pub arguments: String,
    /// The tooltip.
    pub description: String,
}

/// Writes (or replaces) a shortcut at `path`, with the target's own icon.
pub fn write(path: &Path, shortcut: &Shortcut) -> windows::core::Result<()> {
    write_with_icon(path, shortcut, None)
}

/// [`write`], with the icon from `icon` (an `.ico`) when given.
pub fn write_with_icon(path: &Path, shortcut: &Shortcut, icon: Option<&Path>) -> windows::core::Result<()> {
    let icon = icon.map_or_else(|| shortcut.target.clone(), Path::to_path_buf);
    with_com(|| {
        // SAFETY: plain COM calls on a live ShellLink object; the strings
        // outlive the calls.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(shortcut.target.as_os_str()))?;
            link.SetArguments(&HSTRING::from(shortcut.arguments.as_str()))?;
            link.SetDescription(&HSTRING::from(shortcut.description.as_str()))?;
            link.SetIconLocation(&HSTRING::from(icon.as_os_str()), 0)?;
            link.cast::<IPersistFile>()?.Save(&HSTRING::from(path.as_os_str()), true)
        }
    })
}

/// The arguments a shortcut passes, or `None` if it can't be read.
pub fn arguments(path: &Path) -> Option<String> {
    with_com(|| {
        // SAFETY: as in `write`; the buffer is large enough for any
        // shortcut's arguments (INFOTIPSIZE is 1024).
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            link.cast::<IPersistFile>()
                .ok()?
                .Load(&HSTRING::from(path.as_os_str()), Default::default())
                .ok()?;
            let mut buffer = [0u16; 4096];
            link.GetArguments(&mut buffer).ok()?;
            let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            Some(String::from_utf16_lossy(&buffer[..len]))
        }
    })
}

/// What a shortcut points at, or `None` if it can't be read (or points at
/// no file, as a shortcut to a website does).
pub fn target(path: &Path) -> Option<PathBuf> {
    with_com(|| {
        // SAFETY: as in `arguments`; the buffer holds the longest path.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            link.cast::<IPersistFile>()
                .ok()?
                .Load(&HSTRING::from(path.as_os_str()), Default::default())
                .ok()?;
            let mut buffer = [0u16; 32768];
            link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
            let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
        }
    })
}

/// The user's "Send to" folder.
pub fn send_to_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_SendTo)
}

/// This user's desktop.
pub fn desktop_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop)
}

/// The desktop every user of the PC sees, where setup puts its shortcut.
pub fn public_desktop_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_PublicDesktop)
}

/// The user's Recent folder: a shortcut to each file opened lately.
pub fn recent_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Recent)
}

pub(crate) fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    // SAFETY: the returned string is freed with CoTaskMemFree as documented.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.as_ptr().cast()));
        result
    }
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
            arguments: r#"--send-to abc --data-dir "C:\some dir""#.into(),
            description: "Send to Pixel".into(),
        };
        write(&path, &shortcut).unwrap();
        assert_eq!(arguments(&path).as_deref(), Some(shortcut.arguments.as_str()));
        assert_eq!(target(&path), Some(shortcut.target.clone()));
        assert_eq!(target(&dir.path().join("missing.lnk")), None);
        // Replacing works too.
        write(&path, &Shortcut { arguments: "--send-to def".into(), ..shortcut }).unwrap();
        assert_eq!(arguments(&path).as_deref(), Some("--send-to def"));
        assert_eq!(arguments(&dir.path().join("missing.lnk")), None);
    }

    #[test]
    fn finds_the_send_to_folder() {
        let dir = send_to_dir().expect("every user has one");
        assert!(dir.ends_with("SendTo"), "{}", dir.display());
    }

    #[test]
    fn finds_the_recent_folder() {
        let dir = recent_dir().expect("every user has one");
        assert!(dir.ends_with("Recent"), "{}", dir.display());
    }
}
