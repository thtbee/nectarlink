// SPDX-License-Identifier: GPL-3.0-or-later
//! The PC's screens, for a phone viewing it to pick one (#442): their places
//! on the desktop, the main one first, then left to right and top to bottom.
//! A screen's id is its place in that order, so it stays the same while the
//! screens do.

use windows::{
    Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
        UI::WindowsAndMessaging::MONITORINFOF_PRIMARY,
    },
    core::BOOL,
};

/// One screen: where it is on the desktop, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    pub id: u32,
    pub rect: RECT,
    pub primary: bool,
}

impl Display {
    pub fn width(&self) -> u32 {
        (self.rect.right - self.rect.left).max(1) as u32
    }

    pub fn height(&self) -> u32 {
        (self.rect.bottom - self.rect.top).max(1) as u32
    }
}

/// The PC's screens, in id order.
pub fn list() -> Vec<Display> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `Vec` passed below, alive for the whole enumeration.
        let found = unsafe { &mut *(data.0 as *mut Vec<(RECT, bool)>) };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        // SAFETY: `monitor` comes from Windows for this call; `info` is sized for it.
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            found.push((info.rcMonitor, info.dwFlags & MONITORINFOF_PRIMARY != 0));
        }
        true.into()
    }
    let mut found: Vec<(RECT, bool)> = Vec::new();
    // SAFETY: the callback only writes to `found`, which outlives the call.
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut _ as isize)) };
    order(found)
}

/// Main screen first, then by place: left to right, then top to bottom.
fn order(mut found: Vec<(RECT, bool)>) -> Vec<Display> {
    found.sort_by_key(|(rect, primary)| (!primary, rect.left, rect.top));
    found
        .into_iter()
        .enumerate()
        .map(|(i, (rect, primary))| Display { id: i as u32, rect, primary })
        .collect()
}

/// The screen `id`, while it's there.
pub fn by_id(id: u32) -> Option<Display> {
    list().into_iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, w: i32, h: i32) -> RECT {
        RECT { left, top, right: left + w, bottom: top + h }
    }

    #[test]
    fn the_main_screen_comes_first_then_left_to_right() {
        let found = vec![
            (rect(1920, 0, 1920, 1080), false),
            (rect(-1280, 0, 1280, 1024), false),
            (rect(0, 0, 1920, 1080), true),
        ];
        let displays = order(found);
        assert!(displays[0].primary);
        assert_eq!(displays[1].rect.left, -1280);
        assert_eq!(displays[2].rect.left, 1920);
        assert_eq!(displays.iter().map(|d| d.id).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!((displays[1].width(), displays[1].height()), (1280, 1024));
    }

    #[test]
    fn this_pc_has_a_main_screen() {
        let displays = list();
        // A CI runner may have none; a PC with screens has exactly one main one.
        if !displays.is_empty() {
            assert_eq!(displays.iter().filter(|d| d.primary).count(), 1);
            assert_eq!(displays[0].id, 0);
        }
    }
}
