use std::io::Result;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
    Shelf,
    TakePhoto,
    ScanDocument,
    FindPhone,
    OpenLinkOnPhone,
    Quit,
    Resumed,
    AppearanceChanged,
}

#[derive(Debug, Clone)]
pub struct MenuLabels {
    pub open: String,
    pub command_palette: String,
    pub shelf: String,
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub quit: String,
}

const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;

pub fn record_foreground_window() {}

pub fn snapshot_target_window() -> Option<(isize, String)> {
    None
}

pub fn is_window_alive_and_visible(_hwnd: isize) -> bool {
    false
}

pub fn focus_window(_hwnd: isize) -> bool {
    false
}

pub fn send_paste_fallback_if_not_foreground(_hwnd: isize) {}

pub fn apply_no_activate_style_by_title(_title: &str) {}

pub fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut mods = MOD_NOREPEAT;
    let mut vk: Option<u32> = None;

    for raw in trimmed.split('+') {
        let token = raw.trim().to_ascii_uppercase();
        if token.is_empty() {
            return None;
        }
        match token.as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" => mods |= MOD_WIN,
            "SPACE" => vk = Some(0x20),
            "TAB" => vk = Some(0x09),
            "ENTER" | "RETURN" => vk = Some(0x0D),
            "ESC" | "ESCAPE" => vk = Some(0x1B),
            s if s.len() == 1 => {
                let ch = s.as_bytes()[0];
                if ch.is_ascii_uppercase() || ch.is_ascii_digit() {
                    vk = Some(u32::from(ch));
                } else {
                    return None;
                }
            }
            s if s.starts_with('F') => {
                let n: u32 = s[1..].parse().ok()?;
                if (1..=24).contains(&n) {
                    vk = Some(0x70 + (n - 1));
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    let key = vk?;
    if mods == MOD_NOREPEAT {
        return None;
    }
    Some((mods, key))
}

pub fn update_hotkeys(
    _photo_spec: &str,
    _scan_spec: &str,
    _palette_spec: &str,
    _shelf_spec: &str,
) {
}

#[derive(Debug)]
pub struct Tray {
    tooltip: Mutex<String>,
    find_phone_enabled: Mutex<bool>,
}

impl Tray {
    pub fn create(
        tooltip: &str,
        _labels: MenuLabels,
        _handler: impl Fn(TrayEvent) + 'static,
    ) -> Result<Tray> {
        Ok(Tray {
            tooltip: Mutex::new(tooltip.to_string()),
            find_phone_enabled: Mutex::new(false),
        })
    }

    pub fn set_tooltip(&self, text: &str) {
        if let Ok(mut guard) = self.tooltip.lock() {
            *guard = text.to_string();
        }
    }

    pub fn set_find_phone_enabled(&self, enabled: bool) {
        if let Ok(mut guard) = self.find_phone_enabled.lock() {
            *guard = enabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkey_accepts_modifier_combinations() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+C").expect("valid hotkey");
        assert_ne!(mods & MOD_CONTROL, 0);
        assert_ne!(mods & MOD_ALT, 0);
        assert_eq!(vk, u32::from(b'C'));

        let (mods2, vk2) = parse_hotkey("Ctrl+Shift+F12").expect("function key");
        assert_ne!(mods2 & MOD_CONTROL, 0);
        assert_ne!(mods2 & MOD_SHIFT, 0);
        assert_eq!(vk2, 0x70 + 11);
    }

    #[test]
    fn parse_hotkey_rejects_missing_modifier_or_key() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("C").is_none());
        assert!(parse_hotkey("Ctrl+Alt").is_none());
        assert!(parse_hotkey("Ctrl+UnknownKey").is_none());
    }
}
