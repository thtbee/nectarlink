// SPDX-License-Identifier: MPL-2.0
//! Smart clipboard classification ("context chips").
//!
//! Inspects plain-text clipboard items and suggests at most one fitting
//! action (`Open`, `Open in Maps`, `Call`, `Track`, `Email`). One-time
//! verification codes ([`crate::otp::is_otp_clip`]) keep their own handling
//! and never produce a chip.
//!
//! Privacy: [`ClipSuggestion`]'s [`fmt::Debug`] implementation intentionally
//! prints only the [`ClipKind`], never the clipboard text or target.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Kind of actionable content recognized in a plain-text clipboard item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    /// `http://` or `https://` web link -> `"Open"`.
    WebLink,
    /// Street address -> `"Open in Maps"`.
    StreetAddress,
    /// Phone number -> `"Call"`.
    PhoneNumber,
    /// Parcel tracking number from a known carrier format -> `"Track"`.
    TrackingNumber,
    /// Email address -> `"Email"`.
    Email,
}

impl ClipKind {
    /// Short chip label shown on the confirmation toast or banner.
    pub fn action_label(self) -> &'static str {
        match self {
            Self::WebLink => "Open",
            Self::StreetAddress => "Open in Maps",
            Self::PhoneNumber => "Call",
            Self::TrackingNumber => "Track",
            Self::Email => "Email",
        }
    }

    /// Stable snake_case identifier for QML/Kotlin bridges and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebLink => "web_link",
            Self::StreetAddress => "street_address",
            Self::PhoneNumber => "phone_number",
            Self::TrackingNumber => "tracking_number",
            Self::Email => "email",
        }
    }
}

/// Actionable suggestion for a plain-text clipboard item.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSuggestion {
    pub kind: ClipKind,
    /// Normalized action target:
    /// - [`ClipKind::WebLink`]: full `http://` or `https://` URL
    /// - [`ClipKind::StreetAddress`]: single-line street address
    /// - [`ClipKind::PhoneNumber`]: dialable phone number (`+` and digits)
    /// - [`ClipKind::TrackingNumber`]: uppercase tracking code
    /// - [`ClipKind::Email`]: email address (`user@domain`)
    pub target: String,
}

impl ClipSuggestion {
    /// Short chip label shown on the confirmation toast or banner.
    pub fn action_label(&self) -> &'static str {
        self.kind.action_label()
    }
}

impl fmt::Debug for ClipSuggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipSuggestion").field("kind", &self.kind).finish_non_exhaustive()
    }
}

/// Maximum clipboard text length considered for smart suggestions.
const MAX_CLIP_LEN: usize = 512;

/// Classifies `text` and returns at most one [`ClipSuggestion`].
///
/// Returns `None` for empty strings, long prose, file paths, currency amounts,
/// dates, ordinary numbers, and one-time verification codes.
pub fn classify_clip(text: &str) -> Option<ClipSuggestion> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_CLIP_LEN {
        return None;
    }

    // One-time verification codes keep their own handling and get no chip.
    if crate::otp::is_otp_clip(trimmed) {
        return None;
    }

    if let Some(url) = detect_web_link(trimmed) {
        return Some(ClipSuggestion { kind: ClipKind::WebLink, target: url });
    }
    if let Some(email) = detect_email(trimmed) {
        return Some(ClipSuggestion { kind: ClipKind::Email, target: email });
    }
    if let Some(tracking) = detect_tracking_number(trimmed) {
        return Some(ClipSuggestion { kind: ClipKind::TrackingNumber, target: tracking });
    }
    if let Some(phone) = detect_phone_number(trimmed) {
        return Some(ClipSuggestion { kind: ClipKind::PhoneNumber, target: phone });
    }
    if let Some(address) = detect_street_address(trimmed) {
        return Some(ClipSuggestion { kind: ClipKind::StreetAddress, target: address });
    }

    None
}

/// Builds a web search URL for a parcel tracking number (no carrier-specific APIs).
pub fn tracking_search_url(tracking_number: &str) -> String {
    format!("https://www.google.com/search?q={}", percent_encode(tracking_number.trim()))
}

