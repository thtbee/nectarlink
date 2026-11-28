// SPDX-License-Identifier: GPL-3.0-or-later
//! The window a player shows in, for Play on phone: which top-level window
//! belongs to a media session, the address of the page a browser plays in,
//! and where on screen the video is, read through UI Automation.
//!
//! Chromium browsers (Chrome, Edge, Brave, Opera, Vivaldi) expose their
//! address bar as an edit field of class `OmniboxViewViews` and a page's
//! elements with their HTML classes, so YouTube's player
//! (`video-stream html5-main-video`) can be found exactly; Firefox's
//! address bar has the automation ID `urlbar-input`, and Firefox-based
//! browsers (Zen, Floorp...) show only their tab's area, not the page. Nothing here reads a
//! page's text: only the address of the tab that plays.

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
            Variant::VARIANT,
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
                UIA_AutomationIdPropertyId, UIA_ClassNamePropertyId, UIA_ControlTypePropertyId,
                UIA_DocumentControlTypeId, UIA_PROPERTY_ID, UIA_ValueValuePropertyId,
            },
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowTextW, GetWindowThreadProcessId, HWND_BOTTOM,
                IsIconic, IsWindowVisible, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SetWindowPos, ShowWindow,
            },
        },
    },
    core::{BOOL, BSTR},
};

/// The programs a media session's app ID stands for. Browsers report a
/// short name (or, for Firefox, a hash of its install path); desktop apps
/// often report their executable.
pub fn programs_for(player: &str) -> Vec<String> {
    let lower = player.to_ascii_lowercase();
    let known: &[(&str, &str)] = &[
        ("chrome", "chrome.exe"),
        ("msedge", "msedge.exe"),
        ("edge", "msedge.exe"),
        ("brave", "brave.exe"),
        ("opera", "opera.exe"),
        ("vivaldi", "vivaldi.exe"),
        ("arc", "arc.exe"),
        ("firefox", "firefox.exe"),
        ("zen", "zen.exe"),
        ("floorp", "floorp.exe"),
        ("librewolf", "librewolf.exe"),
        ("waterfox", "waterfox.exe"),
        // Firefox's app ID is a hash of where it's installed; this is the
        // one for its default location. Other installs (and Firefox-based
        // browsers) have hashes of their own: their window is found by its
        // title (see `window_for`).
        ("308046b0af4a39cb", "firefox.exe"),
    ];
    let mut programs: Vec<String> =
        known.iter().filter(|(id, _)| lower.contains(id)).map(|(_, exe)| (*exe).to_owned()).collect();
    // "vlc.exe", or a path, or "VideoLAN.VLC" (a desktop app ID).
    let last = lower.rsplit(['\\', '/']).next().unwrap_or(&lower);
    if last.ends_with(".exe") {
        programs.push(last.to_owned());
    } else if let Some((_, name)) = last.rsplit_once('.')
        && !name.is_empty()
        && !last.contains('!')
    {
        programs.push(format!("{name}.exe"));
    }
    programs.dedup();
    programs
}

/// Whether `program` (an executable's file name) is a browser whose
/// address bar and page this module can read.
pub fn is_browser(program: &str) -> bool {
    matches!(
        program,
        "chrome.exe"
            | "msedge.exe"
            | "brave.exe"
            | "opera.exe"
            | "vivaldi.exe"
            | "arc.exe"
            | "firefox.exe"
            | "zen.exe"
            | "floorp.exe"
            | "librewolf.exe"
            | "waterfox.exe"
    )
}

/// A visible top-level window.
#[derive(Debug, Clone)]
pub struct Window {
    pub hwnd: HWND,
    /// Its program's file name, lowercase ("chrome.exe").
    pub program: String,
    pub title: String,
}

/// The visible, uncloaked top-level windows that aren't minimized, front
/// to back.
pub fn windows() -> Vec<Window> {
    listed(false)
}

/// As [`windows`], with minimized ones too: a minimized player still plays.
fn windows_and_minimized() -> Vec<Window> {
    listed(true)
}

