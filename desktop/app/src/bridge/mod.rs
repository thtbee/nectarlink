// SPDX-License-Identifier: GPL-3.0-or-later
//! The Qt objects QML talks to. Each one is a thin view over the shared
//! [`crate::state::Hub`]: it subscribes to the parts it shows and refreshes
//! on the Qt thread, coalesced so a burst of core events costs one refresh
//! (spikes/s1-qt-rust, lesson 1).

// cxx-qt generates FFI code with `unsafe`; these modules are the boundary.
#[allow(unsafe_code)]
pub mod app;
#[allow(unsafe_code)]
pub mod devices;
#[allow(unsafe_code)]
pub mod native;
#[allow(unsafe_code)]
pub mod notifications;
#[allow(unsafe_code)]
pub mod pairing;
#[allow(unsafe_code)]
pub mod prefs;

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
/// so QML delegates keep their state and animate. `None` when rows that stay
/// changed their relative order (then the model is reset).
pub(crate) fn diff<T: PartialEq, K: PartialEq>(
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> K,
) -> Option<Vec<Edit>> {
    let mut edits = Vec::new();
    // Removals, from the end so earlier indices stay valid.
    for (row, item) in old.iter().enumerate().rev() {
        if !new.iter().any(|n| key(n) == key(item)) {
            edits.push(Edit::Remove(row));
        }
    }
    let kept: Vec<&T> = old.iter().filter(|o| new.iter().any(|n| key(n) == key(o))).collect();
    let mut next_kept = 0;
    for (row, item) in new.iter().enumerate() {
        match kept.get(next_kept) {
            Some(k) if key(k) == key(item) => {
                if *k != item {
                    edits.push(Edit::Change(row));
                }
                next_kept += 1;
            }
            // Rows already in the list must come in the same order.
            _ if kept.iter().any(|k| key(k) == key(item)) => return None,
            _ => edits.push(Edit::Insert(row)),
        }
    }
    Some(edits)
}