/// Builds a web Maps search URL for a street address.
pub fn maps_web_url(address: &str) -> String {
    format!("https://www.google.com/maps/search/?api=1&query={}", percent_encode(address.trim()))
}

/// Percent-encodes a UTF-8 query parameter component (RFC 3986 unreserved characters kept).
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for &b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                use std::fmt::Write;
                let _ = write!(&mut out, "%{b:02X}");
            }
        }
    }
    out
}

fn detect_web_link(s: &str) -> Option<String> {
    if s.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return None;
    }
    // Reject Windows drive paths (`C:/foo`), UNC paths, and Unix paths.
    if s.starts_with('/') || s.starts_with('~') {
        return None;
    }
    if s.len() >= 3
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.as_bytes()[1] == b':'
        && s.as_bytes()[2] == b'/'
    {
        return None;
    }

    let (normalized, rest) = if let Some(r) = strip_ascii_case_prefix(s, "https://") {
        (format!("https://{r}"), r)
    } else if let Some(r) = strip_ascii_case_prefix(s, "http://") {
        (format!("http://{r}"), r)
    } else {
        let r = strip_ascii_case_prefix(s, "www.")?;
        (format!("https://www.{r}"), s)
    };

    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_port.split('@').next_back().unwrap_or(host_port);
    let host = if host.starts_with('[') {
        // IPv6 literal
        host.split(']').next().map(|h| &h[1..]).unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    if !is_valid_url_host(host) {
        return None;
    }
    Some(normalized)
}

