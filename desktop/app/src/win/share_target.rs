// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows Share sheet target (`windows.shareTarget`): receives files, web
//! links, text, or bitmaps shared from any Windows app (when registered via the
//! Sparse Package `AppxManifest.xml`) or passed on the command line with
//! `--share <target…>`.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use windows::{
    ApplicationModel::{
        Activation::{ActivationKind, IShareTargetActivatedEventArgs},
        AppInstance,
        DataTransfer::StandardDataFormats,
    },
    Storage::Streams::DataReader,
    core::Interface,
};

/// Command-line flag to share files, URLs, or text with a paired phone.
pub const ARG: &str = "--share";

/// Payload extracted from a Windows Share sheet activation or `--share` CLI invocation.
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

fn is_share_url(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 4096 || s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower: String = s.chars().take(8).collect::<String>().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || (lower.starts_with("geo:") && s.len() > 4)
}

fn looks_like_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    Path::new(s).exists()
        || s.contains('\\')
        || s.contains('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Classifies `--share <target…>` CLI arguments into files, a link, or text.
pub fn parse_targets(targets: &[String]) -> SharedPayload {
    let mut payload = SharedPayload::default();
    let mut text_parts = Vec::new();
    for raw in targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if payload.url.is_none() && is_share_url(trimmed) {
            payload.url = Some(trimmed.to_owned());
        } else if looks_like_path(trimmed) {
            payload.files.push(PathBuf::from(trimmed));
        } else {
            text_parts.push(trimmed.to_owned());
        }
    }
    if !text_parts.is_empty() {
        payload.text = Some(text_parts.join(" "));
    }
    payload
}

/// Checks if the current process was activated by the Windows Share sheet
/// (`ActivationKind::ShareTarget`). If so, reads the shared storage items,
/// link, text, or bitmap, marks the share operation completed, and returns
/// the extracted [`SharedPayload`].
pub fn take_activated_share(data_dir: &Path) -> Option<SharedPayload> {
    super::with_com(|| {
        let args = AppInstance::GetActivatedEventArgs().ok()?;
        if args.Kind().ok()? != ActivationKind::ShareTarget {
            return None;
        }
        let share_args: IShareTargetActivatedEventArgs = args.cast().ok()?;
        let share_op = share_args.ShareOperation().ok()?;
        let data = share_op.Data().ok()?;
        let mut payload = SharedPayload::default();

        if let Ok(fmt) = StandardDataFormats::StorageItems()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetStorageItemsAsync()
            && let Ok(items) = op.join()
        {
            for item in items {
                if let Ok(path) = item.Path() {
                    let s = path.to_string();
                    if !s.is_empty() {
                        payload.files.push(PathBuf::from(s));
                    }
                }
            }
        }

        if payload.url.is_none()
            && let Ok(fmt) = StandardDataFormats::WebLink()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetWebLinkAsync()
            && let Ok(uri) = op.join()
            && let Ok(raw) = uri.RawUri()
        {
            let s = raw.to_string();
            if !s.is_empty() {
                payload.url = Some(s);
            }
        }

        if payload.url.is_none()
            && let Ok(fmt) = StandardDataFormats::Uri()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetUriAsync()
            && let Ok(uri) = op.join()
            && let Ok(raw) = uri.RawUri()
        {
            let s = raw.to_string();
            if !s.is_empty() {
                payload.url = Some(s);
            }
        }

        if let Ok(fmt) = StandardDataFormats::Text()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetTextAsync()
            && let Ok(text) = op.join()
        {
            let s = text.to_string();
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                if payload.url.is_none() && is_share_url(trimmed) {
                    payload.url = Some(trimmed.to_owned());
                } else {
                    payload.text = Some(s);
                }
            }
        }

        if payload.files.is_empty()
            && let Ok(fmt) = StandardDataFormats::Bitmap()
            && data.Contains(&fmt).unwrap_or(false)
            && let Some(saved) = save_shared_bitmap(&data, data_dir)
        {
            payload.files.push(saved);
        }

        let _ = share_op.ReportCompleted();
        (!payload.is_empty()).then_some(payload)
    })
}

fn save_shared_bitmap(
    data: &windows::ApplicationModel::DataTransfer::DataPackageView,
    data_dir: &Path,
) -> Option<PathBuf> {
    let stream_ref = data.GetBitmapAsync().ok()?.join().ok()?;
    let stream = stream_ref.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(u32::try_from(size).unwrap_or(u32::MAX)).ok()?.join().ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let png = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bytes
    } else {
        let bmp = super::image::decode(&bytes).ok()?;
        super::image::encode_png(&bmp).ok()?
    };
    let dir = data_dir.join("cache").join("shared");
    std::fs::create_dir_all(&dir).ok()?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let path = dir.join(format!("shared-{nanos}.png"));
    std::fs::write(&path, png).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use windows::{Data::Xml::Dom::XmlDocument, core::HSTRING};

    use super::*;

    #[test]
    fn parses_cli_share_targets() {
        let files = parse_targets(&[r"C:\Users\Alice\Pictures\photo.jpg".into(), r"D:\notes.txt".into()]);
        assert_eq!(
            files,
            SharedPayload {
                files: vec![
                    PathBuf::from(r"C:\Users\Alice\Pictures\photo.jpg"),
                    PathBuf::from(r"D:\notes.txt")
                ],
                url: None,
                text: None,
            }
        );

        let link = parse_targets(&["https://example.com/watch?v=42".into()]);
        assert_eq!(
            link,
            SharedPayload {
                files: Vec::new(),
                url: Some("https://example.com/watch?v=42".into()),
                text: None,
            }
        );

        let text = parse_targets(&["Meet me at 7pm".into()]);
        assert_eq!(text, SharedPayload { files: Vec::new(), url: None, text: Some("Meet me at 7pm".into()) });
    }

    #[test]
    fn sparse_package_manifest_is_valid_xml_with_share_target() {
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../installer/AppxManifest.xml");
        let xml_str = std::fs::read_to_string(&manifest_path).expect("read AppxManifest.xml");
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(&xml_str)).expect("valid AppxManifest.xml");
        assert!(xml_str.contains(r#"Name="Nectarlink.Desktop""#));
        assert!(xml_str.contains("uap10:AllowExternalContent"));
        assert!(xml_str.contains(r#"Category="windows.shareTarget""#));
        for format in ["StorageItems", "WebLink", "Uri", "Text", "Bitmap"] {
            assert!(xml_str.contains(&format!("<uap:DataFormat>{format}</uap:DataFormat>")));
        }
        // Without braces: the schema rejects them, and the package then never registers (#84).
        assert!(xml_str.contains(r#"ToastActivatorCLSID="7B9E0E64-5C5A-4E8B-9D31-5A4C2B10F6E1""#));
    }
}
