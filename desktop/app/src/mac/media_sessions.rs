// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS media session integration stubs.

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};

pub fn start(_on_change: impl Fn(Vec<MediaPlayer>) + Send + Sync + 'static) {}

pub fn command(_player: &str, _action: MediaAction, _pos: Option<u64>) -> Result<(), MediaError> {
    Ok(())
}

#[allow(dead_code)]
pub fn pause_for_call() {}

#[allow(dead_code)]
pub fn resume_after_call() {}

pub fn list_sessions() -> Vec<(String, bool, bool)> {
    Vec::new()
}

pub fn pause_session_if_playing(_id: &str) -> bool {
    false
}

pub fn play_session_if_paused(_id: &str) -> bool {
    false
}

#[allow(dead_code)]
pub fn says_video(_id: &str) -> bool {
    false
}
