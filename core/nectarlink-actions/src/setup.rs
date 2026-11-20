// SPDX-License-Identifier: MPL-2.0
//! One-click setup: adds NectarLink to an agent app's MCP configuration.
//!
//! Only the command and the agent's name go in; the token stays in
//! NectarLink's own folder. The user's file keeps its formatting and every
//! other entry: the `nectarlink` entry is put in or replaced as text, the
//! result is checked to be valid, and the old file is kept beside it as
//! `.nectarlink-backup`. Files with comments (JSONC) aren't touched; the app
//! shows the snippet to paste instead.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The server's name in every configuration.
pub const SERVER: &str = "nectarlink";

/// Where configurations live on this system.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// The user's home folder.
    pub home: PathBuf,
    /// The roaming app data folder: `%APPDATA%` on Windows,
    /// `~/Library/Application Support` on macOS, `~/.config` on Linux.
    pub config: PathBuf,
}

impl Dirs {
    /// This system's folders.
    pub fn of_this_system() -> Option<Dirs> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
        let config = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)?
        } else if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"))
        };
        Some(Dirs { home, config })
    }
}

/// How an app's file holds its servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// JSON with the servers under `key`, each `{ command, args }` (plus
    /// `type: stdio` when `typed`).
    Json { key: &'static str, typed: bool },
    /// Codex's TOML.
    Toml,
}

/// An app NectarLink can add itself to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct App {
    pub kind: &'static str,
    format: Format,
}

/// The apps with a file NectarLink edits. Claude Code is set up with its own
/// command instead (see [`claude_code_args`]): it rewrites its file while running.
pub const APPS: [App; 8] = [
    App { kind: "claude-desktop", format: Format::Json { key: "mcpServers", typed: false } },
    App { kind: "cursor", format: Format::Json { key: "mcpServers", typed: false } },
    App { kind: "windsurf", format: Format::Json { key: "mcpServers", typed: false } },
    App { kind: "antigravity", format: Format::Json { key: "mcpServers", typed: false } },
    App { kind: "gemini", format: Format::Json { key: "mcpServers", typed: false } },
    App { kind: "vscode", format: Format::Json { key: "servers", typed: true } },
    App { kind: "zed", format: Format::Json { key: "context_servers", typed: false } },
    App { kind: "codex", format: Format::Toml },
];

pub fn app(kind: &str) -> Option<App> {
    APPS.into_iter().find(|a| a.kind == kind)
}

impl App {
    /// The file this app reads its MCP servers from.
    pub fn path(&self, dirs: &Dirs) -> PathBuf {
        let (home, config) = (&dirs.home, &dirs.config);
        match self.kind {
            "claude-desktop" => config.join("Claude").join("claude_desktop_config.json"),
            "cursor" => home.join(".cursor").join("mcp.json"),
            "windsurf" => home.join(".codeium").join("windsurf").join("mcp_config.json"),
            "antigravity" => home.join(".gemini").join("config").join("mcp_config.json"),
            "gemini" => home.join(".gemini").join("settings.json"),
            "vscode" => config.join("Code").join("User").join("mcp.json"),
            "zed" if cfg!(windows) => config.join("Zed").join("settings.json"),
            "zed" => home.join(".config").join("zed").join("settings.json"),
            _ => home.join(".codex").join("config.toml"),
        }
    }

    /// Whether the app looks installed: its folder is there.
    pub fn present(&self, dirs: &Dirs) -> bool {
        self.path(dirs).parent().is_some_and(Path::is_dir)
    }

    /// Whether NectarLink is in the app's file already.
    pub fn configured(&self, dirs: &Dirs) -> bool {
        let Ok(text) = std::fs::read_to_string(self.path(dirs)) else { return false };
        match self.format {
            Format::Toml => text.lines().any(|l| l.trim() == "[mcp_servers.nectarlink]"),
            Format::Json { key, .. } => serde_json::from_str::<Value>(&text)
                .ok()
                .is_some_and(|v| v.get(key).and_then(|s| s.get(SERVER)).is_some()),
        }
    }

