// SPDX-License-Identifier: MPL-2.0
//! Keeping this device's details out of log text.
//!
//! Every piece of text the log writes goes through [`text`]: the folders that
//! name the person using the device (their home folder, which has their user
//! name in it, and the app's own folders) become `<home>`, `<data>` and so
//! on, and network addresses become what kind of address they are
//! (`lan-v4:53319`, `public-v4`) rather than the address. The helpers
//! [`peer`], [`path`] and [`addr`] give the same forms for values logged on
//! purpose.

use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::OnceLock,
};

use nectarlink_protocol::DeviceId;

/// Folders to name rather than spell out, longest first.
struct Roots(Vec<(String, &'static str)>);

static ROOTS: OnceLock<Roots> = OnceLock::new();

/// Tells the log which folders are this device's: `data` is the app's data
/// folder. The home folder, the temporary folder and the app's own folder are
/// found here. Only the first call counts.
pub fn init_roots(data: &Path) {
    let mut roots = Vec::new();
    let install = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned));
    for (dir, name) in [
        (Some(data.to_owned()), "<data>"),
        (install, "<install>"),
        (Some(std::env::temp_dir()), "<temp>"),
        (home(), "<home>"),
    ] {
        let Some(dir) = dir else { continue };
        let dir = dir.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
        // A root like `/` or `C:` would turn every path into nonsense.
        if dir.len() >= 4 {
            roots.push((dir, name));
        }
    }
    let _ = ROOTS.set(Roots::new(roots));
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(Into::into)
}

impl Roots {
    fn new(mut roots: Vec<(String, &'static str)>) -> Roots {
        // Both slash styles: Qt and some tools write C:/Users/... on Windows.
        let mut both = Vec::new();
        for (dir, name) in roots.drain(..) {
            let other = if dir.contains('\\') { dir.replace('\\', "/") } else { dir.replace('/', "\\") };
            if other != dir {
                both.push((other, name));
            }
            both.push((dir, name));
        }
        both.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.len()));
        Roots(both)
    }

    fn replace(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (dir, name) in &self.0 {
            out = replace_ignoring_case(&out, dir, name);
        }
        out
    }
}

/// `text` with every `needle` replaced, ignoring ASCII case on Windows
/// (`C:\users\Bee` is the same folder there).
fn replace_ignoring_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    let matches = |hay: &str| if cfg!(windows) { hay.eq_ignore_ascii_case(needle) } else { hay == needle };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let end = i + needle.len();
        if end <= text.len() && text.is_char_boundary(end) && matches(&text[i..end]) {
            out.push_str(with);
            i = end;
        } else {
            let c = text[i..].chars().next().unwrap_or_default();
            out.push(c);
            i += c.len_utf8().max(1);
        }
    }
    out
}

