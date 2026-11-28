// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS browser and window inspection stubs.

pub fn programs_for(player: &str) -> Vec<String> {
    vec![player.to_lowercase()]
}

pub fn is_browser(program: &str) -> bool {
    let lower = program.to_lowercase();
    lower.contains("safari")
        || lower.contains("chrome")
        || lower.contains("firefox")
        || lower.contains("edge")
        || lower.contains("brave")
}

#[derive(Debug, Clone)]
pub struct Window {
    #[allow(dead_code)]
    pub hwnd: isize,
    pub program: String,
    pub title: String,
}

pub fn windows() -> Vec<Window> {
    Vec::new()
}

pub fn active_browser_url_for_hwnd(_hwnd: isize) -> crate::links::ActiveBrowserUrl {
    crate::links::ActiveBrowserUrl::NoBrowser
}
