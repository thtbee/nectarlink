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
    "qml/components/ValueSlider.qml",
    "qml/components/SwitchBar.qml",
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
    if let Some(qmake) = std::env::var_os("QMAKE").map(PathBuf::from)
        && let Some(bin_dir) = qmake.parent()
    {
        for name in ["lrelease", "lrelease.exe", "lrelease-qt6"] {
            let candidate = bin_dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
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

fn offered_languages(manifest_dir: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let i18n_rs = manifest_dir.join("src").join("i18n.rs");
    println!("cargo:rerun-if-changed={}", i18n_rs.display());
    let Ok(src) = fs::read_to_string(&i18n_rs) else {
        return set;
    };
    let Some((_, rest)) = src.split_once("pub const LANGUAGES") else {
        return set;
    };
    let Some((table, _)) = rest.split_once("];") else {
        return set;
    };
    for part in table.split('(').skip(1) {
        if let Some(after_quote) = part.strip_prefix('"')
            && let Some((code, _)) = after_quote.split_once('"')
            && !code.is_empty()
        {
            set.insert(code.to_string());
        }
    }
    set
}

fn compile_translations() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let manifest_path = Path::new(&manifest_dir);
    let offered = offered_languages(manifest_path);
    let i18n_dir = manifest_path.join("i18n");
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
    let Some(lrelease) = find_lrelease() else {
        println!(
            "cargo:warning=lrelease not found; skipping .qm compilation (install qt6-l10n-tools or set LRELEASE)"
        );
        return;
    };
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let out_path = PathBuf::from(out_dir);
    let profile_dir = out_path.ancestors().nth(3).map(Path::to_path_buf);

    for ts in ts_files {
        let Some(stem) = ts.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(lang) = stem.strip_prefix("nectarlink_") else {
            continue;
        };
        if !offered.contains(lang) {
            continue;
        }
        let qm_name = format!("{stem}.qm");
        let out_qm = out_path.join(&qm_name);
        let compiled = std::process::Command::new(&lrelease)
            .arg("-silent")
            .arg(&ts)
            .arg("-qm")
            .arg(&out_qm)
            .status()
            .is_ok_and(|s| s.success());
        if compiled && let Some(ref target_dir) = profile_dir {
            let _ = fs::copy(&out_qm, target_dir.join(&qm_name));
        }
    }
}

/// Opt-in native producer/activation bridge; the separate extension executable
/// is assembled only by scripts/macos/build-native-camera-extension.py.
fn native_camera_support() {
    use std::process::Command;
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported native camera architecture"),
    };
    let mut objects = Vec::new();
    for stem in ["NectarLinkCameraProducer", "NectarLinkCameraActivation"] {
        let source = format!("src/mac/camera_extension/{stem}.m");
        println!("cargo:rerun-if-changed={source}");
        let object = out.join(format!("{stem}.o"));
        assert!(
            Command::new("xcrun")
                .args([
                    "--sdk",
                    "macosx",
                    "clang",
                    "-c",
                    "-fobjc-arc",
                    "-O2",
                    "-arch",
                    arch,
                    "-mmacosx-version-min=12.3",
                    &source,
                    "-o"
                ])
                .arg(&object)
                .status()
                .expect("compile native camera bridge")
                .success()
        );
        objects.push(object);
    }
    for header in ["NectarLinkCamera.h", "NectarLinkCameraActivation.h"] {
        println!("cargo:rerun-if-changed=src/mac/camera_extension/{header}");
    }
    let library = out.join("libnectarlink_camera.a");
    assert!(
        Command::new("xcrun")
            .args(["libtool", "-static", "-o"])
            .arg(&library)
            .args(objects)
            .status()
            .expect("archive native camera bridge")
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=nectarlink_camera");
    for framework in
        ["Foundation", "CoreMediaIO", "CoreMedia", "CoreVideo", "Accelerate", "SystemExtensions", "Security"]
    {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_MACOS_NATIVE_CAMERA").is_some()
    {
        native_camera_support();
    }
    println!("cargo:rerun-if-changed=src/mark.rs");
    for qml in QML.iter().chain(QML_SINGLETONS.iter()) {
        println!("cargo:rerun-if-changed={qml}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        compile_translations();
    }
    let module = QmlModule::new("app.nectarlink")
        .qml_files(QML.iter().map(|f| QmlFile::from(*f)))
        .qml_files(QML_SINGLETONS.iter().map(|f| QmlFile::from(*f).singleton(true)));
    let mut builder = CxxQtBuilder::new_qml_module(module).qt_module("Quick");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        builder = builder.qt_module("Widgets");
    }
    builder
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
            "src/bridge/phone_mic.rs",
            "src/bridge/photo_backup.rs",
            "src/bridge/photos.rs",
            "src/bridge/prefs.rs",
            "src/bridge/setup.rs",
            "src/bridge/timeline.rs",
            "src/bridge/transfers.rs",
            "src/bridge/webcam.rs",
        ])
        .build();

    // The C++ window code calls DWM, Shell property store, and user32 directly on Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dwmapi");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=user32");
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=Qt6Quick");
        println!("cargo:rustc-link-lib=Qt6Widgets");
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN:$ORIGIN/lib:$ORIGIN/../lib:$ORIGIN/../lib/nectarlink/lib"
        );
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
    println!("cargo:rerun-if-changed=cpp/app_helpers.h");
    println!("cargo:rerun-if-changed=cpp/app_helpers.cpp");
    println!("cargo:rerun-if-changed=cpp/native_window.h");
    println!("cargo:rerun-if-changed=cpp/native_window.cpp");
    println!("cargo:rerun-if-changed=cpp/video_view.h");
    println!("cargo:rerun-if-changed=cpp/video_view.cpp");
}
