// SPDX-License-Identifier: GPL-3.0-or-later
//! `Mirror`: the phone screens and app windows shown on this PC (each a
//! window, by key), the phone's apps that open in windows, and the input
//! QML sends to them.

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use nectarlink_core::{FeatureState, MIRROR_SCREEN, MirrorInput};

use crate::{
    mirror::{self, Apps, Phase, Window},
    state::Changes,
};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// The windows, as JSON: `[{ key, device, session, name, title,
        /// phase ("asking", "showing", "ended"), reason, canControl, sound,
        /// app }]`.
        #[qproperty(QString, windows)]
        /// The user turned the phones' sound off on this PC.
        #[qproperty(bool, muted)]
        /// The phone whose apps [`apps`] lists.
        #[qproperty(QString, apps_device, cxx_name = "appsDevice")]
        /// Its apps, as JSON: `[{ pkg, label, icon }]` (icon: a file URL or "").
        #[qproperty(QString, apps)]
        /// The apps opened from this PC most recently on `appsDevice`, as JSON:
        /// `[{ pkg, label, icon }]` (up to 6).
        #[qproperty(QString, recent_apps, cxx_name = "recentApps")]
        /// "", "loading", "ready" or "failed".
        #[qproperty(QString, apps_state, cxx_name = "appsState")]
        /// Why they couldn't be loaded.
        #[qproperty(QString, apps_error, cxx_name = "appsError")]
        /// Device ID currently receiving PC keyboard input without screen mirroring, or "".
        #[qproperty(QString, keyboard_device, cxx_name = "keyboardDevice")]
        type Mirror = super::MirrorRust;
    }

    impl cxx_qt::Threading for Mirror {}
    impl cxx_qt::Initialize for Mirror {}

    unsafe extern "RustQt" {
        /// Asks a phone for its screen (a window opens).
        #[qinvokable]
        fn start(self: &Mirror, device: &QString);
        /// Opens one of a phone's apps in a window of its own.
        #[qinvokable]
        #[cxx_name = "startApp"]
        fn start_app(self: &Mirror, device: &QString, pkg: &QString, label: &QString);
        /// Reopens an ended window (the screen or an app window).
        #[qinvokable]
        fn reopen(self: &Mirror, key: &QString);
        /// Closes a window, ending its mirroring.
        #[qinvokable]
        fn stop(self: &Mirror, key: &QString);
        /// Resizes an app window's display on the phone.
        #[qinvokable]
        fn resize(self: &Mirror, key: &QString, width: u32, height: u32);
        /// Saves an app window's position and size on this PC.
        #[qinvokable]
        #[cxx_name = "saveGeometry"]
        fn save_geometry(self: &Mirror, key: &QString, x: i32, y: i32, width: u32, height: u32);
        /// Loads a phone's apps (into `apps`).
        #[qinvokable]
        #[cxx_name = "loadApps"]
        fn load_apps(self: Pin<&mut Mirror>, device: &QString);
        /// A finger on a window's screen: "down", "move" or "up", at a
        /// fraction of its width and height.
        #[qinvokable]
        fn touch(self: &Mirror, key: &QString, action: &QString, x: f64, y: f64);
        /// The mouse wheel, in notches (positive: down / right).
        #[qinvokable]
        fn scroll(self: &Mirror, key: &QString, x: f64, y: f64, dx: f64, dy: f64);
        /// A key without text ("back", "home", "enter"...).
        #[qinvokable]
        fn press(self: &Mirror, key: &QString, name: &QString);
        /// Typed text.
        #[qinvokable]
        fn text(self: &Mirror, key: &QString, text: &QString);
        /// Types the PC's clipboard text on the phone.
        #[qinvokable]
        fn paste(self: &Mirror, key: &QString);
        /// Turns the phones' sound off on this PC, or back on.
        #[qinvokable]
        #[cxx_name = "toggleSound"]
        fn toggle_sound(self: &Mirror);
        /// Copies a screenshot of a mirror window to the PC clipboard.
        #[qinvokable]
        #[cxx_name = "screenshotClipboard"]
        fn screenshot_clipboard(self: &Mirror, key: &QString);
        /// Saves a screenshot of a mirror window to Downloads\Nectarlink.
        #[qinvokable]
        #[cxx_name = "screenshotFile"]
        fn screenshot_file(self: &Mirror, key: &QString);
        /// Starts or stops MP4 video recording for a mirror window.
        #[qinvokable]
        #[cxx_name = "toggleRecording"]
        fn toggle_recording(self: &Mirror, key: &QString);
        /// Keeps the phone awake while mirroring its screen.
        #[qinvokable]
        #[cxx_name = "setStayAwake"]
        fn set_stay_awake(self: &Mirror, key: &QString, on: bool);
        /// Turns the phone's physical screen off while mirroring.
        #[qinvokable]
        #[cxx_name = "setScreenOff"]
        fn set_screen_off(self: &Mirror, key: &QString, on: bool);
        /// Starts or stops remote keyboard typing on `device` without screen mirroring.
        #[qinvokable]
        #[cxx_name = "toggleKeyboard"]
        fn toggle_keyboard(self: &Mirror, device: &QString);
        /// Stops remote keyboard typing without screen mirroring.
        #[qinvokable]
        #[cxx_name = "stopKeyboard"]
        fn stop_keyboard(self: &Mirror);
        /// Sends a key name to `device` while remote keyboard typing is active.
        #[qinvokable]
        #[cxx_name = "keyboardPress"]
        fn keyboard_press(self: &Mirror, device: &QString, name: &QString);
        /// Sends typed text to `device` while remote keyboard typing is active.
        #[qinvokable]
        #[cxx_name = "keyboardText"]
        fn keyboard_text(self: &Mirror, device: &QString, text: &QString);
        /// Pastes the PC's clipboard text onto `device` while remote keyboard typing is active.
        #[qinvokable]
        #[cxx_name = "keyboardPaste"]
        fn keyboard_paste(self: &Mirror, device: &QString);
    }
}

