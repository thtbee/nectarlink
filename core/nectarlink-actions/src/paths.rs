// SPDX-License-Identifier: MPL-2.0
//! Paths an agent passes for files on this PC. Only plain absolute paths,
//! resolved first, and never into places that hold keys or logins.

use std::path::{Component, Path, PathBuf};

use crate::error::ActionError;

/// Folder names that hold keys, logins or browser data. Matched anywhere in
/// the resolved path, ignoring case.
const NEVER: [&str; 9] =
    [".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".password-store", "keychains", "credentials"];

/// Folder pairs for browser profiles and NectarLink's own data (its keys and tokens).
const NEVER_PAIRS: [(&str, &str); 7] = [
    ("google", "chrome"),
    (".mozilla", "firefox"),
    ("microsoft", "edge"),
    ("mozilla", "firefox"),
    ("bravesoftware", "brave-browser"),
    ("thtbee", "nectarlink"),
    ("app.nectarlink", "nectarlink"),
];

/// The path, resolved, if an agent may use it.
pub fn checked(raw: &str) -> Result<PathBuf, ActionError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(ActionError::invalid(format!("{raw:?} isn't an absolute path.")));
    }
    if raw.starts_with(r"\\") || raw.starts_with("//") {
        return Err(ActionError::invalid("Network and device paths can't be used."));
    }
    let resolved = path
        .canonicalize()
        .map_err(|_| ActionError::new(crate::error::Code::NotFound, format!("{raw:?} doesn't exist.")))?;
    if is_private(&resolved) {
        return Err(ActionError::new(
            crate::error::Code::Denied,
            "That folder holds keys or logins, so agents can't use it.",
        ));
    }
    Ok(resolved)
}

fn is_private(path: &Path) -> bool {
    let names: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    if names.iter().any(|n| NEVER.contains(&n.as_str())) {
        return true;
    }
    if names.iter().any(|n| n == "nectarlink" || n == ".nectarlink")
        && names
            .iter()
            .any(|n| matches!(n.as_str(), "appdata" | ".local" | ".config" | "application support"))
    {
        return true;
    }
    names.windows(2).any(|w| NEVER_PAIRS.iter().any(|(a, b)| w[0] == *a && w[1] == *b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_and_login_folders_are_refused() {
        for p in [
            "/home/bee/.ssh/id_ed25519",
            "/home/bee/.config/nectarlink/identity.key",
            r"C:\Users\bee\AppData\Roaming\Nectarlink\api-token.json",
            r"C:\Users\bee\AppData\Local\Google\Chrome\User Data\Default\Login Data",
            "/home/bee/.mozilla/firefox/abc.default/logins.json",
        ] {
            assert!(is_private(Path::new(p)), "{p}");
        }
        for p in [
            "/home/bee/Documents/report.pdf",
            r"C:\Users\bee\Pictures\cat.png",
            "/home/bee/nectarlink-src/README.md",
        ] {
            assert!(!is_private(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn relative_network_and_missing_paths_are_refused() {
        assert!(checked("relative/file.txt").is_err());
        assert!(checked(r"\\server\share\x").is_err());
        let missing = std::env::temp_dir().join("nectarlink-no-such-file-7f3a");
        assert!(checked(&missing.to_string_lossy()).is_err());
        let real = std::env::temp_dir();
        assert!(checked(&real.to_string_lossy()).is_ok());
    }
}
