// SPDX-License-Identifier: GPL-3.0-or-later
//! Not wired up to the macOS share picker yet, so the Photos page has no Share button.

use std::path::PathBuf;

pub fn show(_paths: &[PathBuf]) -> Result<(), String> {
    Err("no share sheet on this system".to_owned())
}