fn strip_ascii_case_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn is_valid_url_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host.starts_with('.') || host.ends_with('.') || !host.contains('.') {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let all_digits = labels.iter().all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    if all_digits {
        // IPv4 address: 4 octets 0..=255
        return labels.len() == 4 && labels.iter().all(|l| l.parse::<u8>().is_ok());
    }
    for label in &labels {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
    }
    let tld = labels.last().copied().unwrap_or("");
    tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

fn detect_email(s: &str) -> Option<String> {
    let raw = strip_ascii_case_prefix(s, "mailto:").unwrap_or(s);
    // Strip optional mailto query string if mailto: was used.
    let candidate = if s.len() != raw.len() { raw.split('?').next().unwrap_or(raw) } else { raw };
    if candidate.is_empty()
        || candidate.len() > 254
        || candidate.chars().any(|c| c.is_whitespace() || c.is_control())
        || candidate.contains('/')
        || candidate.contains('\\')
        || candidate.contains(':')
    {
        return None;
    }
    let (local, domain) = candidate.split_once('@')?;
    if domain.contains('@') {
        return None;
    }
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-'))
    {
        return None;
    }
    if !is_valid_url_host(domain) || domain.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    let tld = labels.last().copied().unwrap_or("");
    if tld.len() < 2 || !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    // Reject common image/source filenames like `icon@2x.png` or `photo@3x.jpg`.
    if matches!(
        tld.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "pdf" | "zip" | "exe" | "dll"
    ) {
        return None;
    }
    Some(candidate.to_owned())
}

/// ISO-3166-1 alpha-2 country codes commonly used at the end of UPU S10 tracking numbers.
const S10_COUNTRY_CODES: &[&str] = &[
    "AD", "AE", "AR", "AT", "AU", "BE", "BG", "BR", "BY", "CA", "CH", "CL", "CN", "CO", "CY", "CZ", "DE",
    "DK", "EE", "EG", "ES", "FI", "FR", "GB", "GR", "HK", "HR", "HU", "ID", "IE", "IL", "IN", "IS", "IT",
    "JP", "KR", "LT", "LU", "LV", "MC", "MT", "MX", "MY", "NL", "NO", "NZ", "PE", "PH", "PK", "PL", "PT",
    "RO", "RS", "RU", "SA", "SE", "SG", "SI", "SK", "TH", "TR", "TW", "UA", "US", "UY", "VN", "ZA",
];

fn detect_tracking_number(s: &str) -> Option<String> {
    if s.contains('\n') || s.contains('\r') {
        return None;
    }
    // Allow only ASCII alphanumeric, spaces, and hyphens, with at most 4 separator runs.
    if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-') {
        return None;
    }
    if s.starts_with('-') || s.ends_with('-') {
        return None;
    }
    let compact: String =
        s.bytes().filter(|b| b.is_ascii_alphanumeric()).map(|b| (b as char).to_ascii_uppercase()).collect();
    if compact.len() < 12 || compact.len() > 26 {
        return None;
    }

    // 1. UPS: `1Z` + 16 alphanumeric characters (total 18), containing at least 8 digits.
    if compact.len() == 18
        && compact.starts_with("1Z")
        && compact[2..].bytes().all(|b| b.is_ascii_alphanumeric())
        && compact[2..].bytes().filter(|b| b.is_ascii_digit()).count() >= 8
    {
        return Some(compact);
    }

    // 2. UPU S10 international postal format: 2 letters + 9 digits + 2-letter ISO country code (total 13).
    // Valid S10 service indicator first letters: A..=Z (standard postal services use C, E, H, L, R, S, U, V).
    if compact.len() == 13 {
        let bytes = compact.as_bytes();
        let first_two_alpha = bytes[0].is_ascii_uppercase() && bytes[1].is_ascii_uppercase();
        let middle_nine_digits = bytes[2..11].iter().all(|b| b.is_ascii_digit());
        let country = &compact[11..13];
        let service = bytes[0];
        if first_two_alpha
            && middle_nine_digits
            && matches!(service, b'C' | b'E' | b'H' | b'L' | b'R' | b'S' | b'U' | b'V')
            && S10_COUNTRY_CODES.binary_search(&country).is_ok()
        {
            return Some(compact);
        }
    }

    // 3. Amazon Logistics: `TBA` + 12 digits (15 chars).
    if compact.len() == 15 && compact.starts_with("TBA") && compact[3..].bytes().all(|b| b.is_ascii_digit()) {
        return Some(compact);
    }

    // 4. FedEx Door Tag: `DT` + 12 digits (14 chars), or LaserShip `1LS` + 12 digits (15 chars).
    if (compact.len() == 14 && compact.starts_with("DT") && compact[2..].bytes().all(|b| b.is_ascii_digit()))
        || (compact.len() == 15
            && compact.starts_with("1LS")
            && compact[3..].bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(compact);
    }

    // 5. DHL Germany / Europe parcel (`JJD` + 10..=18 digits) or DHL eCommerce (`GM` + 16..=18 digits).
    if let Some(rest) = compact.strip_prefix("JJD")
        && (10..=18).contains(&rest.len())
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Some(compact);
    }
    if let Some(rest) = compact.strip_prefix("GM")
        && (16..=18).contains(&rest.len())
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Some(compact);
    }

    // 6. USPS domestic (20..=22 digits starting with known USPS prefixes 92, 93, 94, 95)
    //    and FedEx Ground / SmartPost 20..=22 digits starting with 9611, 9612, 9622, 6129.
    if (20..=22).contains(&compact.len()) && compact.bytes().all(|b| b.is_ascii_digit()) {
        let known_prefix = compact.starts_with("940")
            || compact.starts_with("941")
            || compact.starts_with("947")
            || compact.starts_with("920")
            || compact.starts_with("927")
            || compact.starts_with("930")
            || compact.starts_with("936")
            || compact.starts_with("937")
            || compact.starts_with("950")
            || compact.starts_with("9611")
            || compact.starts_with("9612")
            || compact.starts_with("9622")
            || compact.starts_with("6129");
        // Reject trivially repetitive digit strings like `94000000000000000000` if all trailing digits are identical.
        let non_trivial = compact.as_bytes()[4..].windows(2).any(|w| w[0] != w[1]);
        if known_prefix && non_trivial {
            return Some(compact);
        }
    }

    None
}

