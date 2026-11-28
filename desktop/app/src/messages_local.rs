// SPDX-License-Identifier: GPL-3.0-or-later
//! What was done with conversations on this PC: deleted, marked unread. The
//! phone is never asked, so this is a layer over what the phone lists: the
//! conversations and messages are still on the phone. Kept by device in
//! `messages-local.json` in the data folder, and dropped with the phone.
//!
//! Dates are the phone's, from the conversation's latest message. A mark
//! applies up to the date it was made at, so a message that arrives later
//! shows again: a deleted conversation shows with only what is newer.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use nectarlink_core::DeviceId;
use serde::{Deserialize, Serialize};

use crate::core_host;

const FILE: &str = "messages-local.json";

/// What was done on one phone, by conversation ID (`chat:...` for the chats
/// read from notifications).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Local {
    /// Deleted: messages up to this date are hidden.
    cleared: BTreeMap<String, i64>,
    /// Messages deleted one by one: a text's ID, or a chat message's [`chat_message_key`].
    deleted: BTreeSet<String>,
    /// Conversations whose latest message was deleted: no snippet up to this date.
    blanked: BTreeMap<String, i64>,
    /// Marked unread, until opened.
    unread: BTreeSet<String>,
}

impl Local {
    fn is_empty(&self) -> bool {
        *self == Local::default()
    }

    /// Deletes `thread` (latest message `date`): what it has now is hidden.
    pub fn delete_thread(&mut self, thread: &str, date: i64) {
        let at = self.cleared.entry(thread.to_owned()).or_insert(date);
        *at = (*at).max(date);
        self.unread.remove(thread);
        self.blanked.remove(thread);
    }

    /// Whether `thread`, with its latest message from `date`, has nothing left to show.
    pub fn thread_hidden(&self, thread: &str, date: i64) -> bool {
        self.cleared.get(thread).is_some_and(|at| date <= *at)
    }

    /// Where a deleted conversation starts: messages up to here are hidden.
    pub fn cleared_at(&self, thread: &str) -> Option<i64> {
        self.cleared.get(thread).copied()
    }

    /// Deletes messages one by one. `last` is the conversation's latest message's
    /// date when it's among them, so the list stops showing its text.
    pub fn delete_messages<'a>(
        &mut self,
        thread: &str,
        keys: impl IntoIterator<Item = &'a str>,
        last: Option<i64>,
    ) {
        self.deleted.extend(keys.into_iter().map(str::to_owned));
        if let Some(date) = last {
            let at = self.blanked.entry(thread.to_owned()).or_insert(date);
            *at = (*at).max(date);
        }
    }

    /// Whether a message of `thread`, sent at `date`, was deleted.
    pub fn message_hidden(&self, thread: &str, key: &str, date: i64) -> bool {
        self.deleted.contains(key) || self.cleared.get(thread).is_some_and(|at| date <= *at)
    }

    /// Whether the list shows no text for `thread`, whose latest message is from `date`.
    pub fn snippet_hidden(&self, thread: &str, date: i64) -> bool {
        self.blanked.get(thread).is_some_and(|at| date <= *at)
    }

    pub fn mark_unread(&mut self, thread: &str) {
        self.unread.insert(thread.to_owned());
    }

    /// Opened: it's read. Whether it was marked.
    pub fn clear_unread(&mut self, thread: &str) -> bool {
        self.unread.remove(thread)
    }

    pub fn is_marked_unread(&self, thread: &str) -> bool {
        self.unread.contains(thread)
    }

    /// The conversations marked unread.
    pub fn marked_unread(&self) -> impl Iterator<Item = &str> {
        self.unread.iter().map(String::as_str)
    }

    /// Forgets what newer messages made stale, given the conversations as listed
    /// now (`(id, latest message date)`), so a deleted latest message stays
    /// blanked only until a newer one arrives. Whether anything changed.
    pub fn reconcile<'a>(&mut self, listed: impl IntoIterator<Item = (&'a str, i64)>) -> bool {
        let mut changed = false;
        for (thread, date) in listed {
            if self.blanked.get(thread).is_some_and(|at| date > *at) {
                self.blanked.remove(thread);
                changed = true;
            }
        }
        changed
    }
}

/// A chat message's key among the deleted: the same message read again from a
/// later notification (which repeats the conversation's recent messages) gets
/// a new row ID, but the same key. A fingerprint, so no text is kept.
pub fn chat_message_key(thread: &str, sender: &str, text: &str, time_ms: i64, self_sent: bool) -> String {
    let all = format!("{thread}\u{1}{sender}\u{1}{text}\u{1}{time_ms}\u{1}{}", u8::from(self_sent));
    format!("chat:{:016x}", crate::photos::fingerprint(&all))
}

/// Everything kept, by device.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    devices: BTreeMap<String, Local>,
}

impl Kept {
    fn load(dir: &Path) -> Kept {
        match fs::read(dir.join(FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "unreadable what was done with conversations; starting over");
                Kept::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Kept::default(),
            Err(e) => {
                tracing::warn!(error = %e, "can't read what was done with conversations");
                Kept::default()
            }
        }
    }

    /// Writes the whole file or nothing (a temporary file renamed over it).
    fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(FILE);
        if self.devices.is_empty() {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let partial = path.with_extension("json.tmp");
        let mut file = fs::File::create(&partial)?;
        file.write_all(&serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, path)
    }
}

static KEPT: Mutex<Option<(Option<PathBuf>, Kept)>> = Mutex::new(None);

/// Runs `f` on the kept state and its folder, loading it first on the first use.
fn kept<T>(f: impl FnOnce(&Option<PathBuf>, &mut Kept) -> T) -> T {
    let mut guard = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let (dir, kept) = guard.get_or_insert_with(|| {
        let dir = core_host::try_host().map(|h| h.data_dir.clone());
        let kept = dir.as_deref().map(Kept::load).unwrap_or_default();
        (dir, kept)
    });
    f(dir, kept)
}

