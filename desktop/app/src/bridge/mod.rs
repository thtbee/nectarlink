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