#[derive(Default)]
pub struct MirrorRust {
    windows: QString,
    muted: bool,
    apps_device: QString,
    apps: QString,
    recent_apps: QString,
    apps_state: QString,
    apps_error: QString,
    keyboard_device: QString,
}

impl cxx_qt::Initialize for qobject::Mirror {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(
            self.qt_thread(),
            Changes::MIRROR
                | Changes::CAPABILITIES
                | Changes::DEVICES
                | Changes::NOTIFICATIONS
                | Changes::HISTORY,
            Self::refresh,
        );
    }
}

impl qobject::Mirror {
    fn refresh(mut self: Pin<&mut Self>) {
        let host = crate::core_host::host();
        let hub = &host.hub;
        let data_dir = &host.data_dir;
        let windows: Vec<serde_json::Value> = mirror::windows()
            .into_iter()
            .map(|(window, shown)| {
                let (control, can_screen_off, name) = hub.read(|s| {
                    // The screen needs control turned on; app windows are Elevated already.
                    let feature = if window.session == MIRROR_SCREEN {
                        "mirroring.control"
                    } else {
                        "mirroring.app_windows"
                    };
                    let matrix = s.matrices.get(&window.device);
                    let control = matrix.and_then(|m| m.state(feature)) == Some(FeatureState::Available);
                    let can_screen_off = matrix.and_then(|m| m.state("mirroring.app_windows"))
                        == Some(FeatureState::Available);
                    (control, can_screen_off, s.name_of(&window.device).unwrap_or_default())
                });
                let (phase, reason) = match shown.phase {
                    Phase::Asking => ("asking", String::new()),
                    Phase::Showing => ("showing", String::new()),
                    Phase::Ended(reason) => ("ended", reason.unwrap_or_default()),
                };
                let pkg = shown.pkg.clone().unwrap_or_default();
                let icon = (!pkg.is_empty())
                    .then(|| crate::icons::existing(data_dir, &pkg))
                    .flatten()
                    .as_deref()
                    .map(crate::icons::file_url)
                    .unwrap_or_default();
                let geom = (!pkg.is_empty()).then(|| mirror::geometry(&window.device, &pkg)).flatten();
                serde_json::json!({
                    "key": window.key(),
                    "device": window.device.to_string(),
                    "session": window.session,
                    "name": name,
                    "title": shown.app.clone().unwrap_or_else(|| name.clone()),
                    "pkg": pkg,
                    "icon": icon,
                    "app": shown.app.is_some(),
                    "phase": phase,
                    "reason": reason,
                    "canControl": control,
                    "sound": window.session == MIRROR_SCREEN && mirror::has_sound(&window.device),
                    "recording": mirror::is_recording(&window),
                    "recordingStartedMs": mirror::recording_started_ms(&window),
                    "stayAwake": window.session == MIRROR_SCREEN && mirror::stay_awake(&window.device),
                    "screenOff": window.session == MIRROR_SCREEN && mirror::screen_off(&window.device),
                    "canScreenOff": window.session == MIRROR_SCREEN && can_screen_off,
                    "notice": mirror::last_notice(&window),
                    "savedX": geom.map(|g| g.x),
                    "savedY": geom.map(|g| g.y),
                    "savedWidth": geom.map(|g| g.width).unwrap_or(0),
                    "savedHeight": geom.map(|g| g.height).unwrap_or(0),
                })
            })
            .collect();
        let json = serde_json::Value::Array(windows).to_string();
        if self.windows.to_string() != json {
            self.as_mut().set_windows(QString::from(&json));
        }
        self.as_mut().set_muted(mirror::muted());
        let kb = mirror::keyboard_device().map(|d| d.to_string()).unwrap_or_default();
        if self.keyboard_device.to_string() != kb {
            self.as_mut().set_keyboard_device(QString::from(&kb));
        }

        let Some(device) = super::parse_device(&self.apps_device) else { return };
        let (state, apps, error) = match mirror::apps(&device) {
            None => ("", Vec::new(), String::new()),
            Some(Apps::Loading) => ("loading", Vec::new(), String::new()),
            Some(Apps::Failed(why)) => ("failed", Vec::new(), why),
            Some(Apps::Ready(apps)) => ("ready", apps, String::new()),
        };
        let recent_pkgs = mirror::recent_apps(&device);
        let recent: Vec<serde_json::Value> = recent_pkgs
            .iter()
            .filter_map(|pkg| apps.iter().find(|a| &a.pkg == pkg))
            .take(6)
            .map(|a| {
                serde_json::json!({
                    "pkg": a.pkg,
                    "label": a.label,
                    "icon": a.icon.as_deref().map(crate::icons::file_url).unwrap_or_default(),
                })
            })
            .collect();
        let apps: Vec<serde_json::Value> = apps
            .into_iter()
            .map(|a| {
                serde_json::json!({
                    "pkg": a.pkg,
                    "label": a.label,
                    "icon": a.icon.as_deref().map(crate::icons::file_url).unwrap_or_default(),
                })
            })
            .collect();
        self.as_mut().set_recent_apps(QString::from(&serde_json::Value::Array(recent).to_string()));
        self.as_mut().set_apps(QString::from(&serde_json::Value::Array(apps).to_string()));
        self.as_mut().set_apps_error(QString::from(&error));
        self.as_mut().set_apps_state(QString::from(state));
    }