/// `text` as the log may write it: this device's folders named, network
/// addresses replaced by their kind, and control characters (which could
/// fake extra log lines) shown as spaces.
pub fn text(text: &str) -> String {
    let named = match ROOTS.get() {
        Some(roots) => roots.replace(text),
        None => text.to_owned(),
    };
    let masked = mask_addresses(&named);
    masked.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// Replaces each word of `text` that is an IP address, with or without a
/// port, by its kind. Words end at spaces and at `,;()"'<>=` and at a `[`
/// or `]` that isn't part of `[v6]:port`.
fn mask_addresses(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            out.push_str(&mask_word(word));
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '"' | '\'' | '<' | '>' | '=' | '{' | '}')
        {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn mask_word(word: &str) -> String {
    // Trailing punctuation from a sentence ("at 10.0.0.2.") isn't part of it.
    let trimmed = word.trim_end_matches(['.', ':', '!', '?']);
    let tail = &word[trimmed.len()..];
    if let Some(kind) = address_kind(trimmed) {
        return format!("{kind}{tail}");
    }
    // A list's brackets around an address: `[10.0.0.2:1`.
    let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
    if inner.len() < trimmed.len()
        && let Some(kind) = address_kind(inner)
    {
        let start = trimmed.len() - trimmed.trim_start_matches('[').len();
        let end = trimmed.len() - trimmed.trim_end_matches(']').len();
        return format!("{}{kind}{}{tail}", &trimmed[..start], &trimmed[trimmed.len() - end..]);
    }
    word.to_owned()
}

/// What kind of address `word` is, when it's one: `ip`, `ip:port`,
/// `[v6]` or `[v6]:port`.
fn address_kind(word: &str) -> Option<String> {
    if let Ok(sa) = word.parse::<SocketAddr>() {
        return Some(addr(sa));
    }
    if let Ok(ip) = word.parse::<IpAddr>() {
        return Some(ip_kind(ip).to_owned());
    }
    let v6 = word.strip_prefix('[')?.strip_suffix(']')?;
    v6.parse::<IpAddr>().ok().map(|ip| ip_kind(ip).to_owned())
}

/// A paired device as the log writes it: the short form of its ID.
pub fn peer(id: &DeviceId) -> String {
    id.short()
}

/// A file or folder as the log writes it: inside one of this device's
/// folders, that folder's name and the rest (`<data>\cache\x.bin`);
/// anywhere else, only what kind of file it is (`<file>.jpg`), since its
/// name and folders are the user's.
pub fn path(path: &Path) -> String {
    let full = path.to_string_lossy();
    if let Some(roots) = ROOTS.get() {
        let named = roots.replace(&full);
        // Only a path that starts inside a known app folder keeps its
        // name; under <home> the rest is the user's own folders and files.
        let app_owned = ["<data>", "<install>", "<temp>"].iter().any(|r| named.starts_with(r));
        if app_owned {
            return text(&named);
        }
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) => {
            format!("<file>.{}", ext.to_ascii_lowercase())
        }
        _ => "<file>".to_owned(),
    }
}

/// A network address as the log writes it: its kind and port.
pub fn addr(addr: SocketAddr) -> String {
    format!("{}:{}", ip_kind(addr.ip()), addr.port())
}

/// What kind of address `ip` is, without the address.
pub fn ip_kind(ip: IpAddr) -> &'static str {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            if v4.is_loopback() {
                "loopback"
            } else if v4.is_unspecified() {
                "any-v4"
            } else if v4.is_private() {
                "lan-v4"
            } else if v4.is_link_local() {
                "link-local-v4"
            } else if a == 100 && (64..128).contains(&b) {
                // Carrier-grade NAT, and Tailscale.
                "cgnat"
            } else if v4.is_multicast() || v4.is_broadcast() {
                "multicast-v4"
            } else {
                "public-v4"
            }
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            if let Some(v4) = v6.to_ipv4_mapped() {
                ip_kind(IpAddr::V4(v4))
            } else if v6.is_loopback() {
                "loopback"
            } else if v6.is_unspecified() {
                "any-v6"
            } else if first & 0xfe00 == 0xfc00 {
                "lan-v6"
            } else if first & 0xffc0 == 0xfe80 {
                "link-local-v6"
            } else if v6.is_multicast() {
                "multicast-v6"
            } else {
                "public-v6"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        let home = if cfg!(windows) { r"C:\Users\Alice" } else { "/home/alice" };
        let data = if cfg!(windows) {
            r"C:\Users\Alice\AppData\Local\Nectarlink"
        } else {
            "/home/alice/.local/share/nectarlink"
        };
        Roots::new(vec![(home.into(), "<home>"), (data.into(), "<data>")])
    }

    #[test]
    fn the_users_folders_are_named_not_spelled_out() {
        let r = roots();
        if cfg!(windows) {
            assert_eq!(
                r.replace(r"adb=C:\Users\Alice\AppData\Local\Android\Sdk\platform-tools\adb.exe"),
                r"adb=<home>\AppData\Local\Android\Sdk\platform-tools\adb.exe"
            );
            assert_eq!(r.replace(r"c:\users\alice\AppData\Local\Nectarlink\logs"), r"<data>\logs");
            assert_eq!(r.replace("C:/Users/Alice/Pictures"), "<home>/Pictures");
        } else {
            assert_eq!(r.replace("/home/alice/.local/share/nectarlink/logs"), "<data>/logs");
            assert_eq!(r.replace("open /home/alice/Pictures failed"), "open <home>/Pictures failed");
        }
    }

    #[test]
    fn addresses_become_their_kind() {
        assert_eq!(mask_addresses("dialing 192.168.1.20:53319 now"), "dialing lan-v4:53319 now");
        assert_eq!(mask_addresses("relay at 203.0.113.9."), "relay at public-v4.");
        assert_eq!(mask_addresses("tailscale 100.101.5.6"), "tailscale cgnat");
        assert_eq!(mask_addresses("v6 [2001:db8::1]:443, done"), "v6 public-v6:443, done");
        assert_eq!(mask_addresses("ula fd12:3456::1"), "ula lan-v6");
        assert_eq!(mask_addresses("addrs=[10.0.0.2:1, 10.0.0.3:2]"), "addrs=[lan-v4:1, lan-v4:2]");
        // Versions and times aren't addresses.
        assert_eq!(mask_addresses("version 0.1.6 at 12:30:01"), "version 0.1.6 at 12:30:01");
    }

    #[test]
    fn control_characters_cant_fake_a_log_line() {
        assert_eq!(text("a\nW core \"fake\"\r"), "a W core \"fake\" ");
    }

    #[test]
    fn a_users_file_shows_only_its_kind() {
        assert_eq!(path(Path::new("/somewhere/Holiday Photos/IMG_2041.JPG")), "<file>.jpg");
        assert_eq!(path(Path::new("/somewhere/secret-plans")), "<file>");
    }

    #[test]
    fn case_is_ignored_only_where_the_file_system_does() {
        let out = replace_ignoring_case("X ABC y", "abc", "<r>");
        assert_eq!(out, if cfg!(windows) { "X <r> y" } else { "X ABC y" });
        assert_eq!(replace_ignoring_case("é abc é", "abc", "<r>"), "é <r> é");
    }
}
