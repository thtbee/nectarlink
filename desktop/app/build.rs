// SPDX-License-Identifier: GPL-3.0-or-later
//! Builds the QML module `app.nectarlink` (QML files compiled ahead of time,
//! Rust QObjects, C++ helpers), compiles in the bundled fonts, gives the
//! executable its icon and version information, and links the Windows
//! libraries C++ uses.

use std::{
    fs,
    path::{Path, PathBuf},
};

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

// The mark the app draws at runtime, rendered here for the .exe's icon.
#[path = "src/mark.rs"]
mod mark;

const QML: &[&str] = &[
    "qml/App.qml",
    "qml/IslandWindow.qml",
    "qml/LaserOverlay.qml",
    "qml/MainWindow.qml",
    "qml/MirrorWindow.qml",
    "qml/ShelfWindow.qml",
    "qml/components/AppsSheet.qml",
    "qml/components/Avatar.qml",
    "qml/components/Button.qml",
    "qml/components/CallCard.qml",
    "qml/components/CaptionButtons.qml",
    "qml/components/Card.qml",
    "qml/components/Chip.qml",
    "qml/components/CodeDigits.qml",
    "qml/components/CommandPalette.qml",
    "qml/components/Divider.qml",
    "qml/components/GlassRim.qml",
    "qml/components/DoctorSheet.qml",
    "qml/components/HistoryItem.qml",
    "qml/components/Icon.qml",
    "qml/components/IconButton.qml",
    "qml/components/ListRow.qml",
    "qml/components/LockChip.qml",
    "qml/components/NavItem.qml",
    "qml/components/NotificationItem.qml",
    "qml/components/NowPlaying.qml",
    "qml/components/QrCode.qml",
    "qml/components/RoundedImage.qml",
    "qml/components/Segmented.qml",
    "qml/components/Sheet.qml",
    "qml/components/Spinner.qml",
    "qml/components/StatusDot.qml",
    "qml/components/Toast.qml",
    "qml/components/Toggle.qml",
    "qml/components/TransferItem.qml",
    "qml/components/Txt.qml",
    "qml/pages/AppsPage.qml",
    "qml/pages/CallsPage.qml",
    "qml/pages/DeckPage.qml",
    "qml/pages/FeaturesGuidePage.qml",
    "qml/pages/HomePage.qml",
    "qml/pages/MessagesPage.qml",
    "qml/pages/PairingPanel.qml",
    "qml/pages/PhotosPage.qml",
    "qml/pages/SettingsPage.qml",
    "qml/pages/TimelinePage.qml",
    "qml/pages/WalkthroughPage.qml",
    "qml/pages/WelcomePage.qml",
    "qml/components/HiveBackdrop.qml",
    "qml/setup/SadBee.qml",
    "qml/setup/SetupButton.qml",
    "qml/setup/SetupWindow.qml",
];

/// QML singletons.
const QML_SINGLETONS: &[&str] = &["qml/Tokens.qml", "qml/Theme.qml", "qml/Icons.qml"];

/// Bundled fonts (in `assets/fonts`), compiled in under `:/fonts/`.
const FONTS: &[&str] =
    &["Figtree.ttf", "InstrumentSerif-Regular.ttf", "SpaceMono-Regular.ttf", "SpaceMono-Bold.ttf"];

