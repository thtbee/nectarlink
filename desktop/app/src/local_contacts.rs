// SPDX-License-Identifier: GPL-3.0-or-later
//! The Mac's own contacts (Contacts.app), to use next to the phone's. The phone's contacts
//! are not copied here: they're asked of the phone when needed, and it resolves the name and
//! picture of each caller and conversation itself, so a contact added on the phone is there at
//! once. This is for the people only this computer knows, and it's off until asked for in
//! Settings; macOS asks permission once.
//!
//! Numbers are matched by their last digits ([`crate::messages::digits`]), so `+33 6 12 34
//! 56 78`, `06 12 34 56 78` and `06-12-34-56-78` are one number. A contact the phone also
//! has is one person (the phone's name wins); the others are added.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use nectarlink_core::Contact;
// Only the Mac reader and the tests build one.
#[cfg(any(test, target_os = "macos"))]
use nectarlink_core::ContactNumber;

use crate::{messages::digits, state::Changes};

/// Pictures bigger than this are left out: a thumbnail is a few KiB.
const MAX_PHOTO_BYTES: usize = 256 * 1024;
/// How many contacts are kept from all the sources.
const MAX_CONTACTS: usize = 50_000;
/// How old the list may be before opening the Calls or Messages page reads it again.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// A contact of this computer, with its picture saved as a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub contact: Contact,
    pub photo: Option<PathBuf>,
}

/// Who a number belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: String,
    pub photo: Option<PathBuf>,
}

/// A contact as shown: the phone's, this computer's, or one both have.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub contact: Contact,
    pub photo: Option<PathBuf>,
    /// `"phone"`, `"computer"` or `"both"`.
    pub source: &'static str,
}

#[derive(Default)]
struct State {
    system: bool,
    entries: Vec<Entry>,
    by_number: HashMap<String, usize>,
    loaded_at: Option<Instant>,
    /// Bumped by each reading, so a slow one that was replaced drops its result.
    generation: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// Sets whether the Mac's contacts are used, and reads them when that changed.
pub fn configure(system: bool) {
    let changed = state(|s| {
        let changed = s.system != system || s.loaded_at.is_none();
        s.system = system;
        changed
    });
    if changed {
        reload();
    }
}

/// Reads the sources again if it's been a while.
pub fn refresh_if_stale() {
    if state(|s| s.loaded_at.is_some_and(|at| at.elapsed() >= STALE_AFTER)) {
        reload();
    }
}

/// Reads the sources in the background, then tells the pages.
pub fn reload() {
    let (system, generation) = state(|s| {
        s.generation = s.generation.wrapping_add(1);
        (s.system, s.generation)
    });
    if !system {
        apply(generation, Vec::new());
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("local-contacts".into())
        .spawn(move || apply(generation, save_photos(read_system())));
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't read this computer's contacts");
    }
}

fn apply(generation: u64, entries: Vec<Entry>) {
    let changed = state(|s| {
        if s.generation != generation {
            return false;
        }
        let mut by_number = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            for n in &e.contact.numbers {
                let d = digits(&n.number);
                if !d.is_empty() {
                    by_number.entry(d).or_insert(i);
                }
            }
        }
        let changed = s.entries != entries;
        s.entries = entries;
        s.by_number = by_number;
        s.loaded_at = Some(Instant::now());
        changed
    });
    if changed {
        crate::core_host::host().hub.changed(Changes::CALLS | Changes::MESSAGES);
    }
}

/// The contacts read, for the pages that list them.
pub fn entries() -> Vec<Entry> {
    state(|s| s.entries.clone())
}

/// The contact with this number, if this computer has one.
pub fn find(number: &str) -> Option<Found> {
    let d = digits(number);
    if d.is_empty() {
        return None;
    }
    state(|s| {
        let e = &s.entries[*s.by_number.get(&d)?];
        Some(Found { name: e.contact.name.clone(), photo: e.photo.clone() })
    })
}

