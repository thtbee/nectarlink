// SPDX-License-Identifier: MPL-2.0
//! Rich Handoff helpers (`docs/PLAN.md` §3.11):
//! - Map locations (`geo:` URIs and Google / Apple / Bing / OpenStreetMap links)
//! - YouTube and video links with playback timestamps (`t=`, `start=`, `#t=`)
//! - Safe document allowlist for `open_on_arrival` file transfers

use serde::{Deserialize, Serialize};

/// Classification of a link handed off between devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffKind {
    WebLink,
    VideoLink,
    MapLocation,
}

/// Parsed metadata for a link or `geo:` URI handed off between devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffLink {
    pub kind: HandoffKind,
    /// Normalized URL or `geo:` URI ready to send over `link.open`.
    pub url: String,
    /// Short human-readable label (e.g. `"Map · Golden Gate Bridge"`, `"YouTube · 1:30"`).
    pub label: String,
    /// Playback position in seconds when `kind == HandoffKind::VideoLink`.
    pub timestamp_secs: Option<u32>,
    /// Place name or coordinates when `kind == HandoffKind::MapLocation`.
    pub map_query: Option<String>,
}

/// Safe non-executable document and media extensions allowed to open
/// automatically with the OS default application when `open_on_arrival` is set.
const SAFE_HANDOFF_EXTENSIONS: &[&str] = &[
    // Documents & text
    "pdf", "txt", "md", "rtf", "csv", "tsv", "json", "xml", "yaml", "yml", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "odt", "ods", "odp", "epub", // Images
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", // Audio & video
    "mp3", "m4a", "wav", "flac", "ogg", "mp4", "mkv", "mov", "webm",
];

/// Returns `true` if `name` has a safe, non-executable document or media
/// extension that may be opened with the default application on arrival.
#[must_use]
pub fn is_safe_handoff_document(name: &str) -> bool {
    let trimmed = name.trim().trim_end_matches(['.', ' ']);
    let Some((_, ext)) = trimmed.rsplit_once('.') else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    SAFE_HANDOFF_EXTENSIONS.contains(&lower.as_str())
}

/// Formats a video timestamp in seconds as `"M:SS"` or `"H:MM:SS"`.
#[must_use]
pub fn format_video_timestamp(secs: u32) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let rem = secs % 60;
    if hours > 0 { format!("{hours}:{mins:02}:{rem:02}") } else { format!("{mins}:{rem:02}") }
}

/// Parses a video timestamp token such as `"90"`, `"90s"`, `"1m30s"`, `"1h02m03s"`,
/// or `"01:30"` into seconds.
#[must_use]
pub fn parse_video_timestamp(raw: &str) -> Option<u32> {
    let s = raw.trim().trim_end_matches('s').trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<u32>() {
        return (n > 0 && n <= 86_400 * 7).then_some(n);
    }
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        let secs = match parts.as_slice() {
            [m, sec] => m.parse::<u32>().ok()?.checked_mul(60)?.checked_add(sec.parse::<u32>().ok()?)?,
            [h, m, sec] => h
                .parse::<u32>()
                .ok()?
                .checked_mul(3600)?
                .checked_add(m.parse::<u32>().ok()?.checked_mul(60)?)?
                .checked_add(sec.parse::<u32>().ok()?)?,
            _ => return None,
        };
        return (secs > 0 && secs <= 86_400 * 7).then_some(secs);
    }

    // Parse HMS tokens like `1h2m3s` or `2m15` (after trailing `s` strip).
    let lower = raw.trim().to_ascii_lowercase();
    let mut total: u32 = 0;
    let mut cur: u32 = 0;
    let mut saw_unit = false;
    let mut saw_digit = false;
    for ch in lower.chars() {
        if let Some(d) = ch.to_digit(10) {
            cur = cur.checked_mul(10)?.checked_add(d)?;
            saw_digit = true;
        } else if ch == 'h' && saw_digit {
            total = total.checked_add(cur.checked_mul(3600)?)?;
            cur = 0;
            saw_digit = false;
            saw_unit = true;
        } else if ch == 'm' && saw_digit {
            total = total.checked_add(cur.checked_mul(60)?)?;
            cur = 0;
            saw_digit = false;
            saw_unit = true;
        } else if ch == 's' && saw_digit {
            total = total.checked_add(cur)?;
            cur = 0;
            saw_digit = false;
            saw_unit = true;
        } else {
            return None;
        }
    }
    if saw_digit {
        total = total.checked_add(cur)?;
    }
    (saw_unit && total > 0 && total <= 86_400 * 7).then_some(total)
}

