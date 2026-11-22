// SPDX-License-Identifier: GPL-3.0-or-later
//! The user's keyboard layout, for typing through `/dev/uinput`.
//!
//! uinput sends physical key codes and the desktop turns them into
//! characters with the user's layout, so typing "z" means pressing the key
//! that is z on that layout (where Y is on a US keyboard, for German). This
//! compiles the configured layout with libxkbcommon, which every Qt app
//! already loads, and finds for each character the key and the modifiers
//! (Shift, AltGr) that produce it.

use std::{
    collections::HashMap,
    ffi::{CString, c_char, c_int, c_void},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// What the virtual keyboard presses for one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// Linux evdev `KEY_*` code.
    pub code: u16,
    pub shift: bool,
    pub altgr: bool,
}

/// An XKB layout as configured by the desktop: model, layout, variant and options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
}

/// Characters of one layout and the keys that type them.
#[derive(Debug, Default)]
pub struct Keymap {
    by_char: HashMap<char, KeyPress>,
}

impl Keymap {
    /// The key that types `ch`, if this layout has one.
    pub fn press_for(&self, ch: char) -> Option<KeyPress> {
        match ch {
            '\n' => Some(KeyPress { code: KEY_ENTER, shift: false, altgr: false }),
            '\t' => Some(KeyPress { code: KEY_TAB, shift: false, altgr: false }),
            _ => self.by_char.get(&ch).copied(),
        }
    }
}

/// The keymap for the desktop's current layout, compiled once. `None` when
/// the layout can't be found or compiled: callers then must not type through
/// uinput, since guessing a layout types the wrong characters.
pub fn current() -> Option<Arc<Keymap>> {
    // Niri says which of its layouts is active, and that changes as the user
    // switches: ask it (at most once a second) and compile each layout once.
    // If it can't say, typing would guess, so don't (D511).
    if std::env::var_os("NIRI_SOCKET").is_some() {
        return niri_layout().and_then(|layout| compiled(&layout));
    }
    static KEYMAP: OnceLock<Option<Arc<Keymap>>> = OnceLock::new();
    KEYMAP
        .get_or_init(|| {
            let layout = detect_layout()?;
            let keymap = compile(&layout);
            if keymap.is_none() {
                tracing::warn!(layout = %layout.layout, "can't compile the keyboard layout");
            }
            keymap.map(Arc::new)
        })
        .clone()
}

/// `layout`'s keymap, compiled the first time it's asked for.
fn compiled(layout: &Layout) -> Option<Arc<Keymap>> {
    static CACHE: Mutex<Vec<(Layout, Option<Arc<Keymap>>)>> = Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, keymap)) = cache.iter().find(|(l, _)| l == layout) {
        return keymap.clone();
    }
    let keymap = compile(layout).map(Arc::new);
    if keymap.is_none() {
        tracing::warn!(layout = %layout.layout, "can't compile the keyboard layout");
    }
    cache.push((layout.clone(), keymap.clone()));
    keymap
}

/// Niri's active layout, as an XKB layout and variant.
fn niri_layout() -> Option<Layout> {
    static LAST: Mutex<Option<(Instant, Option<Layout>)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, layout)) = last.as_ref()
        && at.elapsed() < Duration::from_secs(1)
    {
        return layout.clone();
    }
    let layout = std::process::Command::new("niri")
        .args(["msg", "-j", "keyboard-layouts"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_niri_layouts(&String::from_utf8_lossy(&o.stdout)))
        .and_then(|name| {
            let rules = xkb_rules_xml()?;
            let found = layout_for_description(&rules, &name);
            if found.is_none() {
                tracing::warn!(layout = %name, "Niri's keyboard layout isn't in the XKB rules");
            }
            found
        });
    *last = Some((Instant::now(), layout.clone()));
    layout
}

/// The active layout's name from `niri msg -j keyboard-layouts`
/// (`{"names": ["English (US)", "Russian"], "current_idx": 1}`).
pub(crate) fn parse_niri_layouts(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let index = usize::try_from(value.get("current_idx")?.as_u64()?).ok()?;
    Some(value.get("names")?.as_array()?.get(index)?.as_str()?.to_owned())
}

