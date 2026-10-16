// SPDX-License-Identifier: GPL-3.0-or-later
//! Your PC in Your Pocket (§3.11 P2): exposes the PC's `Downloads`,
//! `Documents`, `Desktop`, and `Pictures` folders to paired phones over the
//! NectarLink storage service when `share_pc_storage` is enabled in Settings.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{FolderStorage, StorageEntry, StorageError, StorageReadFile, StorageWriteDone};

use crate::settings;

/// Virtual folder names exposed at the PC storage root.
#[cfg(test)]
pub const VIRTUAL_ROOTS: &[&str] = &["Downloads", "Documents", "Desktop", "Pictures"];

/// PC storage provider backed by an explicit list of named root directories.
#[derive(Debug, Clone)]
pub struct PcStorage {
    roots: Vec<(String, PathBuf)>,
}

impl PcStorage {
    /// Creates a [`PcStorage`] backed by the current Windows user's
    /// `Downloads`, `Documents`, `Desktop`, and `Pictures` folders.
    pub fn system() -> Self {
        let mut roots = Vec::with_capacity(4);
        if let Some(p) = dirs::download_dir() {
            roots.push(("Downloads".to_owned(), p));
        }
        if let Some(p) = dirs::document_dir() {
            roots.push(("Documents".to_owned(), p));
        }
        if let Some(p) = dirs::desktop_dir() {
            roots.push(("Desktop".to_owned(), p));
        }
        if let Some(p) = dirs::picture_dir() {
            roots.push(("Pictures".to_owned(), p));
        }
        Self { roots }
    }

    /// Creates a [`PcStorage`] with custom named root directories (used by unit tests).
    #[cfg(test)]
    pub fn with_roots(roots: Vec<(String, PathBuf)>) -> Self {
        Self { roots }
    }

