// SPDX-License-Identifier: GPL-3.0-or-later
//! This Mac's Wi-Fi and Bluetooth for phones (#636): not on the Mac yet, so
//! phones see neither and get no switches for them.

pub fn read() -> (Option<bool>, Option<bool>) {
    (None, None)
}

#[allow(dead_code)]
pub fn available() -> bool {
    false
}

pub fn set(_wifi: Option<bool>, _bluetooth: Option<bool>) -> Result<(), String> {
    Err("not on the Mac yet".into())
}