fn detect_phone_number(s: &str) -> Option<String> {
    if s.contains('\n') || s.contains('\r') {
        return None;
    }
    let raw = strip_ascii_case_prefix(s, "tel:").map(str::trim).unwrap_or(s);
    if raw.is_empty() || raw.len() > 32 {
        return None;
    }

    // Reject anything with letters, currency symbols, slashes, colons, commas, or percent signs.
    for ch in raw.chars() {
        if !(ch.is_ascii_digit() || matches!(ch, '+' | '-' | ' ' | '(' | ')' | '.')) {
            return None;
        }
    }

    let has_plus = raw.starts_with('+');
    // `+` is only valid as the very first character.
    if raw[usize::from(has_plus)..].contains('+') {
        return None;
    }

    // Parentheses must be balanced and contain 2..=5 digits (area code).
    let open_parens = raw.bytes().filter(|&b| b == b'(').count();
    let close_parens = raw.bytes().filter(|&b| b == b')').count();
    if open_parens != close_parens || open_parens > 1 {
        return None;
    }
    if open_parens == 1 {
        let after_open = raw.split_once('(')?.1;
        let (inside, _) = after_open.split_once(')')?;
        let inside_digits = inside.bytes().filter(|b| b.is_ascii_digit()).count();
        if !(2..=5).contains(&inside_digits) || !inside.bytes().all(|b| b.is_ascii_digit() || b == b' ') {
            return None;
        }
    }

    // Dots are only allowed in NANP `NNN.NNN.NNNN` or `+1.NNN.NNN.NNNN`, never decimals/IPs/dates.
    let dot_count = raw.bytes().filter(|&b| b == b'.').count();
    if dot_count > 0 {
        let parts: Vec<&str> = raw.trim_start_matches('+').split('.').collect();
        let valid_dot_nanp = match parts.as_slice() {
            [a, b, c] => a.len() == 3 && b.len() == 3 && c.len() == 4,
            ["1", a, b, c] => a.len() == 3 && b.len() == 3 && c.len() == 4,
            _ => false,
        } && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()));
        if !valid_dot_nanp {
            return None;
        }
    }

    // Reject ISO or numeric dates like `2026-10-08` or `10-08-2026`.
    let hyphen_parts: Vec<&str> = raw.split('-').collect();
    if hyphen_parts.len() == 3
        && hyphen_parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        && ((hyphen_parts[0].len() == 4 && hyphen_parts[1].len() <= 2 && hyphen_parts[2].len() <= 2)
            || (hyphen_parts[0].len() <= 2 && hyphen_parts[1].len() <= 2 && hyphen_parts[2].len() == 4))
    {
        return None;
    }

    // Reject SSN-like `NNN-NN-NNNN` (9 digits) and 8-digit hyphenated codes `NNNN-NNNN`.
    if !has_plus
        && hyphen_parts.len() == 3
        && hyphen_parts[0].len() == 3
        && hyphen_parts[1].len() == 2
        && hyphen_parts[2].len() == 4
    {
        return None;
    }
    if !has_plus && hyphen_parts.len() == 2 && hyphen_parts[0].len() == 4 && hyphen_parts[1].len() == 4 {
        return None;
    }

    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    let digit_count = digits.len();

    if has_plus {
        // International E.164: country code starts with 1..=9, total 8..=15 digits.
        if !(8..=15).contains(&digit_count) || digits.starts_with('0') {
            return None;
        }
        return Some(format!("+{digits}"));
    }

    let has_separators = raw.bytes().any(|b| matches!(b, b'-' | b' ' | b'(' | b')' | b'.'));
    if has_separators {
        // Formatted national / domestic number: 10..=11 digits (e.g. `(415) 555-0199`, `1-800-555-0199`, `020 7946 0958`).
        if !(10..=11).contains(&digit_count) {
            return None;
        }
        if digit_count == 11 && !digits.starts_with(['0', '1']) {
            return None;
        }
        if digit_count == 10 && digits.starts_with('1') {
            // 10-digit national numbers in NANP/IN start with 2..=9, or 0 in UK/EU/AU/JP.
            return None;
        }
        return Some(digits);
    }

    // Bare unseparated digits: only accept unambiguous 10-digit national numbers starting with 2..=9
    // (or 11-digit UK/domestic numbers starting with `07` / `01` / `02` / `03` / `04`),
    // and reject all-identical digit runs or sequential runs.
    let all_same = digits.as_bytes().windows(2).all(|w| w[0] == w[1]);
    if all_same || digits == "0123456789" || digits == "1234567890" || digits == "9876543210" {
        return None;
    }
    if digit_count == 10 && matches!(digits.as_bytes()[0], b'2'..=b'9') {
        // NANP / India 10-digit mobile/landline: second-to-fourth digit should not make it look like a round integer `2000000000`.
        let trailing_zeros = digits.bytes().rev().take_while(|&b| b == b'0').count();
        if trailing_zeros >= 6 {
            return None;
        }
        return Some(digits);
    }
    if digit_count == 11 && matches!(&digits[..2], "01" | "02" | "03" | "04" | "07" | "08" | "09") {
        let trailing_zeros = digits.bytes().rev().take_while(|&b| b == b'0').count();
        if trailing_zeros >= 6 {
            return None;
        }
        return Some(digits);
    }

    None
}