/// The XKB rules file that lists every layout by its description.
fn xkb_rules_xml() -> Option<String> {
    let roots = std::env::var("XKB_CONFIG_ROOT").ok().into_iter().chain(
        ["/usr/share/X11/xkb", "/run/current-system/sw/share/X11/xkb", "/usr/share/xkeyboard-config-2"]
            .map(String::from),
    );
    roots
        .flat_map(|root| ["evdev.xml", "base.xml"].map(|f| std::path::Path::new(&root).join("rules").join(f)))
        .find_map(|path| std::fs::read_to_string(path).ok())
}

/// The layout and variant whose description is `description` in an XKB
/// rules file. `None` when none or more than one match: never guess.
pub(crate) fn layout_for_description(rules: &str, description: &str) -> Option<Layout> {
    let tag = |text: &str, name: &str| -> Option<String> {
        let start = text.find(&format!("<{name}>"))? + name.len() + 2;
        let end = start + text[start..].find(&format!("</{name}>"))?;
        Some(unescape_xml(text[start..end].trim()))
    };
    let mut found = Vec::new();
    for block in rules.split("<layout>").skip(1) {
        let block = block.split("</layout>").next().unwrap_or_default();
        let (base, variants) = block.split_once("<variantList>").unwrap_or((block, ""));
        let Some(layout) = tag(base, "name") else { continue };
        if tag(base, "description").as_deref() == Some(description) {
            found.push(Layout { layout: layout.clone(), ..Layout::default() });
        }
        for variant in variants.split("<variant>").skip(1) {
            if tag(variant, "description").as_deref() == Some(description)
                && let Some(name) = tag(variant, "name")
            {
                found.push(Layout { layout: layout.clone(), variant: name, ..Layout::default() });
            }
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

const KEY_TAB: u16 = 15;
const KEY_ENTER: u16 = 28;
/// The right Alt key, which is AltGr (ISO_Level3_Shift) on layouts that have a third level.
pub const KEY_RIGHTALT: u16 = 100;
/// XKB key codes are evdev codes plus 8.
const XKB_EVDEV_OFFSET: u32 = 8;

/// Keypad keys produce digits and operators too, but depend on Num Lock; the
/// main keys are the ones to press.
fn is_keypad(code: u16) -> bool {
    matches!(code, 55 | 71..=83 | 96 | 98 | 117 | 118 | 121)
}

// ---- Finding the layout ----

/// The layout the desktop types with, from (in order) the `XKB_DEFAULT_*`
/// variables, GNOME's input sources, KDE's keyboard settings, and the
/// system's keyboard configuration.
pub fn detect_layout() -> Option<Layout> {
    if let Some(layout) = from_env() {
        return Some(layout);
    }
    if let Some(layout) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.input-sources", "sources"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_gnome_sources(&String::from_utf8_lossy(&o.stdout)))
    {
        return Some(layout);
    }
    if let Some(layout) = dirs::config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("kxkbrc")).ok())
        .and_then(|text| parse_kxkbrc(&text))
    {
        return Some(layout);
    }
    if let Some(layout) =
        std::fs::read_to_string("/etc/default/keyboard").ok().and_then(|text| parse_default_keyboard(&text))
    {
        return Some(layout);
    }
    std::fs::read_to_string("/etc/X11/xorg.conf.d/00-keyboard.conf")
        .ok()
        .and_then(|text| parse_xorg_keyboard(&text))
}

fn from_env() -> Option<Layout> {
    let var = |name: &str| std::env::var(name).unwrap_or_default().trim().to_owned();
    let layout = first_of(&var("XKB_DEFAULT_LAYOUT"));
    (!layout.is_empty()).then(|| Layout {
        model: var("XKB_DEFAULT_MODEL"),
        layout,
        variant: first_of(&var("XKB_DEFAULT_VARIANT")),
        options: var("XKB_DEFAULT_OPTIONS"),
    })
}

/// The first entry of a comma-separated XKB list ("de,us" -> "de").
fn first_of(list: &str) -> String {
    list.split(',').next().unwrap_or_default().trim().to_owned()
}

/// GNOME: `[('xkb', 'de+nodeadkeys'), ('ibus', 'mozc-jp')]`, first XKB source.
/// An empty list (`@a(ss) []`) means GNOME follows the system layout.
pub(crate) fn parse_gnome_sources(text: &str) -> Option<Layout> {
    let mut rest = text;
    while let Some(start) = rest.find("('") {
        rest = &rest[start + 2..];
        let (kind, after) = rest.split_once("', '")?;
        let (id, after) = after.split_once("')")?;
        rest = after;
        if kind == "xkb" && !id.is_empty() {
            let (layout, variant) = id.split_once('+').unwrap_or((id, ""));
            return Some(Layout { layout: layout.into(), variant: variant.into(), ..Layout::default() });
        }
    }
    None
}

/// KDE: `~/.config/kxkbrc`, used only when `Use=true` (otherwise KDE keeps the system layout).
pub(crate) fn parse_kxkbrc(text: &str) -> Option<Layout> {
    let mut in_layout = false;
    let mut values = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_layout = line == "[Layout]";
        } else if in_layout && let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    if values.get("Use").map(String::as_str) != Some("true") {
        return None;
    }
    let layout = first_of(values.get("LayoutList")?);
    (!layout.is_empty()).then(|| Layout {
        model: values.get("Model").cloned().unwrap_or_default(),
        layout,
        variant: first_of(values.get("VariantList").map(String::as_str).unwrap_or_default()),
        options: values.get("Options").cloned().unwrap_or_default(),
    })
}

