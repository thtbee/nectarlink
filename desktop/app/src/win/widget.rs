// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows 11 Widgets board: no NectarLink widget yet (#85). A provider
//! needs the Windows App SDK and a registered package; until it's built, the
//! package doesn't offer one, and `--widget-provider` (from a package that an
//! older version registered) exits.

use std::{path::Path, process::ExitCode};

pub const ARG: &str = "--widget-provider";

pub fn write_state_snapshot(_dir: &Path) {}

pub fn run_provider(_data_dir: &Path) -> ExitCode {
    ExitCode::SUCCESS
}