/// Extracts the first `http://`, `https://`, or `geo:` link from `text` and
/// classifies it as a [`HandoffLink`].
#[must_use]
pub fn extract_handoff_link(text: &str) -> Option<HandoffLink> {
    let token = find_url_or_geo_token(text)?;
    classify_link_token(&token)
}

/// Attaches or updates a playback timestamp (`secs`) on a video URL.
#[must_use]
pub fn with_video_timestamp(url: &str, secs: u32) -> String {
    let trimmed = url.trim();
    if secs == 0 {
        return trimmed.to_owned();
    }
    let (before_frag, frag) = match trimmed.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (trimmed, None),
    };
    let lower_host = link_host(before_frag).to_ascii_lowercase();
    if lower_host == "vimeo.com" || lower_host.ends_with(".vimeo.com") {
        return format!("{before_frag}#t={secs}s");
    }
    let (base, query) = match before_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (before_frag, None),
    };
    let mut kept: Vec<String> = Vec::new();
    if let Some(q) = query {
        for pair in q.split('&').filter(|p| !p.is_empty()) {
            let key = pair.split('=').next().unwrap_or("");
            if !key.eq_ignore_ascii_case("t")
                && !key.eq_ignore_ascii_case("start")
                && !key.eq_ignore_ascii_case("time_continue")
            {
                kept.push(pair.to_owned());
            }
        }
    }
    kept.push(format!("t={secs}"));
    let mut out = format!("{base}?{}", kept.join("&"));
    if let Some(f) = frag
        && !f.starts_with("t=")
    {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Converts a `geo:` URI into a universal Google Maps HTTPS URL so desktop
/// platforms without a registered `geo:` handler can open it in the browser.
#[must_use]
pub fn geo_to_maps_https(uri: &str) -> Option<String> {
    let rest = uri.trim().strip_prefix("geo:").or_else(|| uri.trim().strip_prefix("GEO:"))?;
    let (coords_part, query_part) = match rest.split_once('?') {
        Some((c, q)) => (c, Some(q)),
        None => (rest, None),
    };
    if let Some(q) = query_part {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k.eq_ignore_ascii_case("q")
                && !v.trim().is_empty()
            {
                let decoded = percent_decode(v);
                return Some(format!(
                    "https://www.google.com/maps/search/?api=1&query={}",
                    percent_encode_query(decoded.trim())
                ));
            }
        }
    }
    let coords = coords_part.split(';').next().unwrap_or(coords_part).trim();
    if !coords.is_empty() && coords != "0,0" {
        return Some(format!(
            "https://www.google.com/maps/search/?api=1&query={}",
            percent_encode_query(coords)
        ));
    }
    None
}

/// Converts a street address or place query into a `geo:0,0?q=...` URI.
#[must_use]
pub fn address_to_geo_uri(address: &str) -> String {
    format!("geo:0,0?q={}", percent_encode_query(address.trim()))
}

fn find_url_or_geo_token(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Check if the whole string is a geo: URI (possibly with spaces in ?q= that we encode).
    if trimmed.len() > 4 && trimmed[..4].eq_ignore_ascii_case("geo:") {
        let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
        let normalized: String =
            first_line.chars().filter(|c| !c.is_control()).map(|c| if c == ' ' { '+' } else { c }).collect();
        if normalized.len() > 4 {
            return Some(normalized);
        }
    }
    for word in trimmed.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            matches!(c, '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!')
        });
        let lower_prefix: String = clean.chars().take(8).collect::<String>().to_ascii_lowercase();
        if lower_prefix.starts_with("https://")
            || lower_prefix.starts_with("http://")
            || lower_prefix.starts_with("geo:")
        {
            return Some(clean.to_owned());
        }
    }
    None
}