/// Debian and Ubuntu: `/etc/default/keyboard` (`XKBLAYOUT="de"`, ...).
pub(crate) fn parse_default_keyboard(text: &str) -> Option<Layout> {
    let mut values = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.trim().split_once('=') {
            values.insert(key.trim(), value.trim().trim_matches('"').to_owned());
        }
    }
    let layout = first_of(values.get("XKBLAYOUT")?);
    (!layout.is_empty()).then(|| Layout {
        model: values.get("XKBMODEL").cloned().unwrap_or_default(),
        layout,
        variant: first_of(values.get("XKBVARIANT").map(String::as_str).unwrap_or_default()),
        options: values.get("XKBOPTIONS").cloned().unwrap_or_default(),
    })
}

/// Fedora, Arch and others: `/etc/X11/xorg.conf.d/00-keyboard.conf`
/// (`Option "XkbLayout" "de"`), as written by `localectl`.
pub(crate) fn parse_xorg_keyboard(text: &str) -> Option<Layout> {
    let mut layout = Layout::default();
    for line in text.lines() {
        let mut parts = line.split('"').map(str::trim).filter(|p| !p.is_empty());
        if parts.next() != Some("Option") {
            continue;
        }
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else { continue };
        match key {
            "XkbModel" => layout.model = value.into(),
            "XkbLayout" => layout.layout = first_of(value),
            "XkbVariant" => layout.variant = first_of(value),
            "XkbOptions" => layout.options = value.into(),
            _ => {}
        }
    }
    (!layout.layout.is_empty()).then_some(layout)
}

// ---- Compiling it with libxkbcommon ----

#[repr(C)]
struct RuleNames {
    rules: *const c_char,
    model: *const c_char,
    layout: *const c_char,
    variant: *const c_char,
    options: *const c_char,
}

type ContextNew = unsafe extern "C" fn(c_int) -> *mut c_void;
type ContextUnref = unsafe extern "C" fn(*mut c_void);
type KeymapNewFromNames = unsafe extern "C" fn(*mut c_void, *const RuleNames, c_int) -> *mut c_void;
type KeymapUnref = unsafe extern "C" fn(*mut c_void);
type KeymapKeycode = unsafe extern "C" fn(*mut c_void) -> u32;
type KeymapNumLevels = unsafe extern "C" fn(*mut c_void, u32, u32) -> u32;
type KeymapSymsByLevel = unsafe extern "C" fn(*mut c_void, u32, u32, u32, *mut *const u32) -> c_int;
type KeysymToUtf32 = unsafe extern "C" fn(u32) -> u32;

