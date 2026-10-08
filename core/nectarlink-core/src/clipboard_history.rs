// SPDX-License-Identifier: MPL-2.0
//! Encrypted-at-rest history of the last 50 clipboard items (text and images)
//! exchanged between paired devices (`docs/PLAN.md` §3.4).
//!
//! Sensitive clips and one-time codes (OTPs) are never stored. Everything is
//! sealed on disk using the platform [`KeyProtector`] (Windows DPAPI or Android
//! Keystore AES-GCM).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{Error, identity::KeyProtector, otp};

/// Maximum number of clips retained in history.
pub const MAX_CLIPBOARD_HISTORY: usize = 50;

/// Larger images (a big photo) aren't kept, and the oldest unpinned images
/// go once together they'd take more than this, so history stays small on disk.
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGES_TOTAL: u64 = 128 * 1024 * 1024;

const INDEX_FILE: &str = "clipboard_history.enc";
const IMAGES_DIR: &str = "clipboard_images";

/// Whether a clipboard history item is text or an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardItemKind {
    Text,
    Image,
}

/// One clip in the local clipboard history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardHistoryEntry {
    pub id: String,
    pub kind: ClipboardItemKind,
    /// The text for [`ClipboardItemKind::Text`], or empty for [`ClipboardItemKind::Image`].
    pub text: String,
    /// `"image/png"` or `"image/jpeg"` for [`ClipboardItemKind::Image`].
    pub mime: Option<String>,
    /// Name of the paired device it was sent to or received from.
    pub device_name: String,
    /// `true` when received from a peer; `false` when sent from this device.
    pub incoming: bool,
    /// Unix timestamp (seconds) when the clip was exchanged.
    pub timestamp: i64,
    /// Whether the user pinned this clip so it stays at the top and is not evicted.
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    #[serde(flatten)]
    entry: ClipboardHistoryEntry,
    #[serde(default)]
    image_hash: Option<u64>,
    /// The image's size in bytes (0 for text).
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredState {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    items: Vec<StoredEntry>,
}

fn default_enabled() -> bool {
    true
}

impl Default for StoredState {
    fn default() -> Self {
        Self { enabled: true, next_id: 1, items: Vec::new() }
    }
}

/// Thread-safe encrypted store for clipboard history.
pub struct ClipboardHistoryStore {
    data_dir: PathBuf,
    protector: Arc<dyn KeyProtector>,
    state: Mutex<StoredState>,
}

impl std::fmt::Debug for ClipboardHistoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardHistoryStore").field("data_dir", &self.data_dir).finish_non_exhaustive()
    }
}

impl ClipboardHistoryStore {
    /// Opens the clipboard history store in `data_dir`, decrypting any existing
    /// history with `protector`.
    pub fn open(data_dir: &Path, protector: Arc<dyn KeyProtector>) -> Self {
        let index_path = data_dir.join(INDEX_FILE);
        let state = match fs::read(&index_path) {
            Ok(sealed) => protector
                .unprotect(&sealed)
                .ok()
                .and_then(|plain| serde_json::from_slice::<StoredState>(&plain).ok())
                .unwrap_or_default(),
            Err(_) => StoredState::default(),
        };
        Self { data_dir: data_dir.to_path_buf(), protector, state: Mutex::new(state) }
    }