/// The phone's contacts and this computer's as one list: favorites first, then by name.
/// A contact of this computer that has a number the phone's list has is that same person:
/// the phone's name stays, and its picture is this computer's when the phone has none.
pub fn merge(phone: &[Contact], phone_photos: &HashMap<String, PathBuf>, local: &[Entry]) -> Vec<Merged> {
    let mut merged: Vec<Merged> = phone
        .iter()
        .map(|c| Merged { contact: c.clone(), photo: phone_photos.get(&c.id).cloned(), source: "phone" })
        .collect();
    let mut owner: HashMap<String, usize> = HashMap::new();
    for (i, m) in merged.iter().enumerate() {
        for n in &m.contact.numbers {
            let d = digits(&n.number);
            if !d.is_empty() {
                owner.entry(d).or_insert(i);
            }
        }
    }
    let phone_count = merged.len();
    for e in local {
        let shared = e.contact.numbers.iter().map(|n| digits(&n.number)).find_map(|d| owner.get(&d).copied());
        match shared {
            Some(i) if i < phone_count => {
                let m = &mut merged[i];
                if m.photo.is_none() {
                    m.photo = e.photo.clone();
                }
                m.source = "both";
            }
            // Two contacts of this computer with one number are one.
            Some(_) => {}
            None => {
                let at = merged.len();
                for n in &e.contact.numbers {
                    let d = digits(&n.number);
                    if !d.is_empty() {
                        owner.entry(d).or_insert(at);
                    }
                }
                merged.push(Merged {
                    contact: e.contact.clone(),
                    photo: e.photo.clone(),
                    source: "computer",
                });
            }
        }
    }
    merged.sort_by_cached_key(|m| (!m.contact.starred, m.contact.name.to_lowercase()));
    merged
}

// ---- Saving the pictures ----

/// Writes each contact's picture as a file, and keeps the contact without the bytes.
fn save_photos(read: Vec<(Contact, Option<Vec<u8>>)>) -> Vec<Entry> {
    let dir = crate::notifications::images_dir();
    let can_write = std::fs::create_dir_all(&dir).is_ok();
    let mut seen = HashSet::new();
    read.into_iter()
        .filter(|(c, _)| !c.numbers.is_empty() && !c.name.trim().is_empty() && seen.insert(c.id.clone()))
        .take(MAX_CONTACTS)
        .map(|(mut contact, photo)| {
            let path = photo.filter(|p| can_write && p.len() <= MAX_PHOTO_BYTES).and_then(|bytes| {
                // Not `contact-…`: those are the phone's, and go when the phone is unpaired.
                let path =
                    dir.join(format!("pccontact-{:016x}.jpg", crate::photos::fingerprint_bytes(&bytes)));
                (path.exists() || std::fs::write(&path, &bytes).is_ok()).then_some(path)
            });
            contact.photo = None;
            Entry { contact, photo: path }
        })
        .collect()
}

// ---- The system address book ----

#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // The shim in `cpp/mac_contacts.mm`.
fn read_system() -> Vec<(Contact, Option<Vec<u8>>)> {
    use std::ffi::{CStr, CString, c_char};

    unsafe extern "C" {
        fn nl_mac_contacts_access() -> i32;
        fn nl_mac_contacts_fetch(photo_dir: *const c_char) -> *mut c_char;
        fn nl_mac_contacts_free(json: *mut c_char);
    }

    match unsafe { nl_mac_contacts_access() } {
        1 => {}
        0 => {
            tracing::info!(
                "the Mac's contacts aren't allowed: turn them on in System Settings › Privacy & Security › Contacts"
            );
            return Vec::new();
        }
        _ => {
            tracing::info!("can't ask for the Mac's contacts from this build (no usage description)");
            return Vec::new();
        }
    }
    let photos = crate::notifications::images_dir().join("pccontact-tmp");
    let _ = std::fs::remove_dir_all(&photos);
    if std::fs::create_dir_all(&photos).is_err() {
        return Vec::new();
    }
    let Ok(dir) = CString::new(photos.to_string_lossy().into_owned()) else { return Vec::new() };
    let json = unsafe { nl_mac_contacts_fetch(dir.as_ptr()) };
    if json.is_null() {
        tracing::warn!("can't read the Mac's contacts");
        let _ = std::fs::remove_dir_all(&photos);
        return Vec::new();
    }
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    unsafe { nl_mac_contacts_free(json) };
    let out = parse_system_json(&text, &photos);
    let _ = std::fs::remove_dir_all(&photos);
    out
}

