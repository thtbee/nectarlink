// SPDX-License-Identifier: MPL-2.0
//! The `tracing` layer that writes the activity log, fail-closed.
//!
//! For each event it writes the time (UTC), the level, where it came from and
//! the message, then its fields:
//!
//! - numbers and true/false values always;
//! - text only for a field named in [`SAFE_FIELDS`], names used for codes,
//!   counts, kinds and short IDs. Any other field is written as
//!   `name=<hidden>`, so a new log line that carries a name, a message or a
//!   file can't leak it by accident;
//! - from other libraries (iroh and so on), text fields are hidden too,
//!   except errors that are plain I/O errors, written as their kind and OS
//!   code (`io:AddrNotAvailable/os10049`).
//!
//! All text written, the message included, goes through [`redact::text`]
//! first. The message is meant to be fixed text; one that's built from
//! values is still written, minus folders and addresses.

use std::{
    error::Error,
    fmt::{self, Write as _},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context};

use super::{redact, ring::RingLog};

/// Fields whose text is written. Each one carries a code, a kind, a count, a
/// version or a short device ID, never something a person typed or named.
pub const SAFE_FIELDS: &[&str] = &[
    "addr",
    "backend",
    "channel",
    "code",
    "codec",
    "congestion",
    "device",
    "error",
    "feature",
    "format",
    "from",
    "hotkey",
    "kind",
    "layout",
    "path",
    "peer",
    "reason",
    "retry_in",
    "session",
    "state",
    "t",
    "trigger",
    "version",
];

/// Writes events to a [`RingLog`].
#[derive(Debug)]
pub struct RingLayer {
    ring: Arc<RingLog>,
}

impl RingLayer {
    pub fn new(ring: Arc<RingLog>) -> RingLayer {
        RingLayer { ring }
    }
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut line = String::with_capacity(128);
        write_time(&mut line, SystemTime::now());
        let _ = write!(line, " {} {}:", level_letter(*meta.level()), meta.target());
        let mut fields = Fields { ours: is_ours(meta.target()), message: None, rest: String::new() };
        event.record(&mut fields);
        if let Some(message) = fields.message {
            line.push(' ');
            line.push_str(&message);
        }
        line.push_str(&fields.rest);
        line.push('\n');
        self.ring.append(line.as_bytes());
    }
}

/// Whether `target` is NectarLink's own code, whose field names follow
/// [`SAFE_FIELDS`].
fn is_ours(target: &str) -> bool {
    target.starts_with("nectarlink")
}

fn level_letter(level: Level) -> char {
    match level {
        Level::ERROR => 'E',
        Level::WARN => 'W',
        Level::INFO => 'I',
        Level::DEBUG => 'D',
        Level::TRACE => 'T',
    }
}

struct Fields {
    ours: bool,
    message: Option<String>,
    rest: String,
}

impl Fields {
    fn shows_text(&self, field: &Field) -> bool {
        let name = field.name();
        if self.ours { SAFE_FIELDS.contains(&name) } else { name == "error" }
    }

    fn push(&mut self, field: &Field, value: &str) {
        let _ = write!(self.rest, " {}=", field.name());
        if value.is_empty() || value.contains([' ', '"', '=']) {
            let _ = write!(self.rest, "\"{}\"", value.replace('"', "'"));
        } else {
            self.rest.push_str(value);
        }
    }

    fn hidden(&mut self, field: &Field) {
        let _ = write!(self.rest, " {}=<hidden>", field.name());
    }

    fn text(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(redact::text(value));
        } else if self.shows_text(field) {
            let value = redact::text(value);
            self.push(field, &value);
        } else {
            self.hidden(field);
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // `%x` arrives here too, written with its Display.
        if field.name() == "message" || self.shows_text(field) {
            self.text(field, &format!("{value:?}"));
        } else {
            self.hidden(field);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field, value);
    }

    fn record_error(&mut self, field: &Field, value: &(dyn Error + 'static)) {
        if let Some(io) = value.downcast_ref::<std::io::Error>() {
            let code = io.raw_os_error().map(|c| format!("/os{c}")).unwrap_or_default();
            self.push(field, &format!("io:{:?}{code}", io.kind()));
        } else if self.ours && self.shows_text(field) {
            self.text(field, &value.to_string());
        } else {
            self.hidden(field);
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, &value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, &value.to_string());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.push(field, &value.to_string());
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.push(field, &value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, &value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, if value { "true" } else { "false" });
    }
}

/// Writes `at` as `2026-11-20T10:01:02.123456Z`.
fn write_time(out: &mut String, at: SystemTime) {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    let _ = write!(
        out,
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:06}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_micros()
    );
}

/// The date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tracing_subscriber::prelude::*;

    use super::*;

    fn logged(f: impl FnOnce()) -> String {
        let dir = tempfile::tempdir().unwrap();
        let ring = Arc::new(RingLog::open(dir.path(), 1 << 20).unwrap());
        let subscriber = tracing_subscriber::registry().with(RingLayer::new(ring.clone()));
        tracing::subscriber::with_default(subscriber, f);
        fs::read_to_string(&ring.files()[0]).unwrap()
    }

    #[test]
    fn fields_not_on_the_list_are_hidden() {
        let out = logged(|| {
            tracing::warn!(
                target: "nectarlink_core::test",
                peer = "ybndrfg8",
                title = "Mum: dinner at 7?",
                count = 3,
                ok = true,
                "notification shown"
            );
        });
        assert!(out.contains(" W nectarlink_core::test: notification shown"), "{out}");
        assert!(out.contains("peer=ybndrfg8"), "{out}");
        assert!(out.contains("title=<hidden>"), "{out}");
        assert!(out.contains("count=3 ok=true"), "{out}");
        assert!(!out.contains("Mum"), "{out}");
    }

    #[test]
    fn other_libraries_keep_only_numbers_and_io_errors() {
        let err = std::io::Error::from_raw_os_error(10049);
        let out = logged(|| {
            tracing::warn!(
                target: "swarm_discovery",
                error = &err as &(dyn Error + 'static),
                peer = "x",
                port = 5353,
                "can't join multicast"
            );
        });
        assert!(out.contains("can't join multicast"), "{out}");
        assert!(out.contains("error=io:"), "{out}");
        assert!(out.contains("/os10049"), "{out}");
        assert!(out.contains("peer=<hidden>"), "{out}");
        assert!(out.contains("port=5353"), "{out}");
    }

    #[test]
    fn addresses_in_messages_become_their_kind() {
        let out = logged(|| {
            let at = "192.168.1.169:53319";
            tracing::info!(target: "iroh::net", "found peer at {at}");
        });
        assert!(out.contains("found peer at lan-v4:53319"), "{out}");
        assert!(!out.contains("192.168"), "{out}");
    }

    #[test]
    fn one_event_is_one_line() {
        let out = logged(|| {
            tracing::error!(target: "nectarlink_core::x", error = "first\nE nectarlink: forged", "failed");
        });
        assert_eq!(out.lines().count(), 1, "{out}");
    }

    #[test]
    fn times_are_utc() {
        let mut s = String::new();
        write_time(&mut s, UNIX_EPOCH + Duration::from_secs(1_763_632_862) + Duration::from_micros(5));
        assert_eq!(s, "2025-11-20T10:01:02.000005Z");
        s.clear();
        write_time(&mut s, UNIX_EPOCH + Duration::from_secs(951_782_400));
        assert_eq!(s, "2000-02-29T00:00:00.000000Z");
    }
}