/// Compiles `layout` and maps every character it can type to a key.
pub fn compile(layout: &Layout) -> Option<Keymap> {
    // SAFETY: libxkbcommon is loaded and its functions resolved by name with
    // the signatures from xkbcommon.h; every object created here is released
    // before returning, and `syms` points into the keymap while it is alive.
    unsafe {
        let lib = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if lib.is_null() {
            return None;
        }
        let sym = |name: &std::ffi::CStr| {
            let p = libc::dlsym(lib, name.as_ptr());
            (!p.is_null()).then_some(p)
        };
        let context_new: ContextNew = std::mem::transmute(sym(c"xkb_context_new")?);
        let context_unref: ContextUnref = std::mem::transmute(sym(c"xkb_context_unref")?);
        let keymap_new: KeymapNewFromNames = std::mem::transmute(sym(c"xkb_keymap_new_from_names")?);
        let keymap_unref: KeymapUnref = std::mem::transmute(sym(c"xkb_keymap_unref")?);
        let min_keycode: KeymapKeycode = std::mem::transmute(sym(c"xkb_keymap_min_keycode")?);
        let max_keycode: KeymapKeycode = std::mem::transmute(sym(c"xkb_keymap_max_keycode")?);
        let num_levels: KeymapNumLevels = std::mem::transmute(sym(c"xkb_keymap_num_levels_for_key")?);
        let syms_by_level: KeymapSymsByLevel = std::mem::transmute(sym(c"xkb_keymap_key_get_syms_by_level")?);
        let to_utf32: KeysymToUtf32 = std::mem::transmute(sym(c"xkb_keysym_to_utf32")?);

        let c = |s: &str| CString::new(s).ok();
        let (model, name, variant, options) =
            (c(&layout.model)?, c(&layout.layout)?, c(&layout.variant)?, c(&layout.options)?);
        let opt = |s: &CString| if s.as_bytes().is_empty() { std::ptr::null() } else { s.as_ptr() };
        let names = RuleNames {
            rules: std::ptr::null(),
            model: opt(&model),
            layout: name.as_ptr(),
            variant: opt(&variant),
            options: opt(&options),
        };

        let ctx = context_new(0);
        if ctx.is_null() {
            return None;
        }
        let km = keymap_new(ctx, &names, 0);
        if km.is_null() {
            context_unref(ctx);
            return None;
        }

        let mut by_char = HashMap::new();
        for keycode in min_keycode(km)..=max_keycode(km) {
            let Some(code) = keycode.checked_sub(XKB_EVDEV_OFFSET).and_then(|c| u16::try_from(c).ok()) else {
                continue;
            };
            if is_keypad(code) {
                continue;
            }
            // Levels 0 to 3 of the standard key types: none, Shift, AltGr, Shift+AltGr.
            for level in 0..num_levels(km, keycode, 0).min(4) {
                let mut syms: *const u32 = std::ptr::null();
                let count = syms_by_level(km, keycode, 0, level, &mut syms);
                if count <= 0 || syms.is_null() {
                    continue;
                }
                for &keysym in std::slice::from_raw_parts(syms, count as usize) {
                    let Some(ch) = char::from_u32(to_utf32(keysym)).filter(|ch| !ch.is_control()) else {
                        continue;
                    };
                    // The lowest level wins: plain before Shift before AltGr.
                    by_char.entry(ch).or_insert(KeyPress { code, shift: level % 2 == 1, altgr: level >= 2 });
                }
            }
        }
        keymap_unref(km);
        context_unref(ctx);
        Some(Keymap { by_char })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_active_niri_layout() {
        let json = r#"{"names":["English (US)","Russian","German (no dead keys)"],"current_idx":2}"#;
        assert_eq!(parse_niri_layouts(json).as_deref(), Some("German (no dead keys)"));
        assert_eq!(parse_niri_layouts(r#"{"names":["English (US)"],"current_idx":3}"#), None);
        assert_eq!(parse_niri_layouts("not json"), None);

        let rules = r#"<xkbConfigRegistry><layoutList>
          <layout><configItem><name>us</name><shortDescription>en</shortDescription><description>English (US)</description></configItem>
            <variantList><variant><configItem><name>intl</name><description>English (US, intl., with dead keys)</description></configItem></variant></variantList></layout>
          <layout><configItem><name>de</name><description>German</description></configItem>
            <variantList><variant><configItem><name>nodeadkeys</name><description>German (no dead keys)</description></configItem></variant></variantList></layout>
          <layout><configItem><name>ru</name><description>Russian</description></configItem></layout>
          <layout><configItem><name>gb</name><description>English (UK) &amp; more</description></configItem></layout>
        </layoutList></xkbConfigRegistry>"#;
        let found = |d: &str| layout_for_description(rules, d).map(|l| (l.layout, l.variant));
        assert_eq!(found("English (US)"), Some(("us".into(), String::new())));
        assert_eq!(found("German (no dead keys)"), Some(("de".into(), "nodeadkeys".into())));
        assert_eq!(found("Russian"), Some(("ru".into(), String::new())));
        assert_eq!(found("English (UK) & more"), Some(("gb".into(), String::new())));
        assert_eq!(found("Klingon"), None);
    }

    #[test]
    fn reads_the_layout_from_each_desktop() {
        let gnome = parse_gnome_sources("[('ibus', 'mozc-jp'), ('xkb', 'de+nodeadkeys'), ('xkb', 'us')]\n");
        assert_eq!(gnome.map(|l| (l.layout, l.variant)), Some(("de".into(), "nodeadkeys".into())));
        assert_eq!(parse_gnome_sources("@a(ss) []\n"), None);

        let kde = "[Layout]\nDisplayNames=,\nLayoutList=fr,us\nModel=pc105\nUse=true\nVariantList=azerty,\n";
        let kde = parse_kxkbrc(kde).unwrap();
        assert_eq!(
            (kde.layout.as_str(), kde.variant.as_str(), kde.model.as_str()),
            ("fr", "azerty", "pc105")
        );
        assert_eq!(
            parse_kxkbrc("[Layout]\nLayoutList=fr\nUse=false\n"),
            None,
            "Use=false keeps the system layout"
        );

        let debian = "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"de,us\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\n";
        assert_eq!(parse_default_keyboard(debian).map(|l| l.layout), Some("de".into()));

        let xorg = "Section \"InputClass\"\n        Identifier \"system-keyboard\"\n        Option \"XkbLayout\" \"ch\"\n        Option \"XkbVariant\" \"fr\"\nEndSection\n";
        let xorg = parse_xorg_keyboard(xorg).unwrap();
        assert_eq!((xorg.layout.as_str(), xorg.variant.as_str()), ("ch", "fr"));
    }

    fn layout(name: &str, variant: &str) -> Keymap {
        compile(&Layout { layout: name.into(), variant: variant.into(), ..Layout::default() })
            .expect("libxkbcommon and xkeyboard-config are installed wherever Qt is")
    }

    #[test]
    fn finds_the_key_for_each_character_on_the_layout() {
        let us = layout("us", "");
        assert_eq!(us.press_for('z'), Some(KeyPress { code: 44, shift: false, altgr: false }));
        assert_eq!(us.press_for('Z'), Some(KeyPress { code: 44, shift: true, altgr: false }));
        assert_eq!(us.press_for('@'), Some(KeyPress { code: 3, shift: true, altgr: false }));

        // German: Y and Z swap places, @ is AltGr+Q, umlauts have their own keys.
        let de = layout("de", "");
        assert_eq!(de.press_for('z'), Some(KeyPress { code: 21, shift: false, altgr: false }));
        assert_eq!(de.press_for('y'), Some(KeyPress { code: 44, shift: false, altgr: false }));
        assert_eq!(de.press_for('@'), Some(KeyPress { code: 16, shift: false, altgr: true }));
        assert_eq!(de.press_for('ö'), Some(KeyPress { code: 39, shift: false, altgr: false }));

        // French: A and Q swap, digits need Shift.
        let fr = layout("fr", "");
        assert_eq!(fr.press_for('a'), Some(KeyPress { code: 16, shift: false, altgr: false }));
        assert_eq!(fr.press_for('1'), Some(KeyPress { code: 2, shift: true, altgr: false }));

        assert_eq!(us.press_for('\n').map(|p| p.code), Some(KEY_ENTER));
        assert_eq!(us.press_for('ö'), None, "a character the layout can't type");
    }
}