/// What the Mac's address book gave, with each picture read from where it was written.
#[cfg(target_os = "macos")]
fn parse_system_json(text: &str, photos: &std::path::Path) -> Vec<(Contact, Option<Vec<u8>>)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(text) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let name = item["name"].as_str()?.trim().to_owned();
            let numbers: Vec<ContactNumber> = item["numbers"]
                .as_array()?
                .iter()
                .filter_map(|n| {
                    let number = n["number"].as_str()?.trim().to_owned();
                    let label = n["label"].as_str().map(str::trim).filter(|l| !l.is_empty());
                    (!number.is_empty())
                        .then(|| ContactNumber { number, label: label.map(|l| l.to_lowercase()) })
                })
                .take(16)
                .collect();
            let photo = item["photo"]
                .as_str()
                .filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric()))
                .and_then(|f| std::fs::read(photos.join(f)).ok());
            Some((
                Contact {
                    id: format!("pc:{}", item["id"].as_str()?),
                    name,
                    numbers,
                    starred: false,
                    photo: None,
                },
                photo,
            ))
        })
        .collect()
}

/// Windows and Linux have no system address book to read yet.
#[cfg(not(target_os = "macos"))]
fn read_system() -> Vec<(Contact, Option<Vec<u8>>)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: &str, name: &str, numbers: &[&str]) -> Contact {
        Contact {
            id: id.into(),
            name: name.into(),
            numbers: numbers.iter().map(|n| ContactNumber { number: (*n).into(), label: None }).collect(),
            starred: false,
            photo: None,
        }
    }

    fn entry(id: &str, name: &str, numbers: &[&str], photo: Option<&str>) -> Entry {
        Entry { contact: contact(id, name, numbers), photo: photo.map(PathBuf::from) }
    }

    #[test]
    fn numbers_written_differently_are_one() {
        for pair in [
            ("+33 6 12 34 56 78", "06 12 34 56 78"),
            ("+33612345678", "06-12-34-56-78"),
            ("0033 6 12 34 56 78", "(06) 12.34.56.78"),
            ("+1 (555) 010-0100", "555 010 0100"),
        ] {
            assert_eq!(digits(pair.0), digits(pair.1), "{pair:?}");
        }
        assert_ne!(digits("06 12 34 56 78"), digits("06 12 34 56 79"));
    }

    #[test]
    fn the_phones_contact_and_this_computers_are_one_person() {
        let phone = vec![contact("1", "Alice Rivera", &["+33 6 12 34 56 78"])];
        let local = vec![
            entry("pc:a", "Alice R.", &["06-12-34-56-78", "01 23 45 67 89"], Some("/p/alice.jpg")),
            entry("pc:b", "Bob", &["+1 555 010 0100"], None),
        ];
        let merged = merge(&phone, &HashMap::new(), &local);
        assert_eq!(merged.len(), 2);
        let alice = merged.iter().find(|m| m.contact.id == "1").unwrap();
        assert_eq!(alice.contact.name, "Alice Rivera", "the phone's name wins");
        assert_eq!(alice.source, "both");
        assert_eq!(alice.photo, Some(PathBuf::from("/p/alice.jpg")), "the phone has no picture");
        let bob = merged.iter().find(|m| m.contact.id == "pc:b").unwrap();
        assert_eq!(bob.source, "computer");
    }

    #[test]
    fn the_phones_picture_stays_over_this_computers() {
        let phone = vec![contact("1", "Alice", &["0612345678"])];
        let photos = HashMap::from([("1".to_owned(), PathBuf::from("/p/phone.jpg"))]);
        let local = vec![entry("pc:a", "Alice", &["+33612345678"], Some("/p/local.jpg"))];
        let merged = merge(&phone, &photos, &local);
        assert_eq!(merged[0].photo, Some(PathBuf::from("/p/phone.jpg")));
    }

    #[test]
    fn two_contacts_here_with_one_number_are_listed_once() {
        let local = vec![
            entry("pc:a", "Carol", &["07 11 22 33 44"], None),
            entry("pc:b", "Carol Home", &["+44 7 11 22 33 44"], None),
        ];
        assert_eq!(merge(&[], &HashMap::new(), &local).len(), 1);
    }

    #[test]
    fn favorites_come_first_then_by_name() {
        let mut star = contact("2", "Zed", &["111111111"]);
        star.starred = true;
        let phone = vec![contact("1", "bob", &["222222222"]), star];
        let local = vec![entry("pc:a", "Alice", &["333333333"], None)];
        let names: Vec<_> =
            merge(&phone, &HashMap::new(), &local).into_iter().map(|m| m.contact.name).collect();
        assert_eq!(names, ["Zed", "Alice", "bob"]);
    }
}