    fn entry(&self, exe: &str, agent: &str) -> Value {
        let args = json!(["mcp", "--agent", agent]);
        match self.format {
            Format::Json { typed: true, .. } => json!({ "type": "stdio", "command": exe, "args": args }),
            _ => json!({ "command": exe, "args": args }),
        }
    }

    /// Adds NectarLink as agent `agent`, running `exe`, and returns the file.
    pub fn install(&self, dirs: &Dirs, exe: &str, agent: &str) -> Result<PathBuf, SetupError> {
        let path = self.path(dirs);
        if !self.present(dirs) {
            return Err(SetupError::NotInstalled);
        }
        let old = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(SetupError::Io(e.to_string())),
        };
        let new = match self.format {
            Format::Json { key, .. } => json_with(&old, key, &self.entry(exe, agent))?,
            Format::Toml => toml_with(&old, exe, agent),
        };
        if new == old {
            return Ok(path);
        }
        if !old.is_empty() {
            let backup = path.with_extension(format!(
                "{}.nectarlink-backup",
                path.extension().and_then(|e| e.to_str()).unwrap_or("")
            ));
            std::fs::write(&backup, &old).map_err(|e| SetupError::Io(e.to_string()))?;
        }
        let partial = path.with_extension("nectarlink-new");
        std::fs::write(&partial, &new).map_err(|e| SetupError::Io(e.to_string()))?;
        std::fs::rename(&partial, &path).map_err(|e| {
            let _ = std::fs::remove_file(&partial);
            SetupError::Io(e.to_string())
        })?;
        Ok(path)
    }
}

/// The `claude` arguments that add NectarLink to Claude Code for this user.
pub fn claude_code_args(exe: &str, agent: &str) -> Vec<String> {
    ["mcp", "add", "--scope", "user", SERVER, "--", exe, "mcp", "--agent", agent]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    #[error("the app doesn't seem to be installed")]
    NotInstalled,
    #[error("its settings file has comments or isn't plain JSON, so it was left alone")]
    NotPlainJson,
    #[error("{0}")]
    Io(String),
}

/// `text` (a JSON object, or empty) with `entry` as `root[key].nectarlink`,
/// everything else as it was.
fn json_with(text: &str, key: &str, entry: &Value) -> Result<String, SetupError> {
    let entry_text = serde_json::to_string_pretty(entry).unwrap_or_default();
    if text.trim().is_empty() {
        let fresh = json!({ key: { SERVER: entry } });
        return Ok(serde_json::to_string_pretty(&fresh).unwrap_or_default() + "\n");
    }
    let parsed: Value = serde_json::from_str(text).map_err(|_| SetupError::NotPlainJson)?;
    if !parsed.is_object() {
        return Err(SetupError::NotPlainJson);
    }
    if parsed.get(key).and_then(|s| s.get(SERVER)) == Some(entry) {
        return Ok(text.to_owned());
    }
    let root = object_members(text, first_brace(text, 0).ok_or(SetupError::NotPlainJson)?)
        .ok_or(SetupError::NotPlainJson)?;
    let out = match root.members.iter().find(|m| m.key == key) {
        Some(section) if text[section.value.clone()].trim_start().starts_with('{') => {
            let open = section.value.start + text[section.value.clone()].find('{').unwrap_or(0);
            let inner = object_members(text, open).ok_or(SetupError::NotPlainJson)?;
            match inner.members.iter().find(|m| m.key == SERVER) {
                Some(existing) => splice(text, existing.value.clone(), &indent(&entry_text, 4)),
                None => insert_member(text, &inner, SERVER, &indent(&entry_text, 4), 4),
            }
        }
        // There but not an object: not ours to fix.
        Some(_) => return Err(SetupError::NotPlainJson),
        None => {
            let section = serde_json::to_string_pretty(&json!({ SERVER: entry })).unwrap_or_default();
            insert_member(text, &root, key, &indent(&section, 2), 2)
        }
    };
    // Check the result before it's written.
    let check: Value = serde_json::from_str(&out).map_err(|_| SetupError::NotPlainJson)?;
    if check.get(key).and_then(|s| s.get(SERVER)) != Some(entry) {
        return Err(SetupError::NotPlainJson);
    }
    Ok(out)
}

