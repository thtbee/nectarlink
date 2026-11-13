// SPDX-License-Identifier: GPL-3.0-or-later
//! `Mirror`: the phone screens and app windows shown on this PC (each a
//! window, by key), the phone's apps that open in windows, and the input
//! QML sends to them.

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::{QList, QString, QUrl, QVariant};
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
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
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
        /// The phones whose screen stays off, as a JSON list of device IDs.
        #[qproperty(QString, keep_screen_off_phones)]
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
        /// Makes an app window's app draw bigger (`steps` > 0) or smaller;
        /// the next `resize` applies it. False when nothing changed.
        #[qinvokable]
        #[cxx_name = "changeZoom"]
        fn change_zoom(self: &Mirror, key: &QString, steps: i32) -> bool;
        /// Applies the zoom to a window that isn't flexible, whose picture
        /// is `width` x `height` pixels.
        #[qinvokable]
        #[cxx_name = "zoomDisplay"]
        fn zoom_display(self: &Mirror, key: &QString, width: u32, height: u32);
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
        /// Turns an app window's flexible display on or off (off restarts
        /// the app at the phone's resolution).
        #[qinvokable]
        #[cxx_name = "setFlexible"]
        fn set_flexible(self: &Mirror, key: &QString, on: bool);
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
        /// Keeps `device`'s screen off from the start of each mirroring or app window.
        #[qinvokable]
        #[cxx_name = "setKeepScreenOff"]
        fn set_keep_screen_off(self: &Mirror, device: &QString, on: bool);
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
        /// Sends files dropped onto a mirror window to the mirrored phone.
        #[qinvokable]
        #[cxx_name = "dropFiles"]
        fn drop_files(self: &Mirror, urls: &QList_QVariant);
        /// Synchronizes the clipboard between PC and phone when a mirror window gains focus.
        #[qinvokable]
        #[cxx_name = "syncClipboardOnFocus"]
        fn sync_clipboard_on_focus(self: &Mirror, key: &QString);
        /// Requests remote input control from the phone for a mirror window.
        #[qinvokable]
        #[cxx_name = "requestControl"]
        fn request_control(self: &Mirror, key: &QString);
        /// Notifies PC-to-phone cursor/keyboard Flow when the mouse enters a mirror window.
        #[qinvokable]
        #[cxx_name = "enterWindow"]
        fn enter_window(self: &Mirror, key: &QString);
        /// Notifies PC-to-phone cursor/keyboard Flow when the mouse leaves a mirror window.
        #[qinvokable]
        #[cxx_name = "leaveWindow"]
        fn leave_window(self: &Mirror);
    }
}

#[derive(Default)]
pub struct MirrorRust {
    windows: QString,
    keep_screen_off_phones: QString,
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
                let power_buttons = mirror::power_matters(&window);
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
                let geom = if window.session == MIRROR_SCREEN {
                    mirror::geometry(&window.device, mirror::SCREEN_GEOMETRY)
                } else {
                    (!pkg.is_empty()).then(|| mirror::geometry(&window.device, &pkg)).flatten()
                };
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
                    // An app window has the phone's sound while it plays here (#121).
                    "sound": if window.session == MIRROR_SCREEN {
                        mirror::has_sound(&window.device)
                    } else {
                        crate::phone_sound::playing(&window.device)
                    },
                    "recording": mirror::is_recording(&window),
                    "recordingStartedMs": mirror::recording_started_ms(&window),
                    // Off where the window has no say in them (and no buttons).
                    "stayAwake": power_buttons && mirror::stay_awake(&window.device),
                    "screenOff": power_buttons && mirror::screen_off(&window.device),
                    "canScreenOff": can_screen_off,
                    // Stay awake and screen off: only where the phone's power matters to this window.
                    "powerButtons": power_buttons,
                    "flexible": mirror::flexible(&window),
                    "zoom": mirror::zoom(&window),
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
        let keep = serde_json::Value::from(crate::settings::keep_screen_off_phones()).to_string();
        if self.keep_screen_off_phones.to_string() != keep {
            self.as_mut().set_keep_screen_off_phones(QString::from(&keep));
        }
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
        let recent_qstr = QString::from(&serde_json::Value::Array(recent).to_string());
        if *self.recent_apps() != recent_qstr {
            self.as_mut().set_recent_apps(recent_qstr);
        }
        let apps_qstr = QString::from(&serde_json::Value::Array(apps).to_string());
        if *self.apps() != apps_qstr {
            self.as_mut().set_apps(apps_qstr);
        }
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

    pub fn change_zoom(&self, key: &QString, steps: i32) -> bool {
        Window::parse(&String::from(key)).is_some_and(|window| mirror::change_zoom(window, steps))
    }

    pub fn zoom_display(&self, key: &QString, width: u32, height: u32) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::zoom_display(window, width, height);
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
        if let crate::win::clipboard::Clip::Text { text, .. } = crate::win::clipboard::read_for_paste() {
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

    pub fn set_flexible(&self, key: &QString, on: bool) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::set_flexible(window, on);
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

    pub fn set_keep_screen_off(&self, device: &QString, on: bool) {
        if let Some(device) = super::parse_device(device) {
            mirror::remember_screen_off(device, on);
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

    pub fn drop_files(&self, urls: &QList<QVariant>) {
        let mut raw_urls: Vec<String> = Vec::with_capacity(urls.len().max(0) as usize);
        for v in urls.iter() {
            let raw = v
                .value::<QString>()
                .map(|qs| String::from(&qs))
                .or_else(|| v.value::<QUrl>().map(|qu| String::from(&qu.to_qstring())))
                .unwrap_or_default();
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                raw_urls.push(trimmed.to_owned());
            }
        }
        mirror::drop_files(&raw_urls);
    }

    pub fn sync_clipboard_on_focus(&self, key: &QString) {
        if let Some(window) = Window::parse(&String::from(key)) {
            mirror::sync_clipboard_on_focus(window);
        }
    }

    pub fn request_control(&self, key: &QString) {
        mirror::request_control(&String::from(key));
    }

    pub fn enter_window(&self, key: &QString) {
        mirror::enter_window(&String::from(key));
    }

    pub fn leave_window(&self) {
        mirror::leave_window();
    }
}

/// A position on the screen, kept on it.
fn fraction(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}