/// Street/road designator words across English, French, German, Spanish, Italian, and Indian addresses.
const STREET_DESIGNATORS: &[&str] = &[
    "allee",
    "allée",
    "ave",
    "avenida",
    "avenue",
    "bagh",
    "blvd",
    "boulevard",
    "calle",
    "chemin",
    "chowk",
    "cir",
    "circle",
    "close",
    "colony",
    "corso",
    "court",
    "cours",
    "cres",
    "crescent",
    "ct",
    "damm",
    "dr",
    "drive",
    "enclave",
    "gardens",
    "gasse",
    "grove",
    "highway",
    "hwy",
    "impasse",
    "lane",
    "ln",
    "marg",
    "nagar",
    "parkway",
    "paseo",
    "piazza",
    "pkwy",
    "pl",
    "place",
    "platz",
    "puram",
    "quai",
    "rd",
    "ring",
    "road",
    "route",
    "rua",
    "rue",
    "salai",
    "sector",
    "sq",
    "square",
    "st",
    "str",
    "strasse",
    "straße",
    "street",
    "ter",
    "terrace",
    "ufer",
    "via",
    "viale",
    "vihar",
    "way",
    "weg",
];

/// Words that indicate ordinary prose / sentences rather than a street address.
const PROSE_STOPWORDS: &[&str] = &[
    "about",
    "after",
    "again",
    "always",
    "because",
    "before",
    "between",
    "could",
    "did",
    "does",
    "doing",
    "down",
    "during",
    "error",
    "every",
    "failed",
    "found",
    "going",
    "have",
    "having",
    "here",
    "however",
    "into",
    "just",
    "know",
    "like",
    "make",
    "many",
    "miles",
    "minutes",
    "more",
    "most",
    "much",
    "must",
    "need",
    "never",
    "only",
    "other",
    "over",
    "please",
    "really",
    "right",
    "said",
    "same",
    "should",
    "since",
    "some",
    "still",
    "such",
    "take",
    "than",
    "thank",
    "thanks",
    "that",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "think",
    "this",
    "those",
    "through",
    "today",
    "tomorrow",
    "under",
    "until",
    "very",
    "wait",
    "waited",
    "want",
    "warning",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "will",
    "winner",
    "with",
    "would",
    "yesterday",
    "your",
];

fn detect_street_address(s: &str) -> Option<String> {
    if s.len() < 8 || s.len() > 200 {
        return None;
    }
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() || lines.len() > 4 {
        return None;
    }
    // Reject code, URLs, paths, emails, and shell snippets.
    if s.contains("://")
        || s.contains('@')
        || s.contains('\\')
        || s.chars()
            .any(|c| matches!(c, '{' | '}' | '[' | ']' | '<' | '>' | ';' | '=' | '|' | '$' | '`' | '?' | '!'))
    {
        return None;
    }

    let normalized = lines.join(", ");
    let words: Vec<&str> = normalized
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '/'))
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 3 || words.len() > 24 {
        return None;
    }

    let mut has_house_number = false;
    let mut has_street_designator = false;

    for w in &words {
        let clean = w.trim_matches(|c: char| matches!(c, '.' | '#' | '(' | ')' | '-'));
        if clean.is_empty() {
            continue;
        }
        let lower = clean.to_lowercase();
        if PROSE_STOPWORDS.binary_search(&lower.as_str()).is_ok() {
            return None;
        }
        if is_house_number_token(clean) {
            has_house_number = true;
        }
        if is_street_designator(&lower) {
            has_street_designator = true;
        }
    }

    if !has_house_number || !has_street_designator {
        return None;
    }

    // Require either:
    // (a) multi-segment address (comma or multiple lines) or postal code, OR
    // (b) compact single-line street address (`<house_no> <Capitalized Street> <Designator> [Dir]`)
    //     or European `<StreetDesignator> <house_no>`.
    let has_comma_or_multiline = normalized.contains(',') || lines.len() >= 2;
    let has_postcode = has_postal_code(&normalized);
    let first_clean = words[0].trim_matches(|c: char| matches!(c, '.' | '#'));
    let starts_with_house_no = is_house_number_token(first_clean)
        && words.get(1).is_some_and(|w| w.chars().next().is_some_and(char::is_uppercase));
    let starts_with_german_street = is_street_designator(&words[0].trim_matches('.').to_lowercase())
        && words.get(1).is_some_and(|w| is_house_number_token(w.trim_matches('.')));

    if has_comma_or_multiline
        || has_postcode
        || (words.len() <= 7 && (starts_with_house_no || starts_with_german_street))
    {
        Some(normalized)
    } else {
        None
    }
}

