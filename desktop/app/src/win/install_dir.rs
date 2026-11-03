// SPDX-License-Identifier: GPL-3.0-or-later
//! Where setup installs the app (#102): Program Files by default, or a
//! folder the user picks.
//!
//! Updates run a task as SYSTEM that starts `nectarlink-desktop.exe` from the
//! install folder, and the installer runs it with administrator rights to
//! close the old copy. That's only safe where nobody but administrators can
//! change what's at that path. The installer locks the install folder (and
//! any folder it creates on the way) the way Program Files is locked, but a
//! folder above it that a normal user can rename lets them put their own
//! folder of the same name in its place. So setup checks every folder above
//! the install folder: when all of them are admin-only, updates stay silent;
//! otherwise there's no update task and updates ask for permission.

use std::path::{Component, Path, PathBuf, Prefix};

use windows::{
    Win32::{
        Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
            Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, GetAce, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
        Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW},
    },
    core::{HSTRING, PWSTR},
};

use crate::{i18n::tr, setup::Placement};

/// The folder the app goes into inside the one the user picks.
pub const FOLDER_NAME: &str = "Nectarlink";

/// Longest install folder accepted: the app's deepest file adds about 90
/// characters, and it has to stay under Windows' 260.
const MAX_LEN: usize = 150;

/// SYSTEM, Administrators and TrustedInstaller: the only owners, and the only
/// ones allowed to delete, rename or re-permission a folder above the app.
const TRUSTED: [&str; 3] =
    ["S-1-5-18", "S-1-5-32-544", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];

/// DELETE, WRITE_DAC, WRITE_OWNER, FILE_DELETE_CHILD and GENERIC_ALL: what
/// lets someone remove or rename a folder, or its children, or give
/// themselves that right.
const TAKES_OVER: u32 = 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x0000_0040 | 0x1000_0000;

const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const INHERIT_ONLY_ACE: u8 = 0x08;
const DRIVE_FIXED: u32 = 3;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

/// Program Files, where the app goes unless the user picks somewhere else.
pub fn default_dir() -> PathBuf {
    let program_files = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    program_files.join(FOLDER_NAME)
}

/// The install folder for a folder the user picked: a `Nectarlink` folder in
/// it, unless they picked one with that name.
pub fn target_dir(chosen: &Path) -> PathBuf {
    let named = chosen.file_name().is_some_and(|name| name.eq_ignore_ascii_case(FOLDER_NAME));
    if named { chosen.to_path_buf() } else { chosen.join(FOLDER_NAME) }
}

/// Checks a folder the user picked and says where the app would go, or, in
/// words for the user, why it can't go there.
pub fn check(chosen: &Path) -> Result<Placement, String> {
    let dir = target_dir(chosen);
    let Some(letter) = drive_letter(&dir) else {
        return Err(tr!("Setup", "Choose a folder on one of this PC's drives."));
    };
    if dir.as_os_str().len() > MAX_LEN {
        return Err(tr!("Setup", "That folder's path is too long. Choose one closer to the top of a drive."));
    }
    let root = format!("{letter}:\\");
    // SAFETY: a null-terminated path to a drive's root.
    if unsafe { GetDriveTypeW(&HSTRING::from(&root)) } != DRIVE_FIXED {
        return Err(tr!("Setup", "Choose a folder on a drive inside this PC, not a USB or network drive."));
    }
    let mut fs = [0u16; 32];
    // SAFETY: the buffer outlives the call, which writes at most its length.
    let named =
        unsafe { GetVolumeInformationW(&HSTRING::from(&root), None, None, None, None, Some(&mut fs)) };
    let fs = String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(fs.len())]);
    if named.is_err() || !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS")) {
        return Err(tr!(
            "Setup",
            "That drive can't keep the app's files safe (it isn't NTFS). Choose another drive."
        ));
    }
    if dir.ancestors().filter(|p| p.exists()).any(is_reparse_point) {
        return Err(tr!("Setup", "Setup can't use that folder. Choose another one."));
    }
    match std::fs::read_dir(&dir).map(|mut entries| entries.next().is_none()) {
        Ok(false) => {
            return Err(tr!(
                "Setup",
                "There's already a %1 folder there with files in it. Choose another place.",
                FOLDER_NAME
            ));
        }
        Ok(true) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) if dir.is_file() => {
            return Err(tr!("Setup", "There's a file named %1 there. Choose another place.", FOLDER_NAME));
        }
        Err(_) => return Err(tr!("Setup", "Setup can't use that folder. Choose another one.")),
    }
    Ok(Placement { admin_only: path_is_admin_only(&dir), dir })
}