    /// Whether clipboard history recording is enabled.
    pub fn enabled(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).enabled
    }

    /// Turns clipboard history recording on or off.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.enabled = enabled;
        self.save_locked(&s)
    }

    /// Records a text clip that went between devices. Returns `false` if
    /// history is disabled, the text is empty, or the text is/contains an OTP.
    pub fn record_text(&self, text: &str, device_name: &str, incoming: bool) -> bool {
        self.record_text_with_id(text, device_name, incoming).is_some()
    }

    /// Records a text clip and returns `(clip_id, evicted_ids)` if kept.
    pub fn record_text_with_id(
        &self,
        text: &str,
        device_name: &str,
        incoming: bool,
    ) -> Option<(String, Vec<String>)> {
        if text.trim().is_empty() || otp::is_otp_clip(text) {
            return None;
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.enabled {
            return None;
        }
        let now = now_unix();
        if let Some(existing) =
            s.items.iter_mut().find(|i| i.entry.kind == ClipboardItemKind::Text && i.entry.text == text)
        {
            let id = existing.entry.id.clone();
            existing.entry.device_name = device_name.to_owned();
            existing.entry.incoming = incoming;
            existing.entry.timestamp = now;
            sort_items(&mut s.items);
            let _ = self.save_locked(&s);
            return Some((id, Vec::new()));
        }

        let id = alloc_id(&mut s.next_id);
        s.items.push(StoredEntry {
            entry: ClipboardHistoryEntry {
                id: id.clone(),
                kind: ClipboardItemKind::Text,
                text: text.to_owned(),
                mime: None,
                device_name: device_name.to_owned(),
                incoming,
                timestamp: now,
                pinned: false,
            },
            image_hash: None,
            size: 0,
        });
        sort_items(&mut s.items);
        let evicted = self.evict_locked(&mut s.items);
        let _ = self.save_locked(&s);
        Some((id, evicted))
    }

    /// Records an image clip that went between devices. Returns `false` if
    /// history is disabled, the image is too large to keep, or it cannot be
    /// encrypted to disk.
    pub fn record_image(&self, mime: &str, bytes: &[u8], device_name: &str, incoming: bool) -> bool {
        self.record_image_with_id(mime, bytes, device_name, incoming).is_some()
    }

    /// Records an image clip and returns `(clip_id, evicted_ids)` if kept.
    pub fn record_image_with_id(
        &self,
        mime: &str,
        bytes: &[u8],
        device_name: &str,
        incoming: bool,
    ) -> Option<(String, Vec<String>)> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.enabled {
            return None;
        }
        let hash = fnv1a(bytes);
        let now = now_unix();
        if let Some(existing) = s.items.iter_mut().find(|i| {
            i.entry.kind == ClipboardItemKind::Image
                && i.image_hash == Some(hash)
                && i.entry.mime.as_deref() == Some(mime)
        }) {
            let existing_id = existing.entry.id.clone();
            existing.entry.device_name = device_name.to_owned();
            existing.entry.incoming = incoming;
            existing.entry.timestamp = now;
            if !self.image_path(&existing_id).is_file()
                && self.write_encrypted_image(&existing_id, bytes).is_err()
            {
                return None;
            }
            sort_items(&mut s.items);
            let _ = self.save_locked(&s);
            return Some((existing_id, Vec::new()));
        }

        let id = alloc_id(&mut s.next_id);
        if self.write_encrypted_image(&id, bytes).is_err() {
            return None;
        }
        s.items.push(StoredEntry {
            entry: ClipboardHistoryEntry {
                id: id.clone(),
                kind: ClipboardItemKind::Image,
                text: String::new(),
                mime: Some(mime.to_owned()),
                device_name: device_name.to_owned(),
                incoming,
                timestamp: now,
                pinned: false,
            },
            image_hash: Some(hash),
            size: bytes.len() as u64,
        });
        sort_items(&mut s.items);
        let evicted = self.evict_locked(&mut s.items);
        let _ = self.save_locked(&s);
        Some((id, evicted))
    }

    /// Lists clipboard history items (pinned first, then newest first),
    /// optionally filtered by `query`.
    pub fn list(&self, query: Option<&str>) -> Vec<ClipboardHistoryEntry> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let q = query.map(str::trim).filter(|q| !q.is_empty()).map(str::to_lowercase);
        s.items
            .iter()
            .filter(|i| match &q {
                None => true,
                Some(q) => {
                    i.entry.text.to_lowercase().contains(q)
                        || i.entry.device_name.to_lowercase().contains(q)
                        || (i.entry.kind == ClipboardItemKind::Image
                            && ("image".contains(q.as_str())
                                || "picture".contains(q.as_str())
                                || "photo".contains(q.as_str())
                                || i.entry.mime.as_deref().is_some_and(|m| m.to_lowercase().contains(q))))
                }
            })
            .map(|i| i.entry.clone())
            .collect()
    }

    /// Returns one entry by ID.
    pub fn entry(&self, id: &str) -> Option<ClipboardHistoryEntry> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.items.iter().find(|i| i.entry.id == id).map(|i| i.entry.clone())
    }

    /// Decrypts and returns `(mime, bytes)` for an image entry.
    pub fn image_bytes(&self, id: &str) -> Option<(String, Vec<u8>)> {
        let mime = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let item = s.items.iter().find(|i| i.entry.id == id)?;
            if item.entry.kind != ClipboardItemKind::Image {
                return None;
            }
            item.entry.mime.clone().unwrap_or_else(|| "image/png".to_owned())
        };
        let sealed = fs::read(self.image_path(id)).ok()?;
        let plain = self.protector.unprotect(&sealed).ok()?;
        Some((mime, plain))
    }

    /// Pins or unpins an entry by ID.
    pub fn set_pinned(&self, id: &str, pinned: bool) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(item) = s.items.iter_mut().find(|i| i.entry.id == id) else {
            return false;
        };
        if item.entry.pinned == pinned {
            return false;
        }
        item.entry.pinned = pinned;
        sort_items(&mut s.items);
        let _ = self.save_locked(&s);
        true
    }

    /// Deletes one entry by ID, removing its encrypted image file from disk if any.
    pub fn delete(&self, id: &str) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pos) = s.items.iter().position(|i| i.entry.id == id) else {
            return false;
        };
        let removed = s.items.remove(pos);
        if removed.entry.kind == ClipboardItemKind::Image {
            let _ = fs::remove_file(self.image_path(&removed.entry.id));
        }
        let _ = self.save_locked(&s);
        true
    }

    /// Clears the entire clipboard history and deletes the encrypted files from disk.
    pub fn clear(&self) -> Result<(), Error> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.items.clear();
        let images_dir = self.data_dir.join(IMAGES_DIR);
        if images_dir.exists() {
            let _ = fs::remove_dir_all(&images_dir);
        }
        let index_path = self.data_dir.join(INDEX_FILE);
        if s.enabled {
            if index_path.exists() {
                fs::remove_file(&index_path)?;
            }
            Ok(())
        } else {
            self.save_locked(&s)
        }
    }

    fn image_path(&self, id: &str) -> PathBuf {
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        self.data_dir.join(IMAGES_DIR).join(format!("{safe}.enc"))
    }

    fn write_encrypted_image(&self, id: &str, bytes: &[u8]) -> Result<(), Error> {
        let sealed = self.protector.protect(bytes)?;
        let dir = self.data_dir.join(IMAGES_DIR);
        fs::create_dir_all(&dir)?;
        let path = self.image_path(id);
        let tmp = path.with_extension("enc.tmp");
        fs::write(&tmp, &sealed)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn evict_locked(&self, items: &mut Vec<StoredEntry>) -> Vec<String> {
        let mut evicted_ids = Vec::new();
        while items.len() > MAX_CLIPBOARD_HISTORY {
            // Evict the oldest unpinned item (at the end of `items` after `sort_items`),
            // or the last item if all are pinned.
            let pos = items.iter().rposition(|i| !i.entry.pinned).unwrap_or(items.len() - 1);
            let evicted = items.remove(pos);
            if evicted.entry.kind == ClipboardItemKind::Image {
                let _ = fs::remove_file(self.image_path(&evicted.entry.id));
            }
            evicted_ids.push(evicted.entry.id);
        }
        // Then the oldest unpinned images, while together they're too large.
        while items.iter().map(|i| i.size).sum::<u64>() > MAX_IMAGES_TOTAL {
            let Some(pos) =
                items.iter().rposition(|i| !i.entry.pinned && i.entry.kind == ClipboardItemKind::Image)
            else {
                break;
            };
            let evicted = items.remove(pos);
            let _ = fs::remove_file(self.image_path(&evicted.entry.id));
            evicted_ids.push(evicted.entry.id);
        }
        evicted_ids
    }

    fn save_locked(&self, state: &StoredState) -> Result<(), Error> {
        fs::create_dir_all(&self.data_dir)?;
        let plain = serde_json::to_vec(state).map_err(|e| Error::Internal(e.to_string()))?;
        let sealed = self.protector.protect(&plain)?;
        let path = self.data_dir.join(INDEX_FILE);
        let tmp = path.with_extension("enc.tmp");
        fs::write(&tmp, &sealed)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

fn sort_items(items: &mut [StoredEntry]) {
    items.sort_by(|a, b| {
        b.entry
            .pinned
            .cmp(&a.entry.pinned)
            .then_with(|| b.entry.timestamp.cmp(&a.entry.timestamp))
            .then_with(|| b.entry.id.cmp(&a.entry.id))
    });
}

fn alloc_id(next_id: &mut u64) -> String {
    let id = *next_id;
    *next_id = next_id.saturating_add(1);
    format!("clip_{id:08}")
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PlainKeyProtector;

    /// A test key protector that XORs with a non-zero key so we can verify
    /// plaintext never appears on disk.
    struct XorKeyProtector;

    impl KeyProtector for XorKeyProtector {
        fn id(&self) -> u8 {
            99
        }
        fn protect(&self, plaintext: &[u8]) -> std::io::Result<Vec<u8>> {
            Ok(plaintext.iter().map(|b| b ^ 0xA5).collect())
        }
        fn unprotect(&self, sealed: &[u8]) -> std::io::Result<Vec<u8>> {
            Ok(sealed.iter().map(|b| b ^ 0xA5).collect())
        }
    }

    #[test]
    fn stores_encrypted_at_rest_and_excludes_otps() {
        let dir = tempfile::tempdir().unwrap();
        let store = ClipboardHistoryStore::open(dir.path(), Arc::new(XorKeyProtector));

        // OTPs (both full messages and bare codes) are never stored.
        assert!(!store.record_text("Your verification code is 482913", "Pixel", true));
        assert!(!store.record_text("482913", "Pixel", true));
        assert!(!store.record_text("G-482913", "Pixel", true));
        assert!(store.list(None).is_empty());

        // Normal text and images are stored encrypted.
        assert!(store.record_text("secret meeting notes", "Pixel", true));
        let png = b"\x89PNG\r\n\x1a\nfake-image-payload";
        assert!(store.record_image("image/png", png, "Desktop", false));

        // Verify disk files do not contain plaintext.
        let raw_index = fs::read(dir.path().join(INDEX_FILE)).unwrap();
        assert!(
            !String::from_utf8_lossy(&raw_index).contains("secret meeting notes"),
            "index must be encrypted on disk"
        );
        let items = store.list(None);
        assert_eq!(items.len(), 2);
        let img_item = items.iter().find(|i| i.kind == ClipboardItemKind::Image).unwrap();
        let raw_img = fs::read(store.image_path(&img_item.id)).unwrap();
        assert_ne!(raw_img, png, "image must be encrypted on disk");

        // Re-opening decrypts both text and image.
        let reopened = ClipboardHistoryStore::open(dir.path(), Arc::new(XorKeyProtector));
        assert_eq!(reopened.list(None).len(), 2);
        let (mime, decrypted) = reopened.image_bytes(&img_item.id).unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(decrypted, png);
    }

    #[test]
    fn caps_at_50_keeps_pinned_and_clears_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = ClipboardHistoryStore::open(dir.path(), Arc::new(PlainKeyProtector));

        for i in 0..55 {
            assert!(store.record_text(&format!("clip number {i}"), "Pixel", true));
            if i == 0 {
                let first_id = store.list(None).first().unwrap().id.clone();
                assert!(store.set_pinned(&first_id, true));
            }
        }

        let all = store.list(None);
        assert_eq!(all.len(), MAX_CLIPBOARD_HISTORY);
        assert_eq!(all[0].text, "clip number 0", "pinned clip stays at top and is not evicted");
        assert!(all[0].pinned);

        // Search filters by text or device name.
        let filtered = store.list(Some("number 54"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].text, "clip number 54");

        // Delete removes one item.
        assert!(store.delete(&filtered[0].id));
        assert_eq!(store.list(None).len(), MAX_CLIPBOARD_HISTORY - 1);

        // Clear really deletes from disk.
        store.clear().unwrap();
        assert!(store.list(None).is_empty());
        assert!(!dir.path().join(INDEX_FILE).exists());
        assert!(!dir.path().join(IMAGES_DIR).exists());

        // Images too large to keep are skipped; many large ones keep only
        // the newest that fit.
        assert!(!store.record_image("image/png", &vec![1u8; MAX_IMAGE_BYTES + 1], "Pixel", true));
        let big = MAX_IMAGE_BYTES;
        for i in 0..10u8 {
            let mut image = vec![i; big];
            image[0] = i;
            assert!(store.record_image("image/png", &image, "Pixel", true));
        }
        let kept = store.list(None);
        assert_eq!(kept.len() as u64, MAX_IMAGES_TOTAL / big as u64);
        assert_eq!(fs::read_dir(dir.path().join(IMAGES_DIR)).unwrap().count(), kept.len());
        store.clear().unwrap();

        // Turning off stops recording.
        store.set_enabled(false).unwrap();
        assert!(!store.enabled());
        assert!(!store.record_text("should not record", "Pixel", false));
        assert!(store.list(None).is_empty());
    }
}