fn write_if_changed(path: &Path, content: impl AsRef<[u8]>) {
    let bytes = content.as_ref();
    if fs::read(path).ok().as_deref() != Some(bytes) {
        fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
}

/// Writes a resource file listing the fonts (they live outside this crate,
/// so each needs an alias) and returns its path.
fn fonts_qrc() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
    let mut qrc = String::from("<RCC>\n  <qresource prefix=\"/fonts\">\n");
    for font in FONTS {
        let path = dir.join(font).canonicalize().unwrap_or_else(|e| panic!("missing font {font}: {e}"));
        println!("cargo::rerun-if-changed={}", path.display());
        // rcc takes forward slashes; strip the verbatim prefix canonicalize adds.
        let path = path.display().to_string().trim_start_matches(r"\\?\").replace('\\', "/");
        qrc.push_str(&format!("    <file alias=\"{font}\">{path}</file>\n"));
    }
    qrc.push_str("  </qresource>\n</RCC>\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("fonts.qrc");
    write_if_changed(&out, qrc);
    out
}

/// Icon sizes in the .exe: what Explorer, the taskbar and Start ask for at
/// 100–200% scale.
const ICON_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 256];

/// An .ico file with the mark at every size, as 32-bit bitmaps (alpha used,
/// AND mask empty).
fn icon_file() -> Vec<u8> {
    let images: Vec<Vec<u8>> = ICON_SIZES
        .iter()
        .map(|&size| {
            let rgba = mark::render(size as usize);
            let mask_row = size.div_ceil(32) * 4;
            let mut image = Vec::new();
            // BITMAPINFOHEADER; the height counts the color and mask halves.
            for field in [40, size, size * 2] {
                image.extend_from_slice(&field.to_le_bytes());
            }
            image.extend_from_slice(&1u16.to_le_bytes());
            image.extend_from_slice(&32u16.to_le_bytes());
            image.extend_from_slice(&[0; 24]);
            // Bottom-up BGRA rows.
            for row in rgba.chunks_exact(size as usize * 4).rev() {
                for p in row.as_chunks::<4>().0 {
                    image.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            image.resize(image.len() + (mask_row * size) as usize, 0);
            image
        })
        .collect();
    let count = u16::try_from(images.len()).expect("a few sizes");
    let mut ico = Vec::new();
    for field in [0u16, 1, count] {
        ico.extend_from_slice(&field.to_le_bytes());
    }
    let mut offset = 6 + 16 * images.len();
    for (&size, image) in ICON_SIZES.iter().zip(&images) {
        // 256 is written as 0.
        let side = u8::try_from(size).unwrap_or(0);
        ico.extend_from_slice(&[side, side, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&u32::try_from(image.len()).expect("small").to_le_bytes());
        ico.extend_from_slice(&u32::try_from(offset).expect("small").to_le_bytes());
        offset += image.len();
    }
    for image in images {
        ico.extend_from_slice(&image);
    }
    ico
}

/// Fragment shaders, served under `qrc:/shaders/`.
const SHADERS: &[&str] = &["glassrim.frag", "hive.frag"];

/// Compiles the shaders with Qt's `qsb` when it can be found (next to qmake, or on PATH),
/// otherwise uses the compiled copies committed beside the sources. Returns the qrc path.
fn shaders_qrc() -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let qsb = std::env::var_os("QMAKE")
        .map(PathBuf::from)
        .and_then(|qmake| qmake.parent().map(|bin| bin.join("qsb")))
        .unwrap_or_else(|| PathBuf::from("qsb"));
    let mut qrc = String::from("<RCC>\n  <qresource prefix=\"/shaders\">\n");
    for shader in SHADERS {
        let source = src.join(shader);
        let committed = src.join(format!("{shader}.qsb"));
        println!("cargo::rerun-if-changed={}", source.display());
        println!("cargo::rerun-if-changed={}", committed.display());
        let built = out.join(format!("{shader}.qsb"));
        let compiled = std::process::Command::new(&qsb)
            .args(["--glsl", "100es,120,150", "--hlsl", "50", "--msl", "12", "-o"])
            .arg(&built)
            .arg(&source)
            .status()
            .is_ok_and(|s| s.success());
        let path = if compiled { built } else { committed };
        let path = path.display().to_string().replace('\\', "/");
        qrc.push_str(&format!("    <file alias=\"{shader}.qsb\">{path}</file>\n"));
    }
    qrc.push_str("  </qresource>\n</RCC>\n");
    let qrc_path = out.join("shaders.qrc");
    write_if_changed(&qrc_path, qrc);
    qrc_path
}

/// The .exe's icon and version information (what Explorer, shortcuts and
/// Task Manager show).
fn windows_resources() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("nectarlink.ico");
    write_if_changed(&ico, icon_file());
    let version = std::env::var("CARGO_PKG_VERSION").expect("version");
    let numbers: Vec<&str> = version.split(['.', '-', '+']).take(3).collect();
    let numeric = format!("{},0", numbers.join(","));
    // No #include: numeric constants keep rc.exe off the SDK headers.
    // 0x40004 is VOS_NT_WINDOWS32, 0x1 is VFT_APP.
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "The NectarLink contributors"
      VALUE "FileDescription", "NectarLink"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "nectarlink-desktop"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "nectarlink-desktop.exe"
      VALUE "ProductName", "NectarLink"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "/"),
    );
    let rc_path = out.join("nectarlink.rc");
    write_if_changed(&rc_path, rc);
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().expect("compile resources");
}