    fn split_virtual_path<'a>(&self, raw: &'a str) -> Result<Option<(&Path, &'a str)>, StorageError> {
        if raw.contains('\\') || raw.contains(':') || raw.chars().any(char::is_control) {
            return Err(StorageError::Denied);
        }
        let trimmed = raw.trim_start_matches('/');
        if trimmed.is_empty() {
            return Ok(None);
        }
        if trimmed.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return Err(StorageError::Denied);
        }
        let (top, rest) = match trimmed.find('/') {
            Some(idx) => (&trimmed[..idx], &trimmed[idx + 1..]),
            None => (trimmed, ""),
        };
        let Some((_, root_path)) = self.roots.iter().find(|(name, _)| name == top) else {
            return Err(StorageError::NotFound);
        };
        Ok(Some((root_path.as_path(), rest)))
    }

    /// Resolves a subpath like `"Downloads/report.pdf"` to a canonical path
    /// inside one of the allowed roots, rejecting any `..` traversal or escape.
    #[cfg(test)]
    pub fn resolve_subpath(&self, raw: &str) -> Result<PathBuf, StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(raw)? else {
            return Err(StorageError::Invalid("virtual root has no single filesystem path".into()));
        };
        FolderStorage::new(root_path).resolve_existing(rest, true)
    }

    /// Lists `""` / `"/"` (the 4 virtual user folders) or a subfolder within one of them.
    pub fn list(&self, path: &str) -> Result<Vec<StorageEntry>, StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(path)? else {
            let mut out = Vec::with_capacity(self.roots.len());
            for (name, dir) in &self.roots {
                if let Ok(md) = std::fs::metadata(dir)
                    && md.is_dir()
                {
                    let modified = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .and_then(|d| i64::try_from(d.as_millis()).ok())
                        .unwrap_or_else(now_ms);
                    out.push(StorageEntry { name: name.clone(), size: 0, modified, is_dir: true });
                }
            }
            return Ok(out);
        };
        FolderStorage::new(root_path).list(rest)
    }

    pub fn open_read(&self, path: &str) -> Result<StorageReadFile, StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(path)? else {
            return Err(StorageError::Denied);
        };
        if rest.is_empty() {
            return Err(StorageError::Denied);
        }
        FolderStorage::new(root_path).open_read(rest)
    }

    pub fn write(
        &self,
        path: &str,
        staged: &Path,
        modified: Option<i64>,
    ) -> Result<StorageWriteDone, StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(path)? else {
            return Err(StorageError::Denied);
        };
        if rest.is_empty() {
            return Err(StorageError::Denied);
        }
        FolderStorage::new(root_path).write(rest, staged, modified)
    }

    pub fn mkdir(&self, path: &str) -> Result<(), StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(path)? else {
            return Err(StorageError::Denied);
        };
        if rest.is_empty() {
            return Err(StorageError::Denied);
        }
        FolderStorage::new(root_path).mkdir(rest)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let Some((from_root, from_rest)) = self.split_virtual_path(from)? else {
            return Err(StorageError::Denied);
        };
        let Some((to_root, to_rest)) = self.split_virtual_path(to)? else {
            return Err(StorageError::Denied);
        };
        if from_rest.is_empty() || to_rest.is_empty() || from_root != to_root {
            return Err(StorageError::Denied);
        }
        FolderStorage::new(from_root).rename(from_rest, to_rest)
    }

    pub fn delete(&self, path: &str, confirmed: bool) -> Result<(), StorageError> {
        let Some((root_path, rest)) = self.split_virtual_path(path)? else {
            return Err(StorageError::Denied);
        };
        if rest.is_empty() {
            return Err(StorageError::Denied);
        }
        FolderStorage::new(root_path).delete(rest, confirmed)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn any_peer_storage_toggle_enabled() -> bool {
    let Some(node) = crate::core_host::node() else {
        return false;
    };
    let Ok(devices) = node.paired_devices() else {
        return false;
    };
    devices.into_iter().any(|d| {
        node.device_toggles(d.id)
            .ok()
            .is_some_and(|toggles| toggles.into_iter().any(|(k, on)| k == "storage" && on))
    })
}

fn ensure_enabled() -> Result<PcStorage, StorageError> {
    if !settings::share_pc_storage() && !any_peer_storage_toggle_enabled() {
        return Err(StorageError::Denied);
    }
    Ok(PcStorage::system())
}

pub fn storage_list(path: &str) -> Result<Vec<StorageEntry>, StorageError> {
    ensure_enabled()?.list(path)
}

pub fn storage_open_read(path: &str) -> Result<StorageReadFile, StorageError> {
    ensure_enabled()?.open_read(path)
}

pub fn storage_write(
    path: &str,
    staged: &Path,
    modified: Option<i64>,
) -> Result<StorageWriteDone, StorageError> {
    ensure_enabled()?.write(path, staged, modified)
}

pub fn storage_mkdir(path: &str) -> Result<(), StorageError> {
    ensure_enabled()?.mkdir(path)
}

pub fn storage_rename(from: &str, to: &str) -> Result<(), StorageError> {
    ensure_enabled()?.rename(from, to)
}

pub fn storage_delete(path: &str, confirmed: bool) -> Result<(), StorageError> {
    ensure_enabled()?.delete(path, confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_storage() -> (tempfile::TempDir, PcStorage) {
        let tmp = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for &name in VIRTUAL_ROOTS {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            roots.push((name.to_owned(), dir));
        }
        (tmp, PcStorage::with_roots(roots))
    }

    #[test]
    fn virtual_root_lists_four_user_folders() {
        let (_tmp, storage) = test_storage();
        let entries = storage.list("").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Downloads", "Documents", "Desktop", "Pictures"]);
        assert!(entries.iter().all(|e| e.is_dir));

        let slash_entries = storage.list("/").unwrap();
        assert_eq!(slash_entries.len(), 4);
    }

    #[test]
    fn resolve_subpath_rejects_traversal_outside_allowed_roots() {
        let (tmp, storage) = test_storage();
        let report = tmp.path().join("Downloads").join("report.pdf");
        std::fs::write(&report, b"pdf-bytes").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), b"top-secret").unwrap();

        let resolved = storage.resolve_subpath("Downloads/report.pdf").unwrap();
        assert!(resolved.ends_with("report.pdf"));

        for bad in [
            "..",
            "../secret.txt",
            "Downloads/../secret.txt",
            "Downloads/../../Windows/System32",
            "Downloads\\report.pdf",
            "C:/Windows/System32",
        ] {
            assert!(storage.resolve_subpath(bad).is_err(), "expected error for {bad}");
            assert!(storage.list(bad).is_err(), "expected list error for {bad}");
            assert!(storage.open_read(bad).is_err(), "expected read error for {bad}");
        }
    }
}