fn listed(minimized: bool) -> Vec<Window> {
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the Vec passed to EnumWindows below, alive for the call.
        let found = unsafe { &mut *(found.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        BOOL(1)
    }
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes to `handles`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut handles as *mut Vec<HWND> as isize)) };
    handles
        .into_iter()
        .filter(|&hwnd| shown(hwnd, minimized))
        .filter_map(|hwnd| Some(Window { hwnd, program: program_of(hwnd)?, title: title_of(hwnd) }))
        .collect()
}

fn shown(hwnd: HWND, minimized: bool) -> bool {
    // SAFETY: plain queries on a window handle (a stale one just fails).
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || (!minimized && IsIconic(hwnd).as_bool()) {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        );
        cloaked == 0
    }
}

fn title_of(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer's length bounds what's written.
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn program_of(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    // SAFETY: plain process queries; the handle is closed below.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        named.ok()?;
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Some(path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase())
    }
}

/// The window that plays `title` for the media session `player`: one of
/// its program's windows whose title has the media's title in it (a
/// browser's shows its active tab's), or its only window.
pub fn window_for(player: &str, title: &str) -> Option<Window> {
    let programs = programs_for(player);
    let all = windows_and_minimized();
    let candidates: Vec<Window> = all.iter().filter(|w| programs.contains(&w.program)).cloned().collect();
    pick(candidates, title).or_else(|| by_title(all, title))
}

/// The one window (of any program but this app) whose title has the
/// media's title in it, for players whose app ID names no program (a hash,
/// as Firefox-based browsers report).
fn by_title(windows: Vec<Window>, title: &str) -> Option<Window> {
    let wanted = title.trim().to_lowercase();
    if wanted.chars().count() < 3 {
        return None;
    }
    let mut matching = windows
        .into_iter()
        .filter(|w| w.program != "nectarlink-desktop.exe" && w.title.to_lowercase().contains(&wanted));
    let found = matching.next()?;
    // Two windows with that title: can't tell which one plays.
    matching.next().is_none().then_some(found)
}

