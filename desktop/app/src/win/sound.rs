// SPDX-License-Identifier: GPL-3.0-or-later
//! Ringing the PC ("find my PC" from the phone): the Windows looping alarm
//! sound, until stopped or for at most a minute.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use windows::{
    Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_FLAGS, SND_LOOP, SND_NODEFAULT},
    core::{HSTRING, PCWSTR},
};

/// Rings stop by themselves after this long.
const MAX_RING: Duration = Duration::from_secs(60);

/// Bumped on every start/stop, so an old timer can't stop a newer ring.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn start_ringing() {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // Fall back to a plain system sound where the alarm alias is missing.
    let played = ["Notification.Looping.Alarm", "SystemExclamation"].iter().any(|alias| {
        let alias = HSTRING::from(*alias);
        // SAFETY: the alias is a valid null-terminated string for the call.
        unsafe { PlaySoundW(&alias, None, SND_ALIAS | SND_ASYNC | SND_LOOP | SND_NODEFAULT) }.as_bool()
    });
    if !played {
        tracing::warn!("no sound available to ring with");
    }
    let _ = std::thread::Builder::new().name("ring-timeout".into()).spawn(move || {
        std::thread::sleep(MAX_RING);
        if GENERATION.load(Ordering::SeqCst) == generation {
            stop_ringing();
        }
    });
}

pub fn stop_ringing() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    // SAFETY: a null sound stops whatever this process is playing.
    unsafe {
        let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
    }
}