    pub fn start(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            mirror::start(device);
        }
    }

    pub fn start_app(&self, device: &QString, pkg: &QString, label: &QString) {
        let pkg = String::from(pkg);
        if let (Some(device), true) = (super::parse_device(device), nectarlink_core::is_package_name(&pkg)) {
            mirror::start_app(device, pkg, String::from(label));
        }
    }

    pub fn reopen(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::reopen(window);
        }
    }

    pub fn stop(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::stop(window);
        }
    }

    pub fn resize(&self, key: &QString, width: u32, height: u32) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::resize(window, width, height);
        }
    }

    pub fn save_geometry(&self, key: &QString, x: i32, y: i32, width: u32, height: u32) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::save_geometry(window, x, y, width, height);
        }
    }

    pub fn load_apps(mut self: Pin<&mut Self>, device: &QString) {
        let Some(id) = super::parse_device(device) else { return };
        self.as_mut().set_apps_device(device.clone());
        mirror::load_apps(id);
        self.refresh();
    }

    fn send(&self, key: &QString, input: MirrorInput) {
        let Some(window) = Window::parse(&String::from(key)) else { return };
        if matches!(mirror::phase(&window), Some(Phase::Showing)) {
            mirror::input(window, input);
        }
    }

    pub fn touch(&self, key: &QString, action: &QString, x: f64, y: f64) {
        use nectarlink_core::TouchAction;
        let action = match String::from(action).as_str() {
            "down" => TouchAction::Down,
            "move" => TouchAction::Move,
            "up" => TouchAction::Up,
            _ => return,
        };
        self.send(key, MirrorInput::Touch { action, x: fraction(x), y: fraction(y) });
    }

    pub fn scroll(&self, key: &QString, x: f64, y: f64, dx: f64, dy: f64) {
        self.send(
            key,
            MirrorInput::Scroll {
                x: fraction(x),
                y: fraction(y),
                dx: dx.clamp(-100.0, 100.0) as f32,
                dy: dy.clamp(-100.0, 100.0) as f32,
            },
        );
    }

    pub fn press(&self, key: &QString, name: &QString) {
        self.send(key, MirrorInput::Key { key: String::from(name) });
    }

    pub fn text(&self, key: &QString, text: &QString) {
        let text = String::from(text);
        if !text.is_empty() && text.len() <= nectarlink_core::MIRROR_MAX_TEXT_BYTES {
            self.send(key, MirrorInput::Text { text });
        }
    }

    pub fn paste(&self, key: &QString) {
        if let crate::win::clipboard::Clip::Text(text) = crate::win::clipboard::read() {
            // Long text goes in pieces, each a whole number of characters.
            let mut piece = String::new();
            for c in text.chars() {
                if piece.len() + c.len_utf8() > nectarlink_core::MIRROR_MAX_TEXT_BYTES {
                    self.send(key, MirrorInput::Text { text: std::mem::take(&mut piece) });
                }
                piece.push(c);
            }
            if !piece.is_empty() {
                self.send(key, MirrorInput::Text { text: piece });
            }
        }
    }

    pub fn toggle_sound(&self) {
        mirror::set_muted(!mirror::muted());
    }

    pub fn screenshot_clipboard(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::screenshot_clipboard(window);
        }
    }

    pub fn screenshot_file(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::screenshot_file(window);
        }
    }

    pub fn toggle_recording(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::toggle_recording(window);
        }
    }

    pub fn set_stay_awake(&self, key: &QString, on: bool) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::set_stay_awake(window, on);
        }
    }

    pub fn set_screen_off(&self, key: &QString, on: bool) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::set_screen_off(window, on);
        }
    }

    pub fn toggle_keyboard(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            mirror::toggle_remote_keyboard(device);
        }
    }

    pub fn stop_keyboard(&self) {
        mirror::set_remote_keyboard(None);
    }

    pub fn keyboard_press(&self, device: &QString, name: &QString) {
        if let Some(device) = super::parse_device(device) {
            mirror::keyboard_press(device, String::from(name));
        }
    }

    pub fn keyboard_text(&self, device: &QString, text: &QString) {
        if let Some(device) = super::parse_device(device) {
            mirror::keyboard_text(device, String::from(text));
        }
    }

    pub fn keyboard_paste(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            mirror::keyboard_paste(device);
        }
    }
}

/// A position on the screen, kept on it.
fn fraction(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}
