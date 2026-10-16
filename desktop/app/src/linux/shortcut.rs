use std::env;
use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub target: PathBuf,
    pub arguments: String,
    pub description: String,
}

pub fn write(path: &Path, shortcut: &Shortcut) -> Result<()> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=NectarLink\nComment={}\nExec=\"{}\" {}\nX-NectarLink-Args={}\nTerminal=false\n",
        shortcut.description.replace('\n', " "),
        shortcut.target.display(),
        shortcut.arguments,
        shortcut.arguments,
    );
    fs::write(path, content)
}

pub fn arguments(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if let Some(args) = line.strip_prefix("X-NectarLink-Args=") {
            return Some(args.to_string());
        }
    }
    None
}

pub fn send_to_dir() -> Option<PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data_home.join("nautilus/scripts"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn write_and_read_shortcut_arguments() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("nectarlink-shortcut-{stamp}.lnk"));
        let shortcut = Shortcut {
            target: PathBuf::from("/usr/bin/nectarlink"),
            arguments: "--data-dir \"/tmp/custom data\"".to_string(),
            description: "NectarLink".to_string(),
        };

        write(&path, &shortcut).expect("write shortcut");
        assert_eq!(
            arguments(&path).as_deref(),
            Some("--data-dir \"/tmp/custom data\"")
        );
        let _ = fs::remove_file(path);
    }
}
