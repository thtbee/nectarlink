// SPDX-License-Identifier: GPL-3.0-or-later
//! `cargo xtask notices <out> [--target <triple>]`: the third-party notices
//! shipped with the Windows app. Every crate compiled into the app (normal
//! dependencies for that target; not build or dev ones) with its license
//! and the license files it ships, then Qt and the bundled fonts.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const ROOT_PACKAGE: &str = "nectarlink-desktop";
const DEFAULT_TARGET: &str = "x86_64-pc-windows-msvc";

/// License-ish files a crate ships.
fn is_license_file(name: &str) -> bool {
    let upper = name.to_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"].iter().any(|p| upper.starts_with(p))
}

struct Crate {
    name: String,
    version: String,
    license: String,
    repository: String,
    files: Vec<(String, String)>,
}

pub fn run(args: &[&str]) -> Result<()> {
    let (out, target) = match args {
        [out] => (*out, DEFAULT_TARGET),
        [out, "--target", target] => (*out, *target),
        _ => bail!("usage: cargo xtask notices <out file> [--target <triple>]"),
    };
    let root = crate::repo_root();
    let output = Command::new(env_cargo())
        .args(["metadata", "--format-version", "1", "--locked", "--filter-platform", target])
        .current_dir(&root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    let crates = shipped_crates(&metadata)?;

    let mut text = String::new();
    writeln!(text, "Nectarlink for Windows: third-party notices")?;
    writeln!(text, "{}", "=".repeat(43))?;
    writeln!(
        text,
        "\nNectarlink is free software under the GNU General Public License, version 3\n\
         or later (LICENSE.txt). It includes the software below, under these terms.\n"
    )?;
    write_qt(&mut text, &root)?;
    write_fonts(&mut text, &root)?;
    writeln!(text, "\nRust crates ({} in this build)\n{}", crates.len(), "-".repeat(30))?;
    for c in &crates {
        writeln!(text, "\n{} {}  ·  {}", c.name, c.version, c.license)?;
        if !c.repository.is_empty() {
            writeln!(text, "{}", c.repository)?;
        }
        for (file, body) in &c.files {
            writeln!(text, "\n--- {} ({file}) ---\n{}", c.name, body.trim_end())?;
        }
    }
    let out = PathBuf::from(out);
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({} crates)", out.display(), crates.len());
    Ok(())
}

fn env_cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// The crates in the app's normal dependency graph, workspace crates left
/// out (they're Nectarlink's own).
fn shipped_crates(metadata: &Value) -> Result<Vec<Crate>> {
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .context("no packages")?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let workspace: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .context("no members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .context("no resolve graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let start = packages
        .iter()
        .find(|(_, p)| p["name"] == ROOT_PACKAGE)
        .map(|(id, _)| *id)
        .context("the desktop app isn't in the workspace")?;

    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().into_iter().flatten() {
            // Only what's linked into the app: not build scripts or tests.
            let normal = dep["dep_kinds"].as_array().into_iter().flatten().any(|k| k["kind"].is_null());
            if let Some(dep_id) = dep["pkg"].as_str()
                && normal
                && seen.insert(dep_id)
            {
                queue.push_back(dep_id);
            }
        }
    }

    let mut crates: Vec<Crate> = seen
        .into_iter()
        .filter(|id| !workspace.contains(id))
        .filter_map(|id| packages.get(id))
        .map(|p| {
            let dir =
                Path::new(p["manifest_path"].as_str().unwrap_or_default()).parent().map(Path::to_path_buf);
            let mut files: Vec<(String, String)> = dir
                .and_then(|d| fs::read_dir(d).ok())
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_license_file(&e.file_name().to_string_lossy()))
                .filter_map(|e| {
                    Some((e.file_name().to_string_lossy().into_owned(), fs::read_to_string(e.path()).ok()?))
                })
                .collect();
            files.sort();
            Crate {
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                version: p["version"].as_str().unwrap_or_default().to_owned(),
                license: p["license"].as_str().unwrap_or("see its files").to_owned(),
                repository: p["repository"].as_str().unwrap_or_default().to_owned(),
                files,
            }
        })
        .collect();
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(crates)
}

fn write_qt(text: &mut String, root: &Path) -> Result<()> {
    writeln!(text, "Qt\n--")?;
    writeln!(
        text,
        "This app uses the Qt framework (https://www.qt.io), version 6, under the GNU\n\
         Lesser General Public License version 3 (below). Qt's libraries are the\n\
         separate Qt6*.dll files and plugins in this folder; you may replace them with\n\
         your own builds of the same version. Qt's source code is available from\n\
         https://download.qt.io/official_releases/qt/ and https://code.qt.io.\n"
    )?;
    let lgpl = fs::read_to_string(root.join("LICENSES/LGPL-3.0-only.txt")).context("reading the LGPL")?;
    writeln!(text, "{}\n", lgpl.trim_end())?;
    Ok(())
}

fn write_fonts(text: &mut String, root: &Path) -> Result<()> {
    writeln!(text, "\nFonts\n-----")?;
    writeln!(text, "Figtree, Instrument Serif and Space Mono, under the SIL Open Font License 1.1:")?;
    let mut ofl: Vec<PathBuf> = fs::read_dir(root.join("assets/fonts"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("OFL-")))
        .collect();
    ofl.sort();
    for path in ofl {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        writeln!(text, "\n--- {name} ---\n{}", fs::read_to_string(&path)?.trim_end())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_license_files() {
        for name in ["LICENSE", "LICENSE-MIT", "license-apache", "COPYING", "NOTICE.md", "UNLICENSE"] {
            assert!(is_license_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src"] {
            assert!(!is_license_file(name), "{name}");
        }
    }
}
