// SPDX-License-Identifier: GPL-3.0-or-later
//! The Qt objects QML talks to. Each one is a thin view over the shared
//! [`crate::state::Hub`]: it subscribes to the parts it shows and refreshes
//! on the Qt thread, coalesced so a burst of core events costs one refresh
//! (spikes/s1-qt-rust, lesson 1).

// cxx-qt generates FFI code with `unsafe`; these modules are the boundary.
#[allow(unsafe_code)]
pub mod agents;
#[allow(unsafe_code)]
pub mod app;
#[allow(unsafe_code)]
pub mod calls;
#[allow(unsafe_code)]
pub mod deck;
#[allow(unsafe_code)]
pub mod devices;
#[allow(unsafe_code)]
pub mod history;
#[allow(unsafe_code)]
pub mod media;
#[allow(unsafe_code)]
pub mod messages;
#[allow(unsafe_code)]
pub mod mirror;
#[allow(unsafe_code)]
pub mod native;
#[allow(unsafe_code)]
pub mod notifications;
#[allow(unsafe_code)]
pub mod pairing;
#[allow(unsafe_code)]
pub mod phone_install;
#[allow(unsafe_code)]
pub mod phone_mic;
#[allow(unsafe_code)]
pub mod photo_backup;
#[allow(unsafe_code)]
pub mod photos;
#[allow(unsafe_code)]
pub mod prefs;
#[allow(unsafe_code)]
pub mod report;
#[allow(unsafe_code)]
pub mod setup;
#[allow(unsafe_code)]
pub mod timeline;
#[allow(unsafe_code)]
pub mod transfers;
#[allow(unsafe_code)]
pub mod webcam;

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use nectarlink_core::DeviceId;

use crate::state::Changes;

/// Subscribes `qt`'s object to `interest`: `refresh` runs on the Qt thread
/// after changes, at most one call queued at a time. The subscription ends
/// when the object is destroyed.
pub(crate) fn subscribe<T>(qt: CxxQtThread<T>, interest: Changes, refresh: fn(Pin<&mut T>))
where
    T: Threading + 'static,
{
    let scheduled = Arc::new(AtomicBool::new(false));
    crate::core_host::host().hub.subscribe(interest, move || {
        if scheduled.swap(true, Ordering::AcqRel) {
            return true; // a refresh is already queued and will see this change
        }
        let scheduled = scheduled.clone();
        qt.queue(move |object| {
            // Clear first so changes during the refresh schedule another.
            scheduled.store(false, Ordering::Release);
            refresh(object);
        })
        .is_ok()
    });
}

/// Parses a device ID coming from QML.
pub(crate) fn parse_device(id: &QString) -> Option<DeviceId> {
    String::from(id).parse().ok()
}

/// One step turning a list model's rows into new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edit {
    Remove(usize),
    Insert(usize),
    Change(usize),
}

