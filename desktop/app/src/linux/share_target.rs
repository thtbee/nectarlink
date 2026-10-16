use std::path::{Path, PathBuf};

pub const ARG: &str = "--share";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedPayload {
    pub files: Vec<PathBuf>,
    pub url: Option<String>,
    pub text: Option<String>,
}

impl SharedPayload {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.url.is_none() && self.text.is_none()
    }
}

pub fn parse_targets(targets: &[String]) -> SharedPayload {
    let mut payload = SharedPayload::default();
    let mut text_parts: Vec<String> = Vec::new();

    for raw in targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_http_url(trimmed) && payload.url.is_none() {
            payload.url = Some(trimmed.to_string());
            continue;
        }
        if let Some(file_path) = parse_file_uri_or_path(trimmed) {
            payload.files.push(file_path);
            continue;
        }
        text_parts.push(trimmed.to_string());
    }

    if !text_parts.is_empty() {
        payload.text = Some(text_parts.join("\n"));
    }

    payload
}

pub fn take_activated_share(_data_dir: &Path) -> Option<SharedPayload> {
    None
}

fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && !value.chars().any(char::is_whitespace)
}

fn parse_file_uri_or_path(value: &str) -> Option<PathBuf> {
    let unquoted = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value);
    if let Some(rest) = unquoted.strip_prefix("file:///") {
        let decoded = percent_decode(rest);
        let path_str = if decoded.as_bytes().get(1) == Some(&b':') {
            decoded
        } else {
            format!("/{}", decoded.trim_start_matches('/'))
        };
        return Some(PathBuf::from(path_str));
    }
    let path = PathBuf::from(unquoted);
    let bytes = unquoted.as_bytes();
    if path.is_absolute()
        || path.exists()
        || unquoted.contains('\\')
        || unquoted.contains('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
    {
        return Some(path);
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[idx + 1]), from_hex(bytes[idx + 2]))
        {
            out.push((hi << 4) | lo);
            idx += 3;
            continue;
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_targets_splits_files_urls_and_text() {
        let targets = vec![
            "/home/alice/Document.pdf".to_string(),
            "https://example.com/article?id=42".to_string(),
            "Hello from share target".to_string(),
        ];
        let payload = parse_targets(&targets);
        assert_eq!(payload.files, vec![PathBuf::from("/home/alice/Document.pdf")]);
        assert_eq!(
            payload.url.as_deref(),
            Some("https://example.com/article?id=42")
        );
        assert_eq!(payload.text.as_deref(), Some("Hello from share target"));
    }

    #[test]
    fn parse_targets_decodes_file_uri() {
        let targets = vec!["file:///home/alice/My%20Photo.png".to_string()];
        let payload = parse_targets(&targets);
        assert_eq!(
            payload.files,
            vec![PathBuf::from("/home/alice/My Photo.png")]
        );
    }
}
