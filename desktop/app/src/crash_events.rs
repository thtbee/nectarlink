// SPDX-License-Identifier: GPL-3.0-or-later
//! What Windows recorded when NectarLink crashed in a way Rust never saw (a
//! crash in Qt, a graphics driver or Windows itself): its "Application
//! Error" event (ID 1000) in the Application log, which names the module
//! that crashed and the exception code. Read once, at startup, and only
//! after a run that ended badly with no record of its own.

use nectarlink_core::diag::crash::{self, CrashRecord};

/// Windows' record of this app crashing since `started` (seconds since
/// 1970), as a crash record; `None` when there's none (or not on Windows).
pub fn native_crash_since(started: u64, version: &str) -> Option<CrashRecord> {
    let exe = std::env::current_exe().ok()?.file_name()?.to_string_lossy().into_owned();
    let xml = query_events()?;
    newest_crash(&xml, &exe, started, version)
}

#[cfg(windows)]
fn query_events() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("wevtutil")
        .args([
            "qe",
            "Application",
            "/q:*[System[Provider[@Name='Application Error'] and (EventID=1000)]]",
            "/c:20",
            "/rd:true",
            "/f:xml",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(windows))]
fn query_events() -> Option<String> {
    None
}

/// The newest event in `xml` (wevtutil's output, newest first) for `exe`
/// at or after `started`.
fn newest_crash(xml: &str, exe: &str, started: u64, version: &str) -> Option<CrashRecord> {
    for event in xml.split("<Event ").skip(1) {
        let at = attr(event, "SystemTime").and_then(parse_time)?;
        if at < started {
            // Newest first: everything after is older still.
            return None;
        }
        if !data(event, "AppName").is_some_and(|name| name.eq_ignore_ascii_case(exe)) {
            continue;
        }
        let module = data(event, "ModuleName").unwrap_or("<unknown>");
        let code = data(event, "ExceptionCode").unwrap_or("<unknown>");
        let offset = data(event, "FaultOffset").unwrap_or("0");
        // Module names are DLL names; anything else isn't kept.
        let module = if module.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            module
        } else {
            "<unknown>"
        };
        let code = if code.chars().all(|c| c.is_ascii_hexdigit() || c == 'x') { code } else { "<unknown>" };
        let offset = if offset.chars().all(|c| c.is_ascii_hexdigit() || c == 'x') { offset } else { "0" };
        return Some(CrashRecord {
            sig: crash::signature(&["native", &module.to_ascii_lowercase(), code]),
            kind: "native".to_owned(),
            location: format!("{module}+{offset}"),
            message: format!("exception {code}"),
            thread: None,
            version: version.to_owned(),
            at,
        });
    }
    None
}

/// The value of `name='…'` in `xml`.
fn attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let start = xml.find(&format!("{name}='"))? + name.len() + 2;
    let len = xml[start..].find('\'')?;
    Some(&xml[start..start + len])
}

/// The text of `<Data Name='name'>…</Data>` in `xml`.
fn data<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<Data Name='{name}'>");
    let start = xml.find(&open)? + open.len();
    let len = xml[start..].find("</Data>")?;
    Some(xml[start..start + len].trim())
}

/// `2026-11-20T10:01:02.1234567Z` as seconds since 1970.
fn parse_time(text: &str) -> Option<u64> {
    let date = text.get(..10)?;
    let time = text.get(11..19)?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next()??);
    let days = days_from_civil(y, m, day);
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s).ok()
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(time: &str, app: &str, module: &str) -> String {
        format!(
            "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
             <Provider Name='Application Error'/><EventID>1000</EventID>\
             <TimeCreated SystemTime='{time}'/></System><EventData>\
             <Data Name='AppName'>{app}</Data><Data Name='AppVersion'>0.1.6.0</Data>\
             <Data Name='ModuleName'>{module}</Data><Data Name='ExceptionCode'>c0000005</Data>\
             <Data Name='FaultOffset'>00000000000a1b2c</Data></EventData></Event>"
        )
    }

    #[test]
    fn the_newest_crash_of_this_app_since_the_run_started() {
        let xml = event("2026-11-20T10:00:05.1234567Z", "other.exe", "x.dll")
            + &event("2026-11-20T10:00:03.0000000Z", "nectarlink-desktop.exe", "Qt6Gui.dll")
            + &event("2026-11-19T09:00:00.0000000Z", "nectarlink-desktop.exe", "old.dll");
        let started = parse_time("2026-11-20T09:59:00Z").unwrap();
        let record = newest_crash(&xml, "Nectarlink-Desktop.exe", started, "0.1.6").unwrap();
        assert_eq!(record.kind, "native");
        assert_eq!(record.location, "Qt6Gui.dll+00000000000a1b2c");
        assert_eq!(record.message, "exception c0000005");
        assert_eq!(record.at, parse_time("2026-11-20T10:00:03Z").unwrap());
        // The same module and code is the same bug at any offset.
        let again = newest_crash(
            &event("2026-11-20T10:00:03.0Z", "nectarlink-desktop.exe", "qt6gui.dll")
                .replace("a1b2c", "ffff0"),
            "nectarlink-desktop.exe",
            started,
            "0.1.7",
        )
        .unwrap();
        assert_eq!(record.sig, again.sig);
    }

    #[test]
    fn crashes_before_the_run_dont_count() {
        let xml = event("2026-11-19T09:00:00.0Z", "nectarlink-desktop.exe", "old.dll");
        let started = parse_time("2026-11-20T09:59:00Z").unwrap();
        assert_eq!(newest_crash(&xml, "nectarlink-desktop.exe", started, "0.1.6"), None);
    }

    #[test]
    fn odd_module_names_arent_kept() {
        let xml = event("2026-11-20T10:00:03.0Z", "nectarlink-desktop.exe", "C:\\Users\\Alice\\x.dll");
        let record = newest_crash(&xml, "nectarlink-desktop.exe", 0, "0.1.6").unwrap();
        assert_eq!(record.location, "<unknown>+00000000000a1b2c");
    }

    #[test]
    fn times_parse() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2000-02-29T00:00:00.5Z"), Some(951_782_400));
        assert_eq!(parse_time("nonsense"), None);
    }
}