fn classify_link_token(url: &str) -> Option<HandoffLink> {
    if url.len() > nectarlink_protocol::messages::LINK_MAX_BYTES
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    let lower_prefix: String = url.chars().take(8).collect::<String>().to_ascii_lowercase();
    if lower_prefix.starts_with("geo:") {
        let rest = &url[4..];
        if rest.is_empty() {
            return None;
        }
        let query = extract_geo_label(rest);
        let label = match &query {
            Some(q) => format!("Map · {q}"),
            None => "Map location".to_owned(),
        };
        return Some(HandoffLink {
            kind: HandoffKind::MapLocation,
            url: format!("geo:{rest}"),
            label,
            timestamp_secs: None,
            map_query: query,
        });
    }
    if !lower_prefix.starts_with("http://") && !lower_prefix.starts_with("https://") {
        return None;
    }
    let rest = url.split("://").nth(1)?;
    if rest.is_empty() {
        return None;
    }
    let host = link_host(url).to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }

    if is_map_url(&host, rest) {
        let query = extract_http_map_query(url);
        let label = match &query {
            Some(q) => format!("Map · {q}"),
            None => "Map location".to_owned(),
        };
        return Some(HandoffLink {
            kind: HandoffKind::MapLocation,
            url: url.to_owned(),
            label,
            timestamp_secs: None,
            map_query: query,
        });
    }

    if let Some((provider, ts)) = classify_video_url(&host, url) {
        let label = match ts {
            Some(secs) => format!("{provider} · {}", format_video_timestamp(secs)),
            None => provider.to_owned(),
        };
        return Some(HandoffLink {
            kind: HandoffKind::VideoLink,
            url: url.to_owned(),
            label,
            timestamp_secs: ts,
            map_query: None,
        });
    }

    Some(HandoffLink {
        kind: HandoffKind::WebLink,
        url: url.to_owned(),
        label: host,
        timestamp_secs: None,
        map_query: None,
    })
}

fn link_host(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("")
}

fn is_map_url(host: &str, rest_after_scheme: &str) -> bool {
    let h = host.strip_prefix("www.").unwrap_or(host);
    if h == "maps.app.goo.gl"
        || h == "maps.apple.com"
        || h == "openstreetmap.org"
        || h.starts_with("maps.google.")
    {
        return true;
    }
    let path = rest_after_scheme.find('/').map_or("", |i| &rest_after_scheme[i..]);
    let lower_path = path.to_ascii_lowercase();
    if (h.starts_with("google.") || h == "goo.gl" || h == "bing.com")
        && (lower_path.starts_with("/maps") || lower_path.contains("/maps/"))
    {
        return true;
    }
    false
}

fn extract_geo_label(rest: &str) -> Option<String> {
    let (coords_part, query_part) = match rest.split_once('?') {
        Some((c, q)) => (c, Some(q)),
        None => (rest, None),
    };
    if let Some(q) = query_part {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k.eq_ignore_ascii_case("q")
            {
                let decoded = clean_map_label(&percent_decode(v));
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        }
    }
    let coords = coords_part.split(';').next().unwrap_or(coords_part).trim();
    if !coords.is_empty() && coords != "0,0" {
        return Some(coords.to_owned());
    }
    None
}

fn extract_http_map_query(url: &str) -> Option<String> {
    let before_frag = url.split('#').next().unwrap_or(url);
    if let Some((_, query)) = before_frag.split_once('?') {
        for key_want in ["q", "query", "daddr", "where1", "address"] {
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=')
                    && k.eq_ignore_ascii_case(key_want)
                {
                    let decoded = clean_map_label(&percent_decode(v));
                    if !decoded.is_empty() {
                        return Some(decoded);
                    }
                }
            }
        }
    }
    // Check `/maps/place/<name>` or `/maps/search/<name>`
    let after_scheme = before_frag.split("://").nth(1)?;
    let path = after_scheme.find('/').map(|i| &after_scheme[i..])?;
    for marker in ["/maps/place/", "/maps/search/"] {
        if let Some(idx) = path.to_ascii_lowercase().find(marker) {
            let rest = &path[idx + marker.len()..];
            let seg = rest.split('/').next().unwrap_or("");
            if !seg.is_empty() && !seg.starts_with('@') {
                let decoded = clean_map_label(&percent_decode(seg));
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        }
    }
    None
}