fn pick(candidates: Vec<Window>, title: &str) -> Option<Window> {
    let wanted = title.trim().to_lowercase();
    if !wanted.is_empty()
        && let Some(found) = candidates.iter().find(|w| w.title.to_lowercase().contains(&wanted))
    {
        return Some(found.clone());
    }
    match candidates.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Has this thread see the screen in physical pixels, as screen captures
/// do, whatever the process's DPI awareness: UI Automation's places then
/// match what a capture of the screen shows.
pub fn use_physical_pixels() {
    // SAFETY: changes this thread's DPI awareness only.
    let _ = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// UI Automation, on a thread that has COM.
pub struct Automation(IUIAutomation);

impl Automation {
    pub fn new() -> windows::core::Result<Automation> {
        // SAFETY: creates the system's UI Automation client.
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.map(Automation)
    }

    fn find(
        &self,
        root: &IUIAutomationElement,
        property: UIA_PROPERTY_ID,
        value: VARIANT,
    ) -> Option<IUIAutomationElement> {
        // SAFETY: COM calls on live interfaces.
        unsafe {
            let condition = self.0.CreatePropertyCondition(property, &value).ok()?;
            root.FindFirst(TreeScope_Descendants, &condition).ok()
        }
    }

    fn root(&self, window: &Window) -> Option<IUIAutomationElement> {
        // SAFETY: a stale handle just fails.
        unsafe { self.0.ElementFromHandle(window.hwnd).ok() }
    }

    /// The address of the page the browser `window` shows, with its scheme
    /// (address bars leave "https://" out).
    pub fn page_url(&self, window: &Window) -> Option<String> {
        let root = self.root(window)?;
        let bar = self
            .find(&root, UIA_ClassNamePropertyId, VARIANT::from("OmniboxViewViews"))
            .or_else(|| self.find(&root, UIA_AutomationIdPropertyId, VARIANT::from("urlbar-input")))
            .or_else(|| self.find(&root, UIA_AutomationIdPropertyId, VARIANT::from("addressBarTextBox")))
            .or_else(|| self.find(&root, UIA_AutomationIdPropertyId, VARIANT::from("AddressBar")));
        let bar = bar?;
        // SAFETY: a COM call on a live element.
        let value = unsafe { bar.GetCurrentPropertyValue(UIA_ValueValuePropertyId) }.ok()?;
        let text = BSTR::try_from(&value).ok()?.to_string();
        with_scheme(&text).filter(|u| nectarlink_core::is_valid_web_url(u))
    }

    /// Where on screen the video plays in `window`, in physical pixels:
    /// YouTube's player when it's there, else the page, else the window.
    #[cfg(test)]
    pub fn video_rect(&self, window: &Window) -> Option<RECT> {
        self.player_rect(window).or_else(|| self.page_rect(window)).or_else(|| frame_of(window.hwnd))
    }

    /// Where YouTube's player is in `window`, once the browser exposes its
    /// page (a moment after it's first asked).
    pub fn player_rect(&self, window: &Window) -> Option<RECT> {
        let root = self.root(window)?;
        self.rect_of(self.find(
            &root,
            UIA_ClassNamePropertyId,
            VARIANT::from("video-stream html5-main-video"),
        ))
    }

    /// Where the page is in `window`: Chromium's page, or the selected
    /// tab's area in Firefox-based browsers (whose pages UI Automation
    /// doesn't show, so their video can't be found).
    pub fn page_rect(&self, window: &Window) -> Option<RECT> {
        let root = self.root(window)?;
        self.rect_of(self.find(&root, UIA_ControlTypePropertyId, VARIANT::from(UIA_DocumentControlTypeId.0)))
            .or_else(|| {
                self.rect_of(self.find(
                    &root,
                    UIA_ClassNamePropertyId,
                    VARIANT::from("browserSidebarContainer deck-selected"),
                ))
            })
    }

    fn rect_of(&self, element: Option<IUIAutomationElement>) -> Option<RECT> {
        // SAFETY: a COM call on a live element.
        let rect = element.and_then(|e| unsafe { e.CurrentBoundingRectangle() }.ok());
        rect.filter(|r| r.right - r.left >= 64 && r.bottom - r.top >= 64)
    }
}

/// A window's frame on screen, without its invisible resize borders.
/// Whether `hwnd` is minimized: Windows draws nothing of it then, so a
/// capture of it stays on its last picture.
pub fn is_minimized(hwnd: HWND) -> bool {
    // SAFETY: a plain query on a window handle.
    unsafe { IsIconic(hwnd).as_bool() }
}

/// Restores a minimized window behind every other one, without bringing it
/// forward or taking the focus: it draws again, out of the way.
pub fn restore_behind(hwnd: HWND) {
    // SAFETY: plain window calls on a handle (a stale one just fails).
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn frame_of(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: writes one RECT.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

/// An address bar's text as a web address: "youtube.com/watch?v=x" becomes
/// "https://youtube.com/watch?v=x". Anything that isn't a web page (a
/// search being typed, a browser's own pages) is `None`.
fn with_scheme(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(text.to_owned());
    }
    if lower.contains("://") || lower.starts_with("about:") {
        return None;
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    (host.contains('.') || host == "localhost").then(|| format!("https://{text}"))
}

/// Whether a browser window is in Incognito / InPrivate / Private Browsing mode.
/// Checks browser-added window title markers while avoiding false positives on
/// normal web pages whose `<title>` happens to include words like "private".
pub fn is_private_browser_window(program: &str, title: &str) -> bool {
    let prog = program.trim().to_ascii_lowercase();
    if matches!(prog.as_str(), "tor.exe" | "torbrowser.exe") {
        return true;
    }
    let lower = title.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    // Chromium / Edge / Brave / Opera / Vivaldi / Arc / Firefox private-window markers.
    // Browsers append their own window suffix at the end, so avoid loose prefixes
    // on generic titles (e.g. a Wikipedia article titled "Private browsing").
    lower.ends_with("(incognito)")
        || lower.ends_with("- incognito")
        || lower.starts_with("new incognito tab - ")
        || (prog == "arc.exe" && (lower == "incognito - arc" || lower.starts_with("incognito - ")))
        || lower.contains("[inprivate]")
        || lower.contains("(inprivate)")
        || lower.contains(" - inprivate - ")
        || lower.starts_with("new inprivate tab - ")
        || lower.ends_with("private browsing")
        || lower.ends_with("(private browsing)")
        || (prog == "opera.exe"
            && (lower == "private browsing - opera" || lower.starts_with("private browsing - ")))
        || lower.ends_with("(private)")
        || lower.ends_with("(private with tor)")
        || lower.starts_with("new private tab - ")
        || lower.starts_with("new private window - ")
}

/// Reads the active tab's `http://`/`https://` URL from `hwnd` (or the current
/// foreground window when `hwnd == 0`), skipping private/Incognito windows.
pub fn active_browser_url_for_hwnd(hwnd: isize) -> crate::links::ActiveBrowserUrl {
    // SAFETY: GetForegroundWindow and window queries read Win32 handle state only.
    let target = if hwnd != 0 {
        HWND(hwnd as *mut _)
    } else {
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
    };
    if target.0.is_null() || !shown(target, false) {
        return crate::links::ActiveBrowserUrl::NoBrowser;
    }
    let Some(program) = program_of(target) else {
        return crate::links::ActiveBrowserUrl::NoBrowser;
    };
    if !is_browser(&program) {
        return crate::links::ActiveBrowserUrl::NoBrowser;
    }
    let title = title_of(target);
    if is_private_browser_window(&program, &title) {
        return crate::links::ActiveBrowserUrl::PrivateWindow;
    }
    // SAFETY: Initializes COM on this worker thread before creating IUIAutomation;
    // CoUninitialize is paired only when CoInitializeEx succeeds on this call.
    let com_hr = unsafe {
        windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED)
    };
    let window = Window { hwnd: target, program: program.clone(), title };
    let res = match Automation::new() {
        Ok(automation) => match automation.page_url(&window) {
            Some(url) if nectarlink_core::is_valid_web_url(&url) => {
                crate::links::ActiveBrowserUrl::Url { url, program }
            }
            _ => crate::links::ActiveBrowserUrl::NoUrl,
        },
        Err(_) => crate::links::ActiveBrowserUrl::NoUrl,
    };
    if com_hr.is_ok() {
        // SAFETY: Matches the successful CoInitializeEx call above on this thread.
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_map_to_their_programs() {
        assert_eq!(programs_for("Chrome"), ["chrome.exe"]);
        assert_eq!(programs_for("MSEdge"), ["msedge.exe"]);
        assert_eq!(programs_for("308046B0AF4A39CB"), ["firefox.exe"]);
        assert_eq!(programs_for("vlc.exe"), ["vlc.exe"]);
        assert_eq!(programs_for(r"C:\Program Files\VideoLAN\VLC\vlc.exe"), ["vlc.exe"]);
        assert_eq!(programs_for("VideoLAN.VLC"), ["vlc.exe"]);
        // A packaged app's ID: no program to find a window by.
        assert!(programs_for("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify").is_empty());
    }

    #[test]
    fn address_bar_text_becomes_a_web_address() {
        assert_eq!(
            with_scheme("youtube.com/watch?v=abc").as_deref(),
            Some("https://youtube.com/watch?v=abc")
        );
        assert_eq!(
            with_scheme("https://www.netflix.com/watch/1").as_deref(),
            Some("https://www.netflix.com/watch/1")
        );
        assert_eq!(with_scheme("127.0.0.1:8765").as_deref(), Some("https://127.0.0.1:8765"));
        assert_eq!(with_scheme("cats on skateboards"), None, "a search being typed");
        assert_eq!(with_scheme("chrome://settings"), None);
        assert_eq!(with_scheme("about:blank"), None);
        assert_eq!(with_scheme("settings"), None);
        assert_eq!(with_scheme(""), None);
    }

    fn window(title: &str) -> Window {
        Window { hwnd: HWND::default(), program: "chrome.exe".into(), title: title.into() }
    }

    #[test]
    fn a_hashed_app_id_finds_its_window_by_title() {
        let zen = Window {
            hwnd: HWND::default(),
            program: "zen.exe".into(),
            title: "A Trailer - YouTube - Zen Browser".into(),
        };
        let mail = window("Mail - Google Chrome");
        assert_eq!(by_title(vec![mail.clone(), zen.clone()], "A Trailer").unwrap().program, "zen.exe");
        // Ambiguous or too short: none.
        assert!(by_title(vec![zen.clone(), zen.clone()], "A Trailer").is_none());
        assert!(by_title(vec![zen], "A").is_none());
        assert!(programs_for("F0DC299D809B9700").is_empty());
        assert!(is_browser("zen.exe"));
    }

    #[test]
    fn the_window_with_the_title_is_the_player() {
        let picked =
            pick(vec![window("Mail - Google Chrome"), window("A Song - YouTube - Google Chrome")], "a song");
        assert_eq!(picked.unwrap().title, "A Song - YouTube - Google Chrome");
        // The playing tab isn't in front in any window: can't tell which.
        assert!(pick(vec![window("Mail"), window("News")], "A Song").is_none());
        // One window: that's it, whatever its tab.
        assert_eq!(pick(vec![window("Mail")], "A Song").unwrap().title, "Mail");
    }

    /// With a Chromium browser showing a page whose title has
    /// `NECTARLINK_TEST_TAB` in it: reads its address and its video's place.
    #[test]
    #[ignore = "needs a browser tab titled NECTARLINK_TEST_TAB"]
    fn reads_a_real_browser() {
        let wanted = std::env::var("NECTARLINK_TEST_TAB").expect("NECTARLINK_TEST_TAB");
        // SAFETY: COM for this test's thread.
        let _ = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        };
        use_physical_pixels();
        let window = window_for("Chrome", &wanted).expect("the test tab's window");
        let automation = Automation::new().unwrap();
        let url = automation.page_url(&window);
        let rect = automation.video_rect(&window).unwrap();
        eprintln!("{:?} {url:?} {rect:?}", window.title);
        assert!(url.is_some_and(|u| u.starts_with("http")));
        assert!(rect.right - rect.left >= 64);
    }

    #[test]
    fn detects_private_and_incognito_browser_windows_without_false_positives() {
        assert!(is_browser("arc.exe"));
        assert!(is_private_browser_window("chrome.exe", "Docs - Google Chrome (Incognito)"));
        assert!(is_private_browser_window("chrome.exe", "New Incognito Tab - Google Chrome"));
        assert!(is_private_browser_window("msedge.exe", "News - [InPrivate] - Microsoft\u{200b} Edge"));
        assert!(is_private_browser_window("msedge.exe", "New InPrivate tab - Microsoft Edge"));
        assert!(is_private_browser_window("brave.exe", "Search - Brave (Private)"));
        assert!(is_private_browser_window("brave.exe", "Search - Brave (Private with Tor)"));
        assert!(is_private_browser_window("firefox.exe", "Example — Mozilla Firefox Private Browsing"));
        assert!(is_private_browser_window("opera.exe", "Private browsing - Opera"));
        assert!(is_private_browser_window("vivaldi.exe", "Page - Vivaldi (Private)"));
        assert!(is_private_browser_window("arc.exe", "Incognito - Arc"));
        assert!(is_private_browser_window("torbrowser.exe", "Anything"));

        assert!(!is_private_browser_window("chrome.exe", "Private browsing - Wikipedia - Google Chrome"));
        assert!(!is_private_browser_window("chrome.exe", "My Private Notes - Google Chrome"));
        assert!(!is_private_browser_window("msedge.exe", "Dashboard - Personal - Microsoft\u{200b} Edge"));
        assert!(!is_private_browser_window("firefox.exe", "GitHub - Mozilla Firefox"));
    }
}