fn is_house_number_token(token: &str) -> bool {
    if token.is_empty() || token.len() > 8 {
        return false;
    }
    let bytes = token.as_bytes();
    if !bytes[0].is_ascii_digit() || bytes[0] == b'0' {
        return false;
    }
    let digit_prefix_len = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if !(1..=5).contains(&digit_prefix_len) {
        return false;
    }
    let rest = &token[digit_prefix_len..];
    if rest.is_empty() {
        return true;
    }
    // Reject ordinals like `1st`, `2nd`, `3rd`, `4th` (e.g. `"3rd place"`), unless followed by more words.
    if matches!(rest.to_ascii_lowercase().as_str(), "st" | "nd" | "rd" | "th") {
        return false;
    }
    // Allow single letter suffix (`221B`, `10A`) or hyphenated range (`12-14`).
    if rest.len() == 1 && rest.as_bytes()[0].is_ascii_alphabetic() {
        return true;
    }
    if let Some(after_dash) = rest.strip_prefix('-') {
        return !after_dash.is_empty()
            && after_dash.len() <= 4
            && after_dash.bytes().all(|b| b.is_ascii_alphanumeric());
    }
    false
}

fn is_street_designator(lower: &str) -> bool {
    if STREET_DESIGNATORS.binary_search(&lower).is_ok() {
        return true;
    }
    // German compound street names like `friedrichstraße`, `kurfürstendamm`, `marienplatz`, `königsallee`, `holzweg`.
    for suffix in ["straße", "strasse", "platz", "allee", "gasse", "damm", "weg"] {
        if lower.len() >= suffix.len() + 3 && lower.ends_with(suffix) {
            return true;
        }
    }
    false
}