/// Edits from `old` to `new` (rows identified by `key`), applied in order,
/// so QML delegates keep their state and animate. A row that moved (e.g. an
/// updated notification going to the top) is removed and inserted again;
/// the model is never reset.
pub(crate) fn diff<T: PartialEq, K: Eq + std::hash::Hash>(
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> K,
) -> Vec<Edit> {
    let mut edits = Vec::new();
    if old.is_empty() && new.is_empty() {
        return edits;
    }
    let new_keys: Vec<K> = new.iter().map(&key).collect();
    let new_set: std::collections::HashSet<&K> = new_keys.iter().collect();
    // Rows as they are after each edit so far.
    let mut rows: Vec<(K, &T)> = old.iter().map(|r| (key(r), r)).collect();
    // Removals first, from the end so earlier indices stay valid.
    for row in (0..rows.len()).rev() {
        if !new_set.contains(&rows[row].0) {
            rows.remove(row);
            edits.push(Edit::Remove(row));
        }
    }
    for (row, (item, k)) in new.iter().zip(new_keys).enumerate() {
        if rows.get(row).is_some_and(|(rk, _)| *rk == k) {
            if *rows[row].1 != *item {
                rows[row].1 = item;
                edits.push(Edit::Change(row));
            }
            continue;
        }
        // Moved here from further down: take it out there first.
        if let Some(from) = rows.iter().skip(row).position(|(rk, _)| *rk == k).map(|i| i + row) {
            rows.remove(from);
            edits.push(Edit::Remove(from));
        }
        rows.insert(row, (k, item));
        edits.push(Edit::Insert(row));
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies edits the way the models do.
    fn apply(old: &[(u8, &str)], new: &[(u8, &str)]) -> Vec<(u8, String)> {
        let mut rows: Vec<(u8, String)> = old.iter().map(|(k, v)| (*k, (*v).to_owned())).collect();
        for edit in diff(old, new, |r| r.0) {
            match edit {
                Edit::Remove(r) => {
                    rows.remove(r);
                }
                Edit::Insert(r) => rows.insert(r, (new[r].0, new[r].1.to_owned())),
                Edit::Change(r) => rows[r] = (new[r].0, new[r].1.to_owned()),
            }
        }
        rows
    }

    fn owned(rows: &[(u8, &str)]) -> Vec<(u8, String)> {
        rows.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
    }

    #[test]
    fn edits_turn_old_rows_into_new_ones() {
        type Rows<'a> = &'a [(u8, &'a str)];
        let cases: &[(Rows, Rows)] = &[
            (&[], &[(1, "a"), (2, "b")]),
            (&[(1, "a"), (2, "b")], &[]),
            (&[(1, "a"), (2, "b"), (3, "c")], &[(1, "a"), (2, "B"), (4, "d")]),
            // An update moves to the top.
            (&[(1, "a"), (2, "b"), (3, "c")], &[(3, "C"), (1, "a"), (2, "b")]),
            // Reversed, with additions and removals.
            (&[(1, "a"), (2, "b"), (3, "c"), (4, "d")], &[(5, "e"), (4, "d"), (2, "b"), (1, "a")]),
        ];
        for (old, new) in cases {
            assert_eq!(apply(old, new), owned(new), "{old:?} -> {new:?}");
        }
    }

    /// `snake_case` as cxx-qt names it for C++.
    fn camel(snake: &str) -> String {
        let mut out = String::new();
        let mut up = false;
        for c in snake.chars() {
            if c == '_' {
                up = true;
            } else if up {
                out.extend(c.to_uppercase());
                up = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    fn cxx_name(attributes: &[String]) -> Option<String> {
        let attribute = attributes.iter().find(|a| a.contains("cxx_name"))?;
        Some(attribute.split('"').nth(1)?.to_owned())
    }

    /// A QML call named like a property's setter (`setFoo` for `foo`) runs the setter, which
    /// only changes the property: an invokable with that name never runs. Every bridge file
    /// is checked.
    #[test]
    fn no_invokable_is_named_like_a_property_setter() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let mut setters = Vec::new();
            let mut invokables = Vec::new();
            let mut attributes: Vec<String> = Vec::new();
            for line in source.lines().map(str::trim) {
                if let Some(rest) = line.strip_prefix("#[qproperty(") {
                    // `READ` alone (with `NOTIFY`) leaves out the setter.
                    let writable = !line.contains("READ") || line.contains("WRITE");
                    let mut parts = rest.split(',').map(str::trim);
                    let name = parts.nth(1).unwrap_or_default().trim_end_matches(")]");
                    let name = cxx_name(&[line.to_owned()]).unwrap_or_else(|| camel(name));
                    let mut chars = name.chars();
                    let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                    if writable {
                        setters.push(format!("set{first}{}", chars.as_str()));
                    }
                } else if line.starts_with("#[") {
                    attributes.push(line.to_owned());
                } else if let Some(rest) = line.strip_prefix("fn ") {
                    if attributes.iter().any(|a| a == "#[qinvokable]") {
                        let rust = rest.split('(').next().unwrap_or_default();
                        invokables.push(cxx_name(&attributes).unwrap_or_else(|| camel(rust)));
                    }
                    attributes.clear();
                } else if !line.starts_with("//") {
                    attributes.clear();
                }
            }
            checked += invokables.len();
            for name in &invokables {
                assert!(
                    !setters.contains(name),
                    "{}: the invokable `{name}` is also the setter of a property; name it otherwise",
                    path.display()
                );
            }
        }
        // The scan found the bridge's invokables at all.
        assert!(checked > 100, "{checked}");
    }

    #[test]
    fn unchanged_rows_need_no_edits_and_moves_touch_only_the_moved_row() {
        let rows = [(1, "a"), (2, "b"), (3, "c")];
        assert_eq!(diff(&rows, &rows, |r| r.0), []);
        let moved = [(3, "c"), (1, "a"), (2, "b")];
        assert_eq!(diff(&rows, &moved, |r| r.0), [Edit::Remove(2), Edit::Insert(0)]);
    }
}