/// The drive letter of an absolute `X:\...` path; `None` for anything else
/// (network shares, `\\?\` paths, relative paths).
fn drive_letter(path: &Path) -> Option<char> {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else { return None };
    let Prefix::Disk(letter) = prefix.kind() else { return None };
    matches!(parts.next(), Some(Component::RootDir)).then_some(char::from(letter).to_ascii_uppercase())
}

/// Whether only administrators can delete, rename or re-permission each
/// folder above `dir` that already exists (setup creates and locks the
/// missing ones itself), up to the drive's root.
fn path_is_admin_only(dir: &Path) -> bool {
    dir.ancestors().skip(1).filter(|folder| folder.exists()).all(|folder| match security(folder) {
        Some((owner, aces)) => admin_only(&owner, &aces),
        None => false,
    })
}

/// One entry of a folder's permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub sid: String,
    pub mask: u32,
}

/// Whether a folder with this owner and these permissions can only be taken
/// over by administrators. Deny entries are ignored (an allow could still be
/// reached another way), and any entry this doesn't understand counts against.
pub(crate) fn admin_only(owner: &str, aces: &[Ace]) -> bool {
    TRUSTED.contains(&owner)
        && aces.iter().all(|ace| match ace.kind {
            ACCESS_DENIED_ACE_TYPE => true,
            _ if ace.flags & INHERIT_ONLY_ACE != 0 => true,
            ACCESS_ALLOWED_ACE_TYPE => TRUSTED.contains(&ace.sid.as_str()) || ace.mask & TAKES_OVER == 0,
            _ => false,
        })
}

/// Whether `path` itself (without following it) is a junction or symbolic link.
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// A folder's owner and permissions, as SID strings. `None` when they can't
/// be read, when the entry is a junction or symbolic link, or when there are
/// no permissions at all (a null DACL lets everyone do everything).
fn security(folder: &Path) -> Option<(String, Vec<Ace>)> {
    if is_reparse_point(folder) {
        return None;
    }
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the out-pointers live across the call; `sd` owns what the
    // others point into and is freed below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            &HSTRING::from(folder.as_os_str()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let result = (|| {
        if dacl.is_null() {
            return None;
        }
        let owner = sid_string(owner)?;
        // SAFETY: `dacl` points to a valid ACL inside `sd`.
        let count = unsafe { (*dacl).AceCount };
        let mut aces = Vec::with_capacity(usize::from(count));
        for index in 0..u32::from(count) {
            let mut raw = std::ptr::null_mut();
            // SAFETY: `index` is below the ACL's count; `raw` points into `sd`.
            unsafe { GetAce(dacl, index, &mut raw) }.ok()?;
            // SAFETY: every ACE starts with a header.
            let header = unsafe { *(raw as *const ACE_HEADER) };
            let ace = if matches!(header.AceType, ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE) {
                // Allowed and denied entries share this layout: the SID
                // starts at `SidStart`.
                let entry = raw as *const ACCESS_ALLOWED_ACE;
                // SAFETY: the type says it has this layout.
                let (mask, sid) =
                    unsafe { ((*entry).Mask, PSID(std::ptr::addr_of!((*entry).SidStart) as *mut _)) };
                Ace { kind: header.AceType, flags: header.AceFlags, sid: sid_string(sid)?, mask }
            } else {
                Ace { kind: header.AceType, flags: header.AceFlags, sid: String::new(), mask: 0 }
            };
            aces.push(ace);
        }
        Some((owner, aces))
    })();
    // SAFETY: `sd` came from GetNamedSecurityInfoW, which allocates with LocalAlloc.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    result
}