struct Member {
    key: String,
    /// Where its value is in the text.
    value: std::ops::Range<usize>,
}

struct Object {
    /// The `{`.
    open: usize,
    /// The `}`.
    close: usize,
    members: Vec<Member>,
}

fn first_brace(text: &str, from: usize) -> Option<usize> {
    text[from..].find('{').map(|i| i + from)
}

/// The members of the JSON object whose `{` is at `open` (valid JSON only).
fn object_members(text: &str, open: usize) -> Option<Object> {
    let b = text.as_bytes();
    let mut i = open + 1;
    let mut members = Vec::new();
    loop {
        i = skip_ws(b, i);
        match *b.get(i)? {
            b'}' => return Some(Object { open, close: i, members }),
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(b, i)?;
        let key: String = serde_json::from_str(&text[i..key_end]).ok()?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        let start = skip_ws(b, i + 1);
        let end = value_end(b, start)?;
        members.push(Member { key, value: start..end });
        i = end;
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// Just past the string that starts at `i`.
fn string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Just past the value that starts at `i`.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = string_end(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']') && !b[j].is_ascii_whitespace() {
                j += 1;
            }
            Some(j)
        }
    }
}

fn splice(text: &str, range: std::ops::Range<usize>, with: &str) -> String {
    format!("{}{}{}", &text[..range.start], with, &text[range.end..])
}