/// What was done on `device`.
pub fn read(device: DeviceId) -> Local {
    kept(|_, k| k.devices.get(&device.to_string()).cloned().unwrap_or_default())
}

/// Changes what was done on `device`, and keeps it if `f` changed anything.
pub fn update<T>(device: DeviceId, f: impl FnOnce(&mut Local) -> T) -> T {
    kept(|dir, k| {
        let key = device.to_string();
        let local = k.devices.entry(key.clone()).or_default();
        let before = local.clone();
        let out = f(local);
        let changed = *local != before;
        if local.is_empty() {
            k.devices.remove(&key);
        }
        if changed {
            save(dir, k);
        }
        out
    })
}

/// A phone was unpaired: what was done to its conversations goes with it.
pub fn forget(device: DeviceId) {
    kept(|dir, k| {
        if k.devices.remove(&device.to_string()).is_some() {
            save(dir, k);
        }
    });
}

fn save(dir: &Option<PathBuf>, kept: &Kept) {
    if let Some(dir) = dir
        && let Err(e) = kept.save(dir)
    {
        tracing::warn!(error = %e, "can't save what was done with conversations");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_conversation_shows_again_with_only_what_is_newer() {
        let mut local = Local::default();
        local.delete_thread("7", 100);
        assert!(local.thread_hidden("7", 100), "what the phone still lists stays hidden");
        assert!(!local.thread_hidden("7", 101), "a new message shows it");
        assert!(local.message_hidden("7", "sms:1", 90));
        assert!(local.message_hidden("7", "sms:2", 100));
        assert!(!local.message_hidden("7", "sms:3", 101));
        assert!(!local.message_hidden("8", "sms:1", 90), "another conversation");
        // Deleting again later hides more, never less.
        local.delete_thread("7", 150);
        local.delete_thread("7", 120);
        assert_eq!(local.cleared_at("7"), Some(150));
    }

    #[test]
    fn deleting_a_conversation_drops_its_other_marks() {
        let mut local = Local::default();
        local.mark_unread("7");
        local.delete_thread("7", 100);
        assert!(!local.is_marked_unread("7"));
    }

    #[test]
    fn a_deleted_message_does_not_come_back() {
        let mut local = Local::default();
        local.delete_messages("7", ["sms:5"], None);
        assert!(local.message_hidden("7", "sms:5", 500));
        assert!(!local.message_hidden("7", "sms:6", 500));
        assert!(!local.snippet_hidden("7", 500), "it wasn't the latest");
        // The latest: the list shows no text for it, until a newer message.
        local.delete_messages("7", ["mms:9"], Some(600));
        assert!(local.snippet_hidden("7", 600));
        assert!(!local.snippet_hidden("7", 601));
        assert!(local.reconcile([("7", 601)]));
        assert!(!local.snippet_hidden("7", 600));
    }

    #[test]
    fn unread_stays_until_opened() {
        let mut local = Local::default();
        local.mark_unread("7");
        local.mark_unread("chat:a:b");
        assert!(local.is_marked_unread("7"));
        assert_eq!(local.marked_unread().collect::<Vec<_>>(), ["7", "chat:a:b"]);
        assert!(local.clear_unread("7"));
        assert!(!local.clear_unread("7"));
        assert!(!local.is_marked_unread("7"));
    }

    #[test]
    fn a_chat_message_has_the_same_key_each_time_it_is_read() {
        let key = chat_message_key("chat:a:b", "Maya", "Hi", 5, false);
        assert_eq!(key, chat_message_key("chat:a:b", "Maya", "Hi", 5, false));
        assert_ne!(key, chat_message_key("chat:a:b", "Maya", "Hi", 6, false));
        assert_ne!(key, chat_message_key("chat:a:b", "Maya", "Hi", 5, true));
        assert_ne!(key, chat_message_key("chat:a:c", "Maya", "Hi", 5, false));
        assert!(!key.contains("Hi"), "no text is kept");
    }

    #[test]
    fn what_was_done_survives_a_restart_and_goes_with_the_phone() {
        let dir = tempfile::tempdir().unwrap();
        let mut kept = Kept::default();
        let mut local = Local::default();
        local.delete_thread("8", 200);
        local.delete_messages("9", ["sms:1"], Some(50));
        local.mark_unread("10");
        kept.devices.insert("aa".into(), local.clone());
        kept.devices.insert("bb".into(), Local::default());
        kept.save(dir.path()).unwrap();

        let loaded = Kept::load(dir.path());
        assert_eq!(loaded.devices.get("aa"), Some(&local));
        assert!(loaded.devices.get("aa").unwrap().thread_hidden("8", 200));

        // The last phone gone: no file left.
        Kept::default().save(dir.path()).unwrap();
        assert!(!dir.path().join(FILE).exists());
        assert_eq!(Kept::load(dir.path()), Kept::default());
    }

    #[test]
    fn a_file_with_an_archived_field_from_before_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            br#"{"devices":{"aa":{"archived":{"7":100},"cleared":{"8":200},"unread":["10"]}}}"#,
        )
        .unwrap();
        let loaded = Kept::load(dir.path());
        let local = loaded.devices.get("aa").expect("the device is kept");
        assert!(local.thread_hidden("8", 200));
        assert!(local.is_marked_unread("10"));
    }

    #[test]
    fn a_missing_or_broken_file_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Kept::load(dir.path()), Kept::default());
        fs::write(dir.path().join(FILE), b"{ nope").unwrap();
        assert_eq!(Kept::load(dir.path()), Kept::default());
    }
}