fn sid_string(sid: PSID) -> Option<String> {
    let mut text = PWSTR::null();
    // SAFETY: `sid` is a valid SID; the string is freed below.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.ok()?;
    // SAFETY: the call returned a null-terminated string.
    let out = unsafe { text.to_string() }.ok();
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: &str = "S-1-5-32-545";
    const AUTHENTICATED: &str = "S-1-5-11";
    const SYSTEM: &str = "S-1-5-18";
    const ADMINS: &str = "S-1-5-32-544";
    const SOMEONE: &str = "S-1-5-21-1-2-3-1001";

    fn allow(sid: &str, mask: u32) -> Ace {
        Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0, sid: sid.into(), mask }
    }

    #[test]
    fn the_app_goes_in_its_own_folder() {
        assert_eq!(target_dir(Path::new(r"D:\")), Path::new(r"D:\Nectarlink"));
        assert_eq!(target_dir(Path::new(r"D:\Apps")), Path::new(r"D:\Apps\Nectarlink"));
        assert_eq!(target_dir(Path::new(r"D:\Apps\nectarlink")), Path::new(r"D:\Apps\nectarlink"));
        assert!(default_dir().ends_with(FOLDER_NAME));
    }

    #[test]
    fn only_local_drive_paths_have_a_letter() {
        assert_eq!(drive_letter(Path::new(r"d:\Apps\Nectarlink")), Some('D'));
        assert_eq!(drive_letter(Path::new(r"\\server\share\Nectarlink")), None);
        assert_eq!(drive_letter(Path::new(r"\\?\D:\Nectarlink")), None);
        assert_eq!(drive_letter(Path::new(r"D:Nectarlink")), None);
        assert_eq!(drive_letter(Path::new(r"Apps\Nectarlink")), None);
    }

    #[test]
    fn a_drive_root_and_program_files_are_admin_only() {
        // A fresh NTFS drive's root: everyone can add folders, nobody else can
        // remove them; the modify entry only applies to what's inside.
        let root = [
            allow(SYSTEM, 0x1F01FF),
            allow(ADMINS, 0x1F01FF),
            allow(USERS, 0x1200A9),
            allow(AUTHENTICATED, 0x0004),
            Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0x0B, sid: AUTHENTICATED.into(), mask: 0x1301BF },
            Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0x0B, sid: "S-1-3-0".into(), mask: 0x1000_0000 },
        ];
        assert!(admin_only(SYSTEM, &root));
        assert!(admin_only(TRUSTED[2], &[allow(TRUSTED[2], 0x1F01FF), allow(USERS, 0x1200A9)]));
    }

    #[test]
    fn a_folder_users_can_change_or_own_is_not() {
        // A folder made on a drive's root inherits modify (with delete) for
        // every signed-in user.
        assert!(!admin_only(ADMINS, &[allow(ADMINS, 0x1F01FF), allow(AUTHENTICATED, 0x1301BF)]));
        // One a user made is theirs, so they can change its permissions.
        assert!(!admin_only(SOMEONE, &[allow(ADMINS, 0x1F01FF)]));
        // Delete-child on a parent lets them rename the folder below it.
        assert!(!admin_only(SYSTEM, &[allow(USERS, 0x40)]));
        assert!(!admin_only(SYSTEM, &[allow(SOMEONE, 0x1000_0000)]));
        // An entry of a kind this doesn't read counts against.
        assert!(!admin_only(SYSTEM, &[Ace { kind: 9, flags: 0, sid: String::new(), mask: 0 }]));
        // A deny entry doesn't make it unsafe.
        assert!(admin_only(
            SYSTEM,
            &[Ace { kind: ACCESS_DENIED_ACE_TYPE, flags: 0, sid: USERS.into(), mask: 0x10000 }]
        ));
    }

    #[test]
    fn program_files_on_this_pc_is_admin_only() {
        // The real thing, read from this PC: Program Files must pass, and a
        // folder in the user's profile (theirs to change) must not.
        assert!(path_is_admin_only(&default_dir()));
        if let Some(home) = dirs::home_dir() {
            assert!(!path_is_admin_only(&home.join("Apps").join(FOLDER_NAME)));
        }
    }

    #[test]
    fn junctions_are_never_trusted_or_accepted() {
        let base = tempfile::tempdir().unwrap();
        let link = base.path().join("link");
        let pf = default_dir().parent().unwrap().to_path_buf();
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&pf)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(is_reparse_point(&link));
        assert!(security(&link).is_none());
        assert!(check(&link.join("Apps")).is_err());
        let _ = std::fs::remove_dir(&link);
    }
}