/// `text` with every line after the first indented by `n` spaces.
fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.to_owned() } else { format!("{pad}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Adds `"key": value` as the object's last member.
fn insert_member(text: &str, object: &Object, key: &str, value: &str, pad: usize) -> String {
    let key = serde_json::to_string(key).unwrap_or_default();
    let spaces = " ".repeat(pad);
    match object.members.last() {
        Some(last) => {
            let at = last.value.end;
            format!("{},\n{spaces}{key}: {value}{}", &text[..at], &text[at..])
        }
        None => {
            let closing = " ".repeat(pad.saturating_sub(2));
            format!("{}\n{spaces}{key}: {value}\n{closing}{}", &text[..=object.open], &text[object.close..])
        }
    }
}

/// Codex's TOML with the NectarLink table put in or replaced.
fn toml_with(text: &str, exe: &str, agent: &str) -> String {
    let quoted = |s: &str| format!("'{}'", s.replace('\'', ""));
    let table = format!(
        "[mcp_servers.nectarlink]\ncommand = {}\nargs = [\"mcp\", \"--agent\", {}]\ntool_timeout_sec = 120\n",
        quoted(exe),
        quoted(agent)
    );
    let lines: Vec<&str> = text.lines().collect();
    if let Some(start) = lines.iter().position(|l| l.trim() == "[mcp_servers.nectarlink]") {
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map(|i| start + 1 + i)
            .unwrap_or(lines.len());
        let mut out: Vec<String> = lines[..start].iter().map(|l| (*l).to_owned()).collect();
        out.push(table.trim_end().to_owned());
        if end < lines.len() {
            out.push(String::new());
        }
        out.extend(lines[end..].iter().map(|l| (*l).to_owned()));
        return out.join("\n") + "\n";
    }
    let mut out = text.trim_end().to_owned();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&table);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Value {
        json!({ "command": "C:\\NL\\nectarlink-agent.exe", "args": ["mcp", "--agent", "cursor"] })
    }

    #[test]
    fn an_empty_file_gets_a_fresh_config() {
        let out = json_with("", "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["nectarlink"], entry());
    }

    #[test]
    fn other_servers_and_the_files_own_order_stay() {
        let text = "{\n  \"zeta\": 1,\n  \"mcpServers\": {\n    \"github\": { \"command\": \"gh\" }\n  },\n  \"alpha\": true\n}\n";
        let out = json_with(text, "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["nectarlink"], entry());
        assert_eq!(v["mcpServers"]["github"]["command"], "gh");
        assert!(out.find("zeta").unwrap() < out.find("alpha").unwrap(), "{out}");
        assert!(out.starts_with("{\n  \"zeta\": 1,"), "{out}");
    }

    #[test]
    fn a_missing_section_is_added_and_an_old_entry_replaced() {
        let out = json_with("{ \"theme\": \"dark\" }", "servers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["servers"]["nectarlink"], entry());
        assert_eq!(v["theme"], "dark");

        let old = "{\"mcpServers\":{\"nectarlink\":{\"command\":\"old.exe\"},\"x\":{}}}";
        let out = json_with(old, "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["nectarlink"], entry());
        assert!(v["mcpServers"]["x"].is_object());
        assert!(!out.contains("old.exe"));

        let empty_section = "{ \"mcpServers\": {} }";
        let v: Value =
            serde_json::from_str(&json_with(empty_section, "mcpServers", &entry()).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["nectarlink"], entry());
    }

    #[test]
    fn already_right_means_no_change_and_comments_mean_hands_off() {
        let done = serde_json::to_string(&json!({ "mcpServers": { "nectarlink": entry() } })).unwrap();
        assert_eq!(json_with(&done, "mcpServers", &entry()).unwrap(), done);
        assert_eq!(
            json_with("{\n  // my settings\n  \"a\": 1\n}", "mcpServers", &entry()),
            Err(SetupError::NotPlainJson)
        );
        assert_eq!(json_with("[1, 2]", "mcpServers", &entry()), Err(SetupError::NotPlainJson));
        assert_eq!(json_with("{\"mcpServers\": 5}", "mcpServers", &entry()), Err(SetupError::NotPlainJson));
    }

    #[test]
    fn tricky_strings_dont_confuse_the_scanner() {
        let text = r#"{ "a": "has } and { and \" inside", "mcpServers": { "b": ["]", "}"] } }"#;
        let v: Value = serde_json::from_str(&json_with(text, "mcpServers", &entry()).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["nectarlink"], entry());
        assert_eq!(v["a"], "has } and { and \" inside");
        assert_eq!(v["mcpServers"]["b"][1], "}");
    }

    #[test]
    fn codex_gets_its_table_once() {
        let first = toml_with("model = 'o5'\n", r"C:\NL\a.exe", "codex");
        assert!(first.starts_with("model = 'o5'\n\n[mcp_servers.nectarlink]\n"), "{first}");
        assert!(first.contains(r"command = 'C:\NL\a.exe'"), "{first}");
        let with_more = format!("{first}\n[mcp_servers.other]\ncommand = 'x'\n");
        let again = toml_with(&with_more, "/new/path", "codex");
        assert_eq!(again.matches("[mcp_servers.nectarlink]").count(), 1, "{again}");
        assert!(again.contains("command = '/new/path'"), "{again}");
        assert!(again.contains("[mcp_servers.other]\ncommand = 'x'"), "{again}");
        assert!(!again.contains("a.exe"), "{again}");
    }

    #[test]
    fn install_writes_a_backup_and_only_where_the_app_is() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs { home: dir.path().join("home"), config: dir.path().join("cfg") };
        let cursor = app("cursor").unwrap();
        assert_eq!(cursor.install(&dirs, "/x/agent", "cursor"), Err(SetupError::NotInstalled));
        std::fs::create_dir_all(dir.path().join("home").join(".cursor")).unwrap();
        let path = cursor.path(&dirs);
        std::fs::write(&path, "{\"mcpServers\":{\"other\":{}}}").unwrap();
        assert!(!cursor.configured(&dirs));
        cursor.install(&dirs, "/x/agent", "cursor").unwrap();
        assert!(cursor.configured(&dirs));
        let backup = std::fs::read_to_string(path.with_extension("json.nectarlink-backup")).unwrap();
        assert_eq!(backup, "{\"mcpServers\":{\"other\":{}}}");
        // Running it again changes nothing.
        let once = std::fs::read_to_string(&path).unwrap();
        cursor.install(&dirs, "/x/agent", "cursor").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }
}
