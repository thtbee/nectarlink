// SPDX-License-Identifier: GPL-3.0-or-later
//! The Qt objects QML talks to. Each one is a thin view over the shared
//! [`crate::state::Hub`]: it subscribes to the parts it shows and refreshes
//! on the Qt thread, coalesced so a burst of core events costs one refresh
//! (spikes/s1-qt-rust, lesson 1).

// cxx-qt generates FFI code with `unsafe`; these modules are the boundary.
#[allow(unsafe_code)]
pub mod app;
#[allow(unsafe_code)]
pub mod calls;
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
pub mod prefs;
#[allow(unsafe_code)]
pub mod transfers;

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
pub(crate) fn diff<T: PartialEq, K: PartialEq>(old: &[T], new: &[T], key: impl Fn(&T) -> K) -> Vec<Edit> {
    let mut edits = Vec::new();
    // Rows as they are after each edit so far.
    let mut rows: Vec<&T> = old.iter().collect();
    // Removals first, from the end so earlier indices stay valid.
    for row in (0..rows.len()).rev() {
        if !new.iter().any(|n| key(n) == key(rows[row])) {
            rows.remove(row);
            edits.push(Edit::Remove(row));
        }
    }
    for (row, item) in new.iter().enumerate() {
        let k = key(item);
        if rows.get(row).is_some_and(|r| key(r) == k) {
            if *rows[row] != *item {
                rows[row] = item;
                edits.push(Edit::Change(row));
            }
            continue;
        }
        // Moved here from further down: take it out there first.
        if let Some(from) = rows.iter().skip(row).position(|r| key(r) == k).map(|i| i + row) {
            rows.remove(from);
            edits.push(Edit::Remove(from));
        }
        rows.insert(row, item);
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

    #[test]
    fn unchanged_rows_need_no_edits_and_moves_touch_only_the_moved_row() {
        let rows = [(1, "a"), (2, "b"), (3, "c")];
        assert_eq!(diff(&rows, &rows, |r| r.0), []);
        let moved = [(3, "c"), (1, "a"), (2, "b")];
        assert_eq!(diff(&rows, &moved, |r| r.0), [Edit::Remove(2), Edit::Insert(0)]);
    }
}