fn find_lrelease() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("LRELEASE") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }
    for candidate in [
        "/usr/lib/qt6/bin/lrelease",
        "/usr/lib64/qt6/bin/lrelease",
        "/usr/bin/lrelease-qt6",
        "/usr/bin/lrelease",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn elf_hash(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in bytes {
        h = h.wrapping_shl(4).wrapping_add(u32::from(b));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    if h == 0 { 1 } else { h }
}

fn unescape_xml(s: &str) -> String {
    s.replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn extract_tag<'a>(block: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start_tag = block.find(&open)?;
    let after_open = &block[start_tag + open.len()..];
    let gt = after_open.find('>')?;
    let content_start = &after_open[gt + 1..];
    let end = content_start.find(&close)?;
    Some(&content_start[..end])
}

fn compile_ts_to_qm_rust(ts_path: &Path, qm_path: &Path) -> bool {
    let Ok(xml) = fs::read_to_string(ts_path) else {
        return false;
    };
    let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut rest = xml.as_str();
    while let Some(ctx_pos) = rest.find("<context>") {
        let after_ctx = &rest[ctx_pos + "<context>".len()..];
        let Some(ctx_end) = after_ctx.find("</context>") else {
            break;
        };
        let ctx_block = &after_ctx[..ctx_end];
        let ctx_name = extract_tag(ctx_block, "name").map(unescape_xml).unwrap_or_default();
        let ctx_bytes = ctx_name.as_bytes();

        let mut msg_rest = ctx_block;
        while let Some(msg_pos) = msg_rest.find("<message") {
            let after_msg = &msg_rest[msg_pos..];
            let Some(msg_end) = after_msg.find("</message>") else {
                break;
            };
            let msg_block = &after_msg[..msg_end];
            if let (Some(raw_src), Some(raw_tr)) =
                (extract_tag(msg_block, "source"), extract_tag(msg_block, "translation"))
            {
                let src = unescape_xml(raw_src);
                let tr_body = extract_tag(raw_tr, "numerusform").unwrap_or(raw_tr);
                let tr = unescape_xml(tr_body);
                if !tr.is_empty() {
                    let src_bytes = src.as_bytes();
                    let h = elf_hash(src_bytes);
                    let mut tr_utf16be = Vec::with_capacity(tr.len() * 2);
                    for u in tr.encode_utf16() {
                        tr_utf16be.extend_from_slice(&u.to_be_bytes());
                    }
                    let mut rec = Vec::new();
                    // Tag_Translation = 3
                    rec.push(3u8);
                    rec.extend_from_slice(&(tr_utf16be.len() as u32).to_be_bytes());
                    rec.extend_from_slice(&tr_utf16be);
                    // Tag_Comment = 8 (empty)
                    rec.push(8u8);
                    rec.extend_from_slice(&0u32.to_be_bytes());
                    // Tag_SourceText = 6
                    rec.push(6u8);
                    rec.extend_from_slice(&(src_bytes.len() as u32).to_be_bytes());
                    rec.extend_from_slice(src_bytes);
                    // Tag_Context = 7
                    rec.push(7u8);
                    rec.extend_from_slice(&(ctx_bytes.len() as u32).to_be_bytes());
                    rec.extend_from_slice(ctx_bytes);
                    // Tag_End = 1
                    rec.push(1u8);
                    entries.push((h, rec));
                }
            }
            msg_rest = &after_msg[msg_end + "</message>".len()..];
        }
        rest = &after_ctx[ctx_end + "</context>".len()..];
    }

    entries.sort_by_key(|(h, _)| *h);
    let mut messages_buf = Vec::new();
    let mut hashes_buf = Vec::with_capacity(entries.len() * 8);
    for (h, rec) in entries {
        hashes_buf.extend_from_slice(&h.to_be_bytes());
        hashes_buf.extend_from_slice(&(messages_buf.len() as u32).to_be_bytes());
        messages_buf.extend_from_slice(&rec);
    }

    let mut qm = Vec::with_capacity(16 + 10 + hashes_buf.len() + messages_buf.len());
    qm.extend_from_slice(&[
        0x3c, 0xb8, 0x64, 0x18, 0xca, 0xef, 0x9c, 0x95, 0xcd, 0x21, 0x1c, 0xbf, 0x60, 0xa1, 0xbd, 0xdd,
    ]);
    // Tag_Hashes = 0x42
    qm.push(0x42);
    qm.extend_from_slice(&(hashes_buf.len() as u32).to_be_bytes());
    qm.extend_from_slice(&hashes_buf);
    // Tag_Messages = 0x69
    qm.push(0x69);
    qm.extend_from_slice(&(messages_buf.len() as u32).to_be_bytes());
    qm.extend_from_slice(&messages_buf);

    fs::write(qm_path, qm).is_ok()
}

fn compile_translations() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let i18n_dir = Path::new(&manifest_dir).join("i18n");
    println!("cargo:rerun-if-changed={}", i18n_dir.display());
    let Ok(entries) = fs::read_dir(&i18n_dir) else {
        return;
    };
    let ts_files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ts"))
        .collect();
    for ts in &ts_files {
        println!("cargo:rerun-if-changed={}", ts.display());
    }
    if ts_files.is_empty() {
        return;
    }
    let lrelease = find_lrelease();
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let out_path = PathBuf::from(out_dir);
    let profile_dir = out_path.ancestors().nth(3).map(Path::to_path_buf);

    for ts in ts_files {
        let Some(stem) = ts.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let qm_name = format!("{stem}.qm");
        let out_qm = out_path.join(&qm_name);
        let compiled = if let Some(ref bin) = lrelease {
            std::process::Command::new(bin)
                .arg("-silent")
                .arg(&ts)
                .arg("-qm")
                .arg(&out_qm)
                .status()
                .is_ok_and(|s| s.success())
        } else {
            compile_ts_to_qm_rust(&ts, &out_qm)
        };
        if compiled && let Some(ref target_dir) = profile_dir {
            let _ = fs::copy(&out_qm, target_dir.join(&qm_name));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/mark.rs");
    for qml in QML.iter().chain(QML_SINGLETONS.iter()) {
        println!("cargo:rerun-if-changed={qml}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
    compile_translations();
    let module = QmlModule::new("app.nectarlink")
        .qml_files(QML.iter().map(|f| QmlFile::from(*f)))
        .qml_files(QML_SINGLETONS.iter().map(|f| QmlFile::from(*f).singleton(true)));
    CxxQtBuilder::new_qml_module(module)
        .qt_module("Quick")
        .include_dir("cpp")
        .cpp_files([
            "cpp/app_helpers.cpp",
            "cpp/native_window.h",
            "cpp/native_window.cpp",
            "cpp/video_view.h",
            "cpp/video_view.cpp",
        ])
        .qrc(fonts_qrc())
        .qrc(shaders_qrc())
        .files([
            "src/bridge/native.rs",
            "src/bridge/app.rs",
            "src/bridge/calls.rs",
            "src/bridge/devices.rs",
            "src/bridge/history.rs",
            "src/bridge/media.rs",
            "src/bridge/messages.rs",
            "src/bridge/mirror.rs",
            "src/bridge/notifications.rs",
            "src/bridge/pairing.rs",
            "src/bridge/photos.rs",
            "src/bridge/prefs.rs",
            "src/bridge/setup.rs",
            "src/bridge/timeline.rs",
            "src/bridge/transfers.rs",
            "src/bridge/webcam.rs",
        ])
        .build();

    // The C++ window code calls DWM, Shell property store, and user32 directly.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dwmapi");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=user32");
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR")
    {
        println!("cargo:rustc-link-arg=-fuse-ld=bfd");
        println!("cargo:rustc-link-lib=Qt6Quick");
        let sysroot_lib =
            std::path::Path::new(&manifest_dir).join("../../.qt6-sysroot/usr/lib/x86_64-linux-gnu");
        if let Ok(canon) = sysroot_lib.canonicalize() {
            println!("cargo:rustc-link-search=native={}", canon.display());
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", canon.display());
        }
    }
    println!("cargo:rerun-if-changed=cpp/app_helpers.h");
    println!("cargo:rerun-if-changed=cpp/app_helpers.cpp");
    println!("cargo:rerun-if-changed=cpp/native_window.h");
    println!("cargo:rerun-if-changed=cpp/native_window.cpp");
    println!("cargo:rerun-if-changed=cpp/video_view.h");
    println!("cargo:rerun-if-changed=cpp/video_view.cpp");
}
