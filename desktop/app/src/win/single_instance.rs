// SPDX-License-Identifier: GPL-3.0-or-later
//! One Nectarlink per user session and data folder. A second launch (from
//! the Start menu, a shortcut or a jump list) asks the running one to show
//! its window and exits.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
    },
    core::HSTRING,
};

/// Held for the lifetime of the primary instance.
#[derive(Debug)]
pub struct PrimaryInstance {
    mutex: HANDLE,
    activate: HANDLE,
}

// SAFETY: kernel handles are usable from any thread.
unsafe impl Send for PrimaryInstance {}
// SAFETY: as above; the handles are only waited on and closed.
unsafe impl Sync for PrimaryInstance {}

#[derive(Debug)]
pub enum Instance {
    Primary(PrimaryInstance),
    /// Another instance is running and was asked to show itself.
    Secondary,
}

/// Object names are per session ("Local\") and per data folder, so dev
/// builds with their own `--data-dir` can run side by side.
fn names(data_dir: &Path) -> (HSTRING, HSTRING) {
    let mut hasher = DefaultHasher::new();
    data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    let tag = format!("{:016x}", hasher.finish());
    (
        HSTRING::from(format!(r"Local\Nectarlink.Desktop.{tag}")),
        HSTRING::from(format!(r"Local\Nectarlink.Desktop.Activate.{tag}")),
    )
}

pub fn acquire(data_dir: &Path) -> windows::core::Result<Instance> {
    let (mutex_name, event_name) = names(data_dir);
    // SAFETY: plain kernel object calls with valid names; handles are closed
    // in Drop (or below for the secondary instance).
    unsafe {
        let mutex = CreateMutexW(None, true, &mutex_name)?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            // We were just launched by the user, so we may take the
            // foreground; pass that right on so the running instance's
            // window comes to the front instead of only flashing.
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, &event_name) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            return Ok(Instance::Secondary);
        }
        // Auto-reset: each SetEvent wakes the waiter once.
        let activate = CreateEventW(None, false, false, &event_name)?;
        Ok(Instance::Primary(PrimaryInstance { mutex, activate }))
    }
}

impl PrimaryInstance {
    /// Calls `on_activate` (on a background thread) whenever another launch
    /// asks this instance to show itself.
    pub fn watch(&'static self, on_activate: impl Fn() + Send + 'static) {
        let spawned = std::thread::Builder::new().name("single-instance".into()).spawn(move || {
            loop {
                // SAFETY: the event handle lives as long as `self` ('static).
                let woke = unsafe { WaitForSingleObject(self.activate, INFINITE) };
                if woke != WAIT_OBJECT_0 {
                    return;
                }
                on_activate();
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "can't watch for other launches");
        }
    }
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        // SAFETY: we own both handles.
        unsafe {
            let _ = CloseHandle(self.activate);
            let _ = CloseHandle(self.mutex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_secondary_and_activates_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let Instance::Primary(primary) = acquire(dir.path()).unwrap() else { panic!("first is primary") };
        assert!(matches!(acquire(dir.path()).unwrap(), Instance::Secondary));
        // The secondary set the activation event.
        // SAFETY: the handle is owned by `primary`, alive for this call.
        assert_eq!(unsafe { WaitForSingleObject(primary.activate, 0) }, WAIT_OBJECT_0);
        drop(primary);
        assert!(matches!(acquire(dir.path()).unwrap(), Instance::Primary(_)), "free again after exit");
    }

    #[test]
    fn different_data_folders_are_independent() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let first = acquire(a.path()).unwrap();
        assert!(matches!(first, Instance::Primary(_)));
        assert!(matches!(acquire(b.path()).unwrap(), Instance::Primary(_)));
    }
}
