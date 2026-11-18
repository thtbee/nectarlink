// SPDX-License-Identifier: GPL-3.0-or-later
//! What LogonUI asked the provider and what it answered, in the Application
//! event log (source "NectarLink Unlock"), to find out why a tile was or
//! wasn't shown. Never account names, security identifiers or passwords. The
//! event log rather than a file: LogonUI runs as SYSTEM, and a file in a folder
//! users can write to could be swapped for a link to somewhere else.

#![allow(unsafe_code)]

use windows::{
    Win32::System::EventLog::{
        DeregisterEventSource, EVENTLOG_INFORMATION_TYPE, RegisterEventSourceW, ReportEventW,
    },
    core::{HSTRING, PCWSTR, w},
};

pub fn write(args: std::fmt::Arguments<'_>) {
    let text = HSTRING::from(format!("pid {}: {args}", std::process::id()));
    // SAFETY: plain calls with strings that outlive them; the handle is closed below.
    unsafe {
        let Ok(source) = RegisterEventSourceW(PCWSTR::null(), w!("NectarLink Unlock")) else {
            return;
        };
        let strings = [PCWSTR(text.as_ptr())];
        let _ = ReportEventW(source, EVENTLOG_INFORMATION_TYPE, 0, 1000, None, 0, Some(&strings), None);
        let _ = DeregisterEventSource(source);
    }
}

macro_rules! note {
    ($($t:tt)*) => { $crate::trace::write(format_args!($($t)*)) };
}
pub(crate) use note;
