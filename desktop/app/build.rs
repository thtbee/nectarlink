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
    "qml/LaserOverlay.qml",
    "qml/MainWindow.qml",
    "qml/MirrorWindow.qml",
    "qml/components/AppsSheet.qml",
    "qml/components/Avatar.qml",
    "qml/components/Button.qml",
    "qml/components/CallCard.qml",
    "qml/components/CaptionButtons.qml",
    "qml/components/Card.qml",
    "qml/components/Chip.qml",
    "qml/components/CodeDigits.qml",
    "qml/components/Divider.qml",
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
    "qml/pages/CallsPage.qml",
    "qml/pages/DeckPage.qml",
    "qml/pages/HomePage.qml",
    "qml/pages/MessagesPage.qml",
    "qml/pages/PairingPanel.qml",
    "qml/pages/PhotosPage.qml",
    "qml/pages/SettingsPage.qml",
    "qml/pages/WelcomePage.qml",
];

/// QML singletons.
const QML_SINGLETONS: &[&str] = &["qml/Tokens.qml", "qml/Theme.qml", "qml/Icons.qml"];

/// Bundled fonts (in `assets/fonts`), compiled in under `:/fonts/`.
const FONTS: &[&str] =
    &["Figtree.ttf", "InstrumentSerif-Regular.ttf", "SpaceMono-Regular.ttf", "SpaceMono-Bold.ttf"];

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
    fs::write(&out, qrc).expect("write fonts.qrc");
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

/// The .exe's icon and version information (what Explorer, shortcuts and
/// Task Manager show).
fn windows_resources() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("nectarlink.ico");
    fs::write(&ico, icon_file()).expect("write the icon");
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
      VALUE "CompanyName", "The Nectarlink contributors"
      VALUE "FileDescription", "Nectarlink"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "nectarlink-desktop"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "nectarlink-desktop.exe"
      VALUE "ProductName", "Nectarlink"
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
    fs::write(&rc_path, rc).expect("write the resource script");
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().expect("compile resources");
}

fn main() {
    println!("cargo:rerun-if-changed=src/mark.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
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
            "src/bridge/transfers.rs",
        ])
        .build();

    // The C++ window code calls DWM, Shell property store, and user32 directly.
    println!("cargo:rustc-link-lib=dwmapi");
    println!("cargo:rustc-link-lib=ole32");
    println!("cargo:rustc-link-lib=shell32");
    println!("cargo:rustc-link-lib=user32");
}