fn clean_map_label(raw: &str) -> String {
    // If geo query is `lat,lon(Label)`, prefer `Label`.
    let trimmed = raw.trim();
    if let Some(open) = trimmed.find('(')
        && let Some(close) = trimmed.rfind(')')
        && open + 1 < close
    {
        let inside = trimmed[open + 1..close].trim();
        if !inside.is_empty() {
            return truncate_label(inside, 60);
        }
    }
    truncate_label(trimmed, 60)
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    let trimmed = clean.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_owned()
    } else {
        let mut out: String = trimmed.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

fn classify_video_url(host: &str, url: &str) -> Option<(&'static str, Option<u32>)> {
    let h = host.strip_prefix("www.").unwrap_or(host);
    let after_scheme = url.split("://").nth(1).unwrap_or("");
    let path_and_more = after_scheme.find('/').map_or("", |i| &after_scheme[i..]);
    let path = path_and_more.split(['?', '#']).next().unwrap_or("");
    let lower_path = path.to_ascii_lowercase();

    let provider = if h == "youtu.be" {
        (path.len() > 1).then_some("YouTube")
    } else if h == "youtube.com" || h == "m.youtube.com" || h == "music.youtube.com" {
        (lower_path == "/watch"
            || lower_path.starts_with("/shorts/")
            || lower_path.starts_with("/live/")
            || lower_path.starts_with("/embed/"))
        .then_some("YouTube")
    } else if h == "vimeo.com" || h == "player.vimeo.com" {
        (path.len() > 1).then_some("Vimeo")
    } else if h == "twitch.tv" || h == "m.twitch.tv" {
        lower_path.starts_with("/videos/").then_some("Twitch")
    } else {
        None
    }?;

    let ts = extract_video_timestamp(url);
    Some((provider, ts))
}

fn extract_video_timestamp(url: &str) -> Option<u32> {
    let (_, after_q) = url.split_once('?').unwrap_or((url, ""));
    let query = after_q.split('#').next().unwrap_or("");
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=')
            && (k.eq_ignore_ascii_case("t")
                || k.eq_ignore_ascii_case("start")
                || k.eq_ignore_ascii_case("time_continue"))
            && let Some(secs) = parse_video_timestamp(v)
        {
            return Some(secs);
        }
    }
    if let Some((_, frag)) = url.split_once('#') {
        for pair in frag.split('&').filter(|p| !p.is_empty()) {
            if let Some((k, v)) = pair.split_once('=')
                && (k.eq_ignore_ascii_case("t") || k.eq_ignore_ascii_case("start"))
                && let Some(secs) = parse_video_timestamp(v)
            {
                return Some(secs);
            }
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b',' | b':') {
            out.push(char::from(b));
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_formats_map_locations() {
        let geo = extract_handoff_link("geo:37.7749,-122.4194?q=Golden+Gate+Bridge").unwrap();
        assert_eq!(geo.kind, HandoffKind::MapLocation);
        assert_eq!(geo.map_query.as_deref(), Some("Golden Gate Bridge"));
        assert_eq!(geo.label, "Map · Golden Gate Bridge");
        assert_eq!(
            geo_to_maps_https(&geo.url).as_deref(),
            Some("https://www.google.com/maps/search/?api=1&query=Golden+Gate+Bridge")
        );

        let coords_only = extract_handoff_link("geo:48.8584,2.2945").unwrap();
        assert_eq!(coords_only.kind, HandoffKind::MapLocation);
        assert_eq!(coords_only.map_query.as_deref(), Some("48.8584,2.2945"));

        let gmaps = extract_handoff_link(
            "Meet me here: https://www.google.com/maps/place/Eiffel+Tower/@48.8583701,2.2922926,17z",
        )
        .unwrap();
        assert_eq!(gmaps.kind, HandoffKind::MapLocation);
        assert_eq!(gmaps.map_query.as_deref(), Some("Eiffel Tower"));
        assert_eq!(gmaps.label, "Map · Eiffel Tower");

        let apple = extract_handoff_link("https://maps.apple.com/?q= Cupertino+CA").unwrap();
        assert_eq!(apple.kind, HandoffKind::MapLocation);
    }

    #[test]
    fn extracts_and_updates_video_timestamps() {
        let yt =
            extract_handoff_link("Watch this https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s").unwrap();
        assert_eq!(yt.kind, HandoffKind::VideoLink);
        assert_eq!(yt.timestamp_secs, Some(90));
        assert_eq!(yt.label, "YouTube · 1:30");

        let short = extract_handoff_link("https://youtu.be/dQw4w9WgXcQ?t=125").unwrap();
        assert_eq!(short.kind, HandoffKind::VideoLink);
        assert_eq!(short.timestamp_secs, Some(125));
        assert_eq!(short.label, "YouTube · 2:05");

        let vimeo = extract_handoff_link("https://vimeo.com/12345678#t=45s").unwrap();
        assert_eq!(vimeo.kind, HandoffKind::VideoLink);
        assert_eq!(vimeo.timestamp_secs, Some(45));
        assert_eq!(vimeo.label, "Vimeo · 0:45");

        let updated = with_video_timestamp("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10", 245);
        assert_eq!(updated, "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=245");
    }

    #[test]
    fn allowlists_safe_handoff_documents_and_rejects_executables() {
        for safe in ["report.pdf", "notes.txt", "slides.pptx", "budget.xlsx", "photo.png", "clip.mp4"] {
            assert!(is_safe_handoff_document(safe), "{safe} should be allowed");
        }
        for unsafe_name in [
            "setup.exe",
            "installer.msi",
            "script.bat",
            "run.ps1",
            "archive.zip",
            "app.apk",
            "README",
            ".bashrc",
        ] {
            assert!(!is_safe_handoff_document(unsafe_name), "{unsafe_name} must be rejected");
        }
    }
}