fn has_postal_code(s: &str) -> bool {
    let tokens: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim_matches('.'))
        .filter(|t| !t.is_empty())
        .collect();
    for (i, &t) in tokens.iter().enumerate() {
        // US 5-digit or ZIP+4 (`94043`, `94043-1351`), DE/FR 5-digit (`10117`, `75004`), IN 6-digit (`560001`).
        if (t.len() == 5 || t.len() == 6) && t.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
        if t.len() == 10
            && t.as_bytes()[5] == b'-'
            && t[..5].bytes().all(|b| b.is_ascii_digit())
            && t[6..].bytes().all(|b| b.is_ascii_digit())
        {
            return true;
        }
        // UK outward + inward (`SW1A 2AA`, `NW1 6XE`, `EC1A 1BB`) or Canadian (`M5V 2T6`, `M5H 2N2`).
        if let Some(&next) = tokens.get(i + 1)
            && next.len() == 3
            && next.as_bytes()[0].is_ascii_digit()
            && next.as_bytes()[1].is_ascii_alphabetic()
            && next.as_bytes()[2].is_ascii_alphabetic()
            && (2..=4).contains(&t.len())
            && t.as_bytes()[0].is_ascii_alphabetic()
            && t.bytes().any(|b| b.is_ascii_digit())
        {
            return true;
        }
        if let Some(&next) = tokens.get(i + 1)
            && t.len() == 3
            && next.len() == 3
            && t.as_bytes()[0].is_ascii_alphabetic()
            && t.as_bytes()[1].is_ascii_digit()
            && t.as_bytes()[2].is_ascii_alphabetic()
            && next.as_bytes()[0].is_ascii_digit()
            && next.as_bytes()[1].is_ascii_alphabetic()
            && next.as_bytes()[2].is_ascii_digit()
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_vs_file_paths_and_text() {
        let positives = [
            ("https://nectarlink.app/docs?tab=1#top", "https://nectarlink.app/docs?tab=1#top"),
            ("  http://example.com/path/to/page  ", "http://example.com/path/to/page"),
            ("HTTPS://github.com/thtbee/nectarlink", "https://github.com/thtbee/nectarlink"),
            (
                "www.wikipedia.org/wiki/Rust_(programming_language)",
                "https://www.wikipedia.org/wiki/Rust_(programming_language)",
            ),
            ("http://192.168.1.1:8080/admin", "http://192.168.1.1:8080/admin"),
            ("http://localhost:3000/api/health", "http://localhost:3000/api/health"),
        ];
        for (input, expected) in positives {
            let got = classify_clip(input);
            assert_eq!(
                got,
                Some(ClipSuggestion { kind: ClipKind::WebLink, target: expected.into() }),
                "expected WebLink for {input:?}"
            );
        }

        let negatives = [
            r"C:\Users\anive\Documents\report.pdf",
            "C:/Users/anive/Documents/report.pdf",
            r"\\server\share\folder\file.txt",
            "/usr/local/bin/nectarlink",
            "~/Downloads/photo.jpg",
            "file:///C:/Users/anive/test.html",
            "ftp://files.example.com/pub/archive.zip",
            "https://",
            "http://not_a_host",
            "Check out https://example.com and let me know",
        ];
        for input in negatives {
            let got = classify_clip(input);
            assert_ne!(
                got.as_ref().map(|s| s.kind),
                Some(ClipKind::WebLink),
                "should not classify {input:?} as WebLink"
            );
        }
    }

    #[test]
    fn emails_vs_handles_and_filenames() {
        let positives = [
            ("alex.chen+work@example.com", "alex.chen+work@example.com"),
            ("  support@nectarlink.dev  ", "support@nectarlink.dev"),
            ("mailto:billing@company.co.uk?subject=Invoice", "billing@company.co.uk"),
        ];
        for (input, expected) in positives {
            let got = classify_clip(input);
            assert_eq!(
                got,
                Some(ClipSuggestion { kind: ClipKind::Email, target: expected.into() }),
                "expected Email for {input:?}"
            );
        }

        let negatives = [
            "@username",
            "git@github.com:thtbee/nectarlink.git",
            "banner@2x.png",
            "user@localhost",
            "user@@example.com",
            ".user@example.com",
            "user@example",
        ];
        for input in negatives {
            let got = classify_clip(input);
            assert_eq!(got, None, "expected None for {input:?}, got {got:?}");
        }
    }

    #[test]
    fn phone_numbers_vs_amounts_dates_and_otps() {
        let positives = [
            ("+1 (415) 555-0199", "+14155550199"),
            ("+15550100", "+15550100"),
            ("+44 20 7946 0958", "+442079460958"),
            ("+91 98765 43210", "+919876543210"),
            ("+49 30 901820", "+4930901820"),
            ("(415) 555-0199", "4155550199"),
            ("415-555-0199", "4155550199"),
            ("415.555.0199", "4155550199"),
            ("1-800-555-0199", "18005550199"),
            ("020 7946 0958", "02079460958"),
            ("9845012345", "9845012345"),
            ("tel:+1-800-555-0130", "+18005550130"),
        ];
        for (input, expected) in positives {
            let got = classify_clip(input);
            assert_eq!(
                got,
                Some(ClipSuggestion { kind: ClipKind::PhoneNumber, target: expected.into() }),
                "expected PhoneNumber for {input:?}"
            );
        }

        let negatives = [
            // One-time codes (must get no chip!)
            "482910",
            "739-201",
            "G-582910",
            "Your verification code is 839201",
            // Hyphenated 8-digit non-OTP
            "1234-5678",
            // SSN
            "123-45-6789",
            // Dates & times
            "2026-10-08",
            "08-10-2026",
            "2026.10.08",
            "10/08/2026",
            // Currency amounts & decimals
            "$1,234,567.89",
            "12,345,678",
            "€49.99",
            "₹1,50,000",
            "123456.78",
            // IPv4 & versions
            "192.168.1.100",
            "10.0.0.1",
            // Unix timestamps / bare 10-digit starting with 1 / round or sequential numbers
            "1700000000",
            "2000000000",
            "1234567890",
            "9876543210",
            "0000000000",
        ];
        for input in negatives {
            let got = classify_clip(input);
            assert_eq!(got, None, "expected None for {input:?}, got {got:?}");
        }
    }

    #[test]
    fn tracking_numbers_vs_ordinary_long_numbers() {
        let positives = [
            // UPS
            ("1Z999AA10123456784", "1Z999AA10123456784"),
            ("1z 999 aa1 01 2345 6784", "1Z999AA10123456784"),
            // USPS 20-22 digits
            ("9400 1000 0000 0000 0000 12", "9400100000000000000012"),
            ("9205590175547700001234", "9205590175547700001234"),
            // UPU S10 international (US, UK, DE, IN, CA, JP)
            ("EA123456789US", "EA123456789US"),
            ("RR 987 654 321 GB", "RR987654321GB"),
            ("CE123456789DE", "CE123456789DE"),
            ("EE123456789IN", "EE123456789IN"),
            ("LX123456789CA", "LX123456789CA"),
            // Amazon Logistics
            ("TBA123456789012", "TBA123456789012"),
            // FedEx Door Tag & Ground 22-digit
            ("DT123456789012", "DT123456789012"),
            ("9612019999999999999912", "9612019999999999999912"),
            // DHL Germany / eCommerce
            ("JJD0003900012345678", "JJD0003900012345678"),
        ];
        for (input, expected) in positives {
            let got = classify_clip(input);
            assert_eq!(
                got,
                Some(ClipSuggestion { kind: ClipKind::TrackingNumber, target: expected.into() }),
                "expected TrackingNumber for {input:?}"
            );
        }

        let negatives = [
            // Ordinary long numbers, credit cards, bank accounts, IMEI
            "123456789012",
            "123456789012345",
            "4111 1111 1111 1111",
            "4532-0151-1283-0366",
            "12345678901234567890",
            // Random 13-char string with non-country suffix
            "AB123456789ZZ",
            "XY123456789US",
        ];
        for input in negatives {
            let got = classify_clip(input);
            assert_eq!(got, None, "expected None for {input:?}, got {got:?}");
        }
    }

    #[test]
    fn street_addresses_across_countries_vs_sentences() {
        let positives = [
            // US
            "1600 Amphitheatre Parkway, Mountain View, CA 94043",
            "350 Fifth Avenue\nNew York, NY 10118",
            "742 Evergreen Terrace",
            // UK
            "221B Baker Street, London NW1 6XE",
            "10 Downing Street, London, SW1A 2AA",
            // Canada
            "100 Queen St W, Toronto, ON M5H 2N2",
            // Germany
            "Friedrichstraße 123, 10117 Berlin",
            "Marienplatz 1, 80331 München",
            // France
            "24 Rue de Rivoli, 75004 Paris",
            // India
            "42 MG Road, Bengaluru, Karnataka 560001",
            // Australia
            "300 George St, Sydney NSW 2000",
        ];
        for input in positives {
            let got = classify_clip(input);
            assert_eq!(
                got.as_ref().map(|s| s.kind),
                Some(ClipKind::StreetAddress),
                "expected StreetAddress for {input:?}, got {got:?}"
            );
        }

        let negatives = [
            "I drove 2 miles down the road",
            "3rd place winner in the tournament",
            "Please wait 5 minutes on the way",
            "Meeting at 3pm tomorrow",
            "let x = 100; // main street variable",
            "Error at line 42: connection reset by peer",
        ];
        for input in negatives {
            let got = classify_clip(input);
            assert_eq!(got, None, "expected None for {input:?}, got {got:?}");
        }
    }

    #[test]
    fn debug_does_not_leak_target() {
        let s = ClipSuggestion { kind: ClipKind::PhoneNumber, target: "+14155550199".into() };
        let dbg = format!("{s:?}");
        assert!(dbg.contains("PhoneNumber"));
        assert!(!dbg.contains("4155550199"), "Debug leaked target: {dbg}");
    }
}
