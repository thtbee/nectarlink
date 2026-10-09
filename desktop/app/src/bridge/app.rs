// SPDX-License-Identifier: GPL-3.0-or-later
//! `AppController`: app-wide state and commands for QML (core status, this
//! PC, capabilities, ringing, appearance) plus the tray icon.

use std::{
    pin::Pin,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{
    QHash, QHashPair_QString_QVariant, QList, QMap, QMapPair_QString_QVariant, QString, QVariant,
};
use nectarlink_core::{DeviceId, Error, FeatureState, LinkState, PhoneToggleValue, features::Upgrade};

use crate::{
    core_host,
    state::{Changes, CoreStatus},
    win::{self, sound, tray},
};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// "starting", "ready" or "failed".
        #[qproperty(QString, status)]
        #[qproperty(QString, error)]
        /// This PC's name and device ID, as phones see them.
        #[qproperty(QString, device_name)]
        #[qproperty(QString, device_id)]
        #[qproperty(bool, has_devices)]
        /// Bumped whenever any capability matrix changes, so bindings that
        /// call `featureState` re-evaluate.
        #[qproperty(i32, caps_revision)]
        /// Bumped when a device toggle is changed from this PC.
        #[qproperty(i32, toggles_revision)]
        /// Bumped whenever a phone's quick settings toggles change.
        #[qproperty(i32, phone_toggles_revision)]
        /// Name of the device making this PC ring ("" when not ringing).
        #[qproperty(QString, ringing_from)]
        #[qproperty(bool, system_dark)]
        #[qproperty(bool, reduce_motion)]
        /// Bloom colors from the desktop wallpaper, as JSON in the shape of
        /// a tokens.json seed (`{ seed, light, dark }`); "" until known.
        #[qproperty(QString, wallpaper_colors)]
        /// Material You color schemes for paired phones (`[{ id, key, name, label, hasAccent, scheme }]`).
        #[qproperty(QString, phone_colors)]
        /// Whether Windows shows this app's notifications.
        #[qproperty(bool, toasts_enabled)]
        #[qproperty(QString, version)]
        /// The Connection Doctor's findings, as JSON: [{ id, outcome:
        /// "ok" | "warn" | "fail", title, detail, fix? }].
        #[qproperty(QString, doctor_checks)]
        /// Checking, or fixing.
        #[qproperty(bool, doctor_busy)]
        /// This copy updates itself (it's installed).
        #[qproperty(bool, can_update)]
        /// A newer version that's available ("" when none).
        #[qproperty(QString, update_version)]
        /// Checking for or installing an update.
        #[qproperty(bool, update_busy)]
        /// Device ID and name of a phone asking to control this PC for the
        /// first time while `remote_input` is off ("" when none).
        #[qproperty(QString, remote_prompt_device_id)]
        #[qproperty(QString, remote_prompt_device_name)]
        /// Whether the presentation laser pointer overlay is showing, and its
        /// normalized position on the primary screen (`0.0..=1.0`).
        #[qproperty(bool, laser_active)]
        #[qproperty(f32, laser_x)]
        #[qproperty(f32, laser_y)]
        /// Primary physical adapter used for Wake-on-LAN ("" when none).
        #[qproperty(QString, wake_adapter)]
        /// `"enabled"`, `"disabled"`, `"unknown"`, or `"none"`.
        #[qproperty(QString, wake_state)]
        /// True when `wake_adapter` is wired Ethernet, false for Wi-Fi or none.
        #[qproperty(bool, wake_wired)]
        /// Selected navigation page and device index, kept in Rust across tray
        /// unloads so the window can be destroyed in the background.
        #[qproperty(QString, current_page)]
        #[qproperty(i32, current_device)]
        /// Phone number to pre-fill in the Calls page dialer when opened from a
        /// clipboard context chip ("" when none).
        #[qproperty(QString, pending_dial)]
        /// Encrypted local clipboard history as JSON:
        /// `[{ id, kind, text, imageDataUrl, deviceName, incoming, timestamp, pinned }]`.
        #[qproperty(QString, clipboard_history)]
        /// Latest 4 timeline entries for the Home page card, as JSON.
        #[qproperty(QString, timeline_preview)]
        /// Bumped whenever the Home card's summary (unread messages, missed calls,
        /// latest photo) changes for the watched phone.
        #[qproperty(i32, home_summary_revision)]
        /// Continuity Camera state (in-flight request and fallback paste prompt).
        #[qproperty(bool, continuity_camera_busy)]
        #[qproperty(QString, continuity_camera_status)]
        #[qproperty(QString, continuity_camera_mode)]
        #[qproperty(bool, continuity_paste_prompt_visible)]
        #[qproperty(QString, continuity_paste_prompt_title)]
        #[qproperty(QString, continuity_paste_prompt_body)]
        #[qproperty(QString, continuity_paste_target_name)]
        /// Command Palette state and latest open latency measurement.
        #[qproperty(bool, command_palette_open)]
        #[qproperty(QString, command_palette_results)]
        #[qproperty(f64, command_palette_open_ms)]
        /// Local storage and retention counts/sizes for Settings → Data & storage, as JSON.
        #[qproperty(QString, data_retention_summary)]
        /// LocalSend LAN sharing state and discovered peers (`[{ id, alias, deviceModel, deviceType, fingerprint, ip, port, protocol }]`).
        #[qproperty(bool, localsend_enabled)]
        #[qproperty(bool, localsend_receiving)]
        #[qproperty(QString, localsend_peers_json)]
        /// Slide-out edge Shelf state and JSON items (`""` while closed so nothing runs while hidden).
        #[qproperty(bool, shelf_open)]
        #[qproperty(QString, shelf_items)]
        type AppController = super::AppControllerRust;

        /// Enables or disables LocalSend LAN discovery and file sharing.
        #[qinvokable]
        fn toggle_localsend(self: Pin<&mut AppController>, enabled: bool);
        /// Triggers an immediate LocalSend multicast/HTTP discovery announcement.
        #[qinvokable]
        fn refresh_localsend(self: Pin<&mut AppController>);

        /// Toggles the slide-out Shelf panel at the screen edge without stealing focus.
        #[qinvokable]
        fn toggle_shelf(self: Pin<&mut AppController>);
        /// Hides the Shelf panel and releases its cached items.
        #[qinvokable]
        fn close_shelf(self: Pin<&mut AppController>);
        /// Recomputes the Shelf items while the Shelf is open.
        #[qinvokable]
        fn refresh_shelf(self: Pin<&mut AppController>);
        /// Applies Win32 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` to the Shelf window.
        #[qinvokable]
        fn style_shelf_window(self: &AppController);
        /// Starts a native OS drag-out operation for a Shelf item (photo, screenshot, clip, or file).
        #[qinvokable]
        fn start_shelf_drag(
            self: &AppController,
            kind: &QString,
            id_or_path: &QString,
            text: &QString,
        ) -> bool;
        /// Sends files, images, links, or text dropped onto the Shelf to the active phone.
        #[qinvokable]
        fn drop_to_shelf(self: &AppController, urls: &QList_QVariant, text: &QString);
        /// Opens, reveals, saves, or copies a Shelf item.
        #[qinvokable]
        fn activate_shelf_item(self: &AppController, kind: &QString, action: &QString, id_or_path: &QString);

        /// Asks a paired device to ring (or stop).
        #[qinvokable]
        fn ring(self: Pin<&mut AppController>, device: &QString, on: bool);
        /// Stops this PC ringing.
        #[qinvokable]
        fn stop_ringing(self: Pin<&mut AppController>);
        /// Allows a paired phone to control this PC's mouse and keyboard (from
        /// the one-time prompt sheet).
        #[qinvokable]
        fn allow_remote_input(self: Pin<&mut AppController>, device: &QString);
        /// Dismisses the one-time remote input prompt without enabling the toggle.
        #[qinvokable]
        fn dismiss_remote_prompt(self: Pin<&mut AppController>);
        /// Reconnects to phones that aren't connected and syncs connected
        /// ones (notifications, media).
        #[qinvokable]
        fn sync_now(self: Pin<&mut AppController>);
        #[qinvokable]
        fn unpair(self: Pin<&mut AppController>, device: &QString);

        /// `{ state, limit, action, target, minutes, reason }` for a feature
        /// of a device (see docs/architecture/capabilities.md).
        #[qinvokable]
        fn feature_state(self: &AppController, device: &QString, feature: &QString) -> QMap_QString_QVariant;
        /// `[{ name, on }]`: what the user allows this device to do.
        #[qinvokable]
        fn device_toggles(self: &AppController, device: &QString) -> QList_QVariant;
        #[qinvokable]
        fn set_device_toggle(self: Pin<&mut AppController>, device: &QString, name: &QString, on: bool);
        /// Latest quick settings toggles for a phone as JSON (`""` when unknown).
        #[qinvokable]
        fn phone_toggles(self: &AppController, device: &QString) -> QString;
        /// Changes one quick settings toggle on a phone (`id`: `"dnd"`,
        /// `"ringer"`, `"flashlight"`, `"volume"`, `"brightness"`, `"wifi"`,
        /// `"bluetooth"`).
        #[qinvokable]
        fn set_phone_toggle(self: Pin<&mut AppController>, device: &QString, id: &QString, value: &QString);

        /// Sends what's copied on this PC to a device.
        #[qinvokable]
        #[cxx_name = "sendClipboard"]
        fn send_clipboard(self: &AppController, device: &QString);
        /// Copies an item from local clipboard history back to the PC clipboard.
        #[qinvokable]
        fn copy_clipboard_history(self: &AppController, id: &QString);
        /// Pins or unpins an item in local clipboard history.
        #[qinvokable]
        fn pin_clipboard_history(self: &AppController, id: &QString, pinned: bool);
        /// Deletes one item from local clipboard history.
        #[qinvokable]
        fn delete_clipboard_history(self: &AppController, id: &QString);
        /// Clears all items from local clipboard history.
        #[qinvokable]
        fn clear_clipboard_history(self: &AppController);
        /// Runs the suggested action for the most recently received clipboard item.
        #[qinvokable]
        fn run_clip_suggestion(self: &AppController);
        /// Hands off a web link, video URL (with timestamp), `geo:` URI, or street address to a phone.
        #[qinvokable]
        fn open_link_on_phone(self: &AppController, device: &QString, text: &QString);
        /// Whether `text` is a web link, video URL, `geo:` URI, or street address that can be handed off.
        #[qinvokable]
        fn is_handoff_text(self: &AppController, text: &QString) -> bool;

        /// Starts a Continuity Camera capture (`"photo"` or `"scan"`) on the active phone.
        #[qinvokable]
        fn start_continuity_camera(self: &AppController, mode: &QString);
        /// Cancels an active Continuity Camera capture request.
        #[qinvokable]
        fn cancel_continuity_camera(self: &AppController);
        /// Pastes the captured Continuity Camera image into the current external window.
        #[qinvokable]
        fn confirm_continuity_paste(self: &AppController);
        /// Dismisses the missing-window paste confirmation prompt, keeping the image on the clipboard.
        #[qinvokable]
        fn dismiss_continuity_paste(self: &AppController);

        /// Opens the Command Palette and populates initial results from local state.
        #[qinvokable]
        fn open_command_palette(self: Pin<&mut AppController>);
        /// Closes the Command Palette.
        #[qinvokable]
        fn close_command_palette(self: Pin<&mut AppController>);
        /// Updates the Command Palette search results for `query`.
        #[qinvokable]
        fn search_command_palette(self: Pin<&mut AppController>, query: &QString);
        /// Executes a Command Palette item by ID and closes the palette.
        #[qinvokable]
        fn run_command_palette(self: Pin<&mut AppController>, id: &QString);
        /// Records when the Command Palette UI finished rendering its first frame.
        #[qinvokable]
        fn note_command_palette_shown(self: Pin<&mut AppController>);

        /// Re-reads whether Windows shows this app's notifications (the user
        /// may have changed it in Settings).
        #[qinvokable]
        #[cxx_name = "refreshToastsEnabled"]
        fn refresh_toasts_enabled(self: Pin<&mut AppController>);

        /// Opens the Connection Doctor and checks the connection.
        #[qinvokable]
        fn run_doctor(self: Pin<&mut AppController>);
        /// Runs one of the Doctor's fixes ("firewall", "network-settings",
        /// "reconnect"), then checks again.
        #[qinvokable]
        fn doctor_fix(self: Pin<&mut AppController>, fix: &QString);

        /// Checks for an update now and says what it found.
        #[qinvokable]
        fn check_for_updates(self: Pin<&mut AppController>);
        /// Downloads and installs the update found.
        #[qinvokable]
        fn install_update(self: Pin<&mut AppController>);

        /// The folder with the app's logs, as a file URL.
        #[qinvokable]
        fn logs_url(self: &AppController) -> QString;

        /// Opens a paired phone's storage sync root in File Explorer.
        #[qinvokable]
        fn open_phone_storage(self: &AppController, device: &QString);
        /// The local sync root folder path for a paired phone.
        #[qinvokable]
        fn phone_storage_path(self: &AppController, device: &QString) -> QString;
        /// Tells the event-driven Home summary which phone the Home page is showing (`""` when hidden).
        #[qinvokable]
        fn watch_home_summary(self: &AppController, device: &QString);
        /// Glanceable Home summary for `device` as JSON.
        #[qinvokable]
        fn home_summary(self: &AppController, device: &QString) -> QString;
        /// Releases idle UI caches and trims the working set when closed to tray.
        #[qinvokable]
        fn trim_working_set(self: &AppController);

        /// Recomputes `data_retention_summary` from local stores and disk caches.
        #[qinvokable]
        fn refresh_data_retention(self: Pin<&mut AppController>);
        /// Clears local notification history (in-memory, on-disk JSON, and cached notification images).
        #[qinvokable]
        fn clear_notification_history(self: Pin<&mut AppController>);
        /// Clears cached SMS/MMS threads, chat threads/messages, and downloaded attachment files.
        #[qinvokable]
        fn clear_message_cache(self: Pin<&mut AppController>);
        /// Clears cached photo thumbnails and viewer files on disk (without touching phone photos).
        #[qinvokable]
        fn clear_photo_thumbnails(self: Pin<&mut AppController>);
        /// Clears received file transfer history records (without deleting saved files in Downloads).
        #[qinvokable]
        fn clear_received_file_history(self: Pin<&mut AppController>);
        /// Clears all local histories and caches across all 6 categories while keeping paired devices and settings intact.
        #[qinvokable]
        fn clear_everything(self: Pin<&mut AppController>);

        /// A short message for the user (e.g. a command failed).
        #[qsignal]
        fn toast(self: Pin<&mut AppController>, message: QString);
        /// A short message with a context action chip (e.g. a received clip).
        #[qsignal]
        fn toast_with_action(self: Pin<&mut AppController>, message: QString, action_label: QString);
        /// Show the main window (tray click, second launch).
        #[qsignal]
        fn activate_requested(self: Pin<&mut AppController>);
        /// The user chose Quit in the tray.
        #[qsignal]
        fn quit_requested(self: Pin<&mut AppController>);
        /// Show the Connection Doctor.
        #[qsignal]
        fn doctor_requested(self: Pin<&mut AppController>);
        /// Open the Handoff document picker on the Home page.
        #[qsignal]
        fn handoff_file_picker_requested(self: Pin<&mut AppController>);
    }

    impl cxx_qt::Threading for AppController {}
    impl cxx_qt::Initialize for AppController {}
}

#[derive(Default)]
pub struct AppControllerRust {
    status: QString,
    error: QString,
    device_name: QString,
    device_id: QString,
    has_devices: bool,
    caps_revision: i32,
    toggles_revision: i32,
    phone_toggles_revision: i32,
    ringing_from: QString,
    system_dark: bool,
    reduce_motion: bool,
    wallpaper_colors: QString,
    phone_colors: QString,
    toasts_enabled: bool,
    version: QString,
    doctor_checks: QString,
    doctor_busy: bool,
    can_update: bool,
    update_version: QString,
    update_busy: bool,
    remote_prompt_device_id: QString,
    remote_prompt_device_name: QString,
    laser_active: bool,
    laser_x: f32,
    laser_y: f32,
    wake_adapter: QString,
    wake_state: QString,
    wake_wired: bool,
    current_page: QString,
    current_device: i32,
    pending_dial: QString,
    clipboard_history: QString,
    timeline_preview: QString,
    home_summary_revision: i32,
    continuity_camera_busy: bool,
    continuity_camera_status: QString,
    continuity_camera_mode: QString,
    continuity_paste_prompt_visible: bool,
    continuity_paste_prompt_title: QString,
    continuity_paste_prompt_body: QString,
    continuity_paste_target_name: QString,
    command_palette_open: bool,
    command_palette_results: QString,
    command_palette_open_ms: f64,
    data_retention_summary: QString,
    localsend_enabled: bool,
    localsend_receiving: bool,
    localsend_peers_json: QString,
    shelf_open: bool,
    shelf_items: QString,
    tray: Option<tray::Tray>,
}

/// The controller's thread handle, so other threads (second launch) can
/// reach it.
static CONTROLLER: OnceLock<Mutex<Option<CxxQtThread<qobject::AppController>>>> = OnceLock::new();
static SHELF_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the slide-out Shelf panel is currently open.
pub fn is_shelf_open() -> bool {
    SHELF_OPEN.load(Ordering::Relaxed)
}

/// Toggles the slide-out Shelf panel from any thread (Command Palette, tray, or hotkey).
pub fn toggle_shelf_from_anywhere() {
    if let Some(qt) = controller() {
        let _ = qt.queue(|object| object.toggle_shelf());
    }
}

/// Downloads and starts the update (from the app or its notification).
pub fn install_update_in_background() {
    let Some(qt) = controller() else { return };
    let started = qt.queue(|mut object| {
        if object.update_busy {
            return;
        }
        object.as_mut().set_update_busy(true);
        object.as_mut().toast(QString::from("Downloading the update…"));
        let qt = object.qt_thread();
        std::thread::spawn(move || {
            let result = crate::updater::install();
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_update_busy(false);
                if let Err(message) = result {
                    object.toast(QString::from(&message));
                }
            });
        });
    });
    if started.is_err() {
        tracing::warn!("can't start the update");
    }
}

/// Runs a Doctor fix (if any), then the checks, off the UI thread; the
/// findings land in `doctor_checks`.
fn diagnose_in_background(mut object: Pin<&mut qobject::AppController>, fix: Option<String>) {
    if object.doctor_busy {
        return;
    }
    object.as_mut().set_doctor_busy(true);
    let qt = object.qt_thread();
    let spawned = std::thread::Builder::new().name("doctor".into()).spawn(move || {
        if let Some(fix) = fix {
            crate::doctor::fix(&fix);
            // A reconnect needs a moment to show.
            if fix == "reconnect" {
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let checks = crate::doctor::diagnose(&crate::doctor::gather(), now);
        let json = serde_json::to_string(&checks).unwrap_or_else(|_| "[]".into());
        let _ = qt.queue(move |mut object| {
            object.as_mut().set_doctor_checks(QString::from(&json));
            object.set_doctor_busy(false);
        });
    });
    if spawned.is_err() {
        object.set_doctor_busy(false);
    }
}

fn controller() -> Option<CxxQtThread<qobject::AppController>> {
    CONTROLLER.get()?.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Asks the app to quit (from any thread), as Quit in the tray does.
pub fn request_quit() {
    if let Some(qt) = controller() {
        let _ = qt.queue(|object| object.quit_requested());
    }
}

/// Asks the UI to show the main window (from any thread).
pub fn request_activation() {
    if let Some(qt) = controller() {
        let _ = qt.queue(|object| object.activate_requested());
    }
}

/// Shows a short message in the app (from any thread).
pub(crate) fn show_message(message: impl Into<String>) {
    let message = message.into();
    if let Some(qt) = controller() {
        let _ = qt.queue(move |object| object.toast(QString::from(&message)));
    }
}

/// Shows a short message with a context action chip in the app (from any thread).
pub(crate) fn show_message_with_action(message: impl Into<String>, action_label: impl Into<String>) {
    let message = message.into();
    let action_label = action_label.into();
    if let Some(qt) = controller() {
        let _ = qt.queue(move |object| {
            object.toast_with_action(QString::from(&message), QString::from(&action_label));
        });
    }
}

/// Opens the Calls page dialer with `number` filled in (without placing the call).
pub(crate) fn open_dialer(device: DeviceId, number: &str) {
    let number = number.trim().to_owned();
    if number.is_empty() {
        return;
    }
    let idx = core_host::host().hub.read(|s| {
        s.devices
            .iter()
            .position(|d| d.id == device && matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.iter().position(|d| matches!(d.link, LinkState::Online { .. })))
            .or_else(|| s.devices.iter().position(|d| d.id == device))
    });
    if let Some(qt) = controller() {
        let _ = qt.queue(move |mut object| {
            if let Some(idx) = idx {
                object.as_mut().set_current_device(idx as i32);
            }
            object.as_mut().set_pending_dial(QString::from(&number));
            object.as_mut().set_current_page(QString::from("calls"));
            object.activate_requested();
        });
    }
}

/// Explains a failed command in a sentence the user understands.
pub(crate) fn describe(error: &Error) -> String {
    match error {
        Error::Offline => "The device isn't connected right now.".into(),
        Error::NotPaired => "That device isn't paired anymore.".into(),
        Error::Timeout => "The device didn't answer in time.".into(),
        Error::Denied | Error::Declined => "The device declined.".into(),
        Error::Unsupported => "The device's app doesn't support that yet.".into(),
        other => format!("Something went wrong: {other}"),
    }
}

impl cxx_qt::Initialize for qobject::AppController {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_version(QString::from(env!("CARGO_PKG_VERSION")));
        self.as_mut().set_current_page(QString::from("home"));
        self.as_mut().set_can_update(crate::updater::can_update());
        self.as_mut().set_clipboard_history(QString::from(&crate::clipboard::history_json()));
        self.as_mut().set_timeline_preview(QString::from(&super::timeline::preview_json()));
        self.as_mut().set_command_palette_results(QString::from("[]"));
        self.as_mut().set_data_retention_summary(QString::from(&compute_data_retention_json()));
        self.as_mut().set_localsend_peers_json(QString::from("[]"));
        self.as_mut().set_shelf_items(QString::default());
        let initial_phone_colors = core_host::host().hub.read(|s| phone_colors_json(&s.devices));
        self.as_mut().set_phone_colors(QString::from(&initial_phone_colors));
        self.as_mut().refresh_appearance();
        self.as_mut().refresh_localsend_state();
        let qt = self.qt_thread();
        *CONTROLLER.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner()) = Some(qt.clone());
        super::subscribe(
            qt.clone(),
            Changes::STATUS
                | Changes::DEVICES
                | Changes::CAPABILITIES
                | Changes::RINGING
                | Changes::REMOTE
                | Changes::TOGGLES,
            Self::refresh,
        );
        super::subscribe(qt.clone(), Changes::UPDATE, |object| {
            let version = crate::updater::available().map(|u| u.version).unwrap_or_default();
            object.set_update_version(QString::from(&version));
        });
        super::subscribe(qt.clone(), Changes::CLIPBOARD, |mut object| {
            let json = crate::clipboard::history_json();
            object.as_mut().set_clipboard_history(QString::from(&json));
            let preview = super::timeline::preview_json();
            object.as_mut().set_timeline_preview(QString::from(&preview));
            if object.shelf_open {
                let shelf = compute_shelf_items_json(object.current_device);
                object.set_shelf_items(QString::from(&shelf));
            }
        });
        super::subscribe(qt.clone(), Changes::TIMELINE | Changes::DEVICES | Changes::STATUS, |mut object| {
            let preview = super::timeline::preview_json();
            object.as_mut().set_timeline_preview(QString::from(&preview));
            if object.shelf_open {
                let shelf = compute_shelf_items_json(object.current_device);
                object.set_shelf_items(QString::from(&shelf));
            }
        });
        super::subscribe(qt.clone(), Changes::TRANSFERS | Changes::PHOTOS, |mut object| {
            if object.shelf_open {
                let shelf = compute_shelf_items_json(object.current_device);
                object.as_mut().set_shelf_items(QString::from(&shelf));
            }
        });
        super::subscribe(qt.clone(), Changes::CONTINUITY, Self::refresh_continuity);
        super::subscribe(qt.clone(), Changes::PALETTE, Self::refresh_palette);
        super::subscribe(qt.clone(), Changes::LOCALSEND | Changes::STATUS, Self::refresh_localsend_state);
        super::subscribe(
            qt.clone(),
            Changes::DATA_RETENTION
                | Changes::CLIPBOARD
                | Changes::TIMELINE
                | Changes::HISTORY
                | Changes::TRANSFERS
                | Changes::MESSAGES
                | Changes::PHOTOS,
            |object| {
                let summary = compute_data_retention_json();
                object.set_data_retention_summary(QString::from(&summary));
            },
        );

        let labels = tray::MenuLabels {
            open: "Open Nectarlink".into(),
            command_palette: "Command palette".into(),
            shelf: "Toggle Shelf".into(),
            take_photo: "Take photo with phone".into(),
            scan_document: "Scan document with phone".into(),
            find_phone: "Find my phone".into(),
            open_link: "Open copied link on phone".into(),
            quit: "Quit Nectarlink".into(),
        };
        match tray::Tray::create("Nectarlink", labels, move |event| on_tray_event(&qt, event)) {
            Ok(tray) => self.rust_mut().tray = Some(tray),
            Err(e) => tracing::warn!(error = %e, "no tray icon"),
        }
    }
}

fn on_tray_event(qt: &CxxQtThread<qobject::AppController>, event: tray::TrayEvent) {
    match event {
        tray::TrayEvent::Open => {
            let _ = qt.queue(|object| object.activate_requested());
        }
        tray::TrayEvent::CommandPalette => {
            let results = crate::command_palette::open();
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_command_palette_results(QString::from(&results));
                object.as_mut().set_command_palette_open(true);
                object.activate_requested();
            });
        }
        tray::TrayEvent::Shelf => {
            let _ = qt.queue(|object| object.toggle_shelf());
        }
        tray::TrayEvent::TakePhoto => crate::continuity_camera::start("photo"),
        tray::TrayEvent::ScanDocument => crate::continuity_camera::start("scan"),
        tray::TrayEvent::OpenLinkOnPhone => crate::links::open_copied_link_on_phone(),
        tray::TrayEvent::Quit => {
            let _ = qt.queue(|object| object.quit_requested());
        }
        tray::TrayEvent::FindPhone => {
            let phone = core_host::host().hub.read(|s| {
                s.devices
                    .iter()
                    .find(|d| matches!(d.link, LinkState::Online { .. }))
                    .or(s.devices.first())
                    .map(|d| d.id)
            });
            if let Some(phone) = phone {
                ring_device(phone, true);
            }
        }
        tray::TrayEvent::Resumed => {
            tracing::info!("resumed from sleep; reconnecting");
            if let Some(node) = core_host::node() {
                core_host::spawn(async move { node.network_changed().await });
            }
        }
        tray::TrayEvent::AppearanceChanged => {
            let _ = qt.queue(|object| object.refresh_appearance());
        }
    }
}

/// Bumped per wallpaper refresh, so a slow, older one can't overwrite a
/// newer result.
static WALLPAPER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Works out the wallpaper's colors on a worker thread (decoding a photo
/// takes a moment) and hands them to QML.
fn refresh_wallpaper_colors(qt: CxxQtThread<qobject::AppController>) {
    let generation = WALLPAPER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new().name("wallpaper-colors".into()).spawn(move || {
        let seed = win::wallpaper::seed();
        tracing::debug!(seed = ?seed.map(|s| format!("#{:06X}", s.as_u32())), "wallpaper colors");
        let json = seed.map(|seed| crate::palette::scheme_json(seed).to_string());
        if WALLPAPER_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let _ = qt.queue(move |object| {
            object.set_wallpaper_colors(QString::from(json.as_deref().unwrap_or_default()));
        });
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't start reading the wallpaper's colors");
    }
}

fn phone_colors_json(devices: &[crate::state::DeviceView]) -> String {
    if devices.is_empty() {
        return serde_json::json!([{
            "id": "",
            "key": "phone",
            "name": "",
            "label": "My phone's colors",
            "hasAccent": false,
            "scheme": serde_json::Value::Null,
        }])
        .to_string();
    }
    let multiple = devices.len() > 1;
    let entries: Vec<serde_json::Value> = devices
        .iter()
        .map(|d| {
            let scheme = d.info.accent.map(|argb| {
                let r = ((argb >> 16) & 0xFF) as u8;
                let g = ((argb >> 8) & 0xFF) as u8;
                let b = (argb & 0xFF) as u8;
                crate::palette::scheme_json(material_colors::color::Rgb::new(r, g, b))
            });
            let id_str = d.id.to_string();
            let key = if multiple { format!("phone:{id_str}") } else { "phone".to_owned() };
            let label = if multiple {
                format!("My phone's colors ({})", d.info.name)
            } else {
                "My phone's colors".to_owned()
            };
            serde_json::json!({
                "id": id_str,
                "key": key,
                "name": d.info.name,
                "label": label,
                "hasAccent": scheme.is_some(),
                "scheme": scheme,
            })
        })
        .collect();
    serde_json::Value::Array(entries).to_string()
}

fn ring_device(device: DeviceId, on: bool) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.ring(device, on).await {
            show_message(describe(&e));
        }
    });
}

impl qobject::AppController {
    fn refresh(mut self: Pin<&mut Self>) {
        let hub = &core_host::host().hub;
        let (
            status,
            devices,
            phone_colors,
            ringing,
            online,
            caps_version,
            toggles_version,
            battery,
            remote_prompt,
            laser,
            wake_adapter,
            wake_state,
            wake_wired,
        ) = hub.read(|s| {
            let ringing = s.ringing_from.and_then(|id| s.name_of(&id)).unwrap_or_default();
            let remote_prompt = s.remote_prompt.map(|id| {
                let name = s.name_of(&id).unwrap_or_else(|| id.short());
                (id.to_string(), name)
            });
            let connected: Vec<_> =
                s.devices.iter().filter(|d| matches!(d.link, LinkState::Online { .. })).collect();
            // With one phone connected, the tray shows its battery.
            let battery = match connected.as_slice() {
                [one] => one.battery.as_ref().map(|b| {
                    let charging = if b.charging { ", charging" } else { "" };
                    format!("{} · {}%{charging}", one.info.name, b.level)
                }),
                _ => None,
            };
            let wake_adapter = s.wake.primary().map(|a| a.label.clone()).unwrap_or_default();
            let wake_state = s.wake.state_str();
            let wake_wired = s.wake.primary().is_some_and(|a| a.wired);
            let phone_colors = phone_colors_json(&s.devices);
            (
                s.core_status(),
                s.devices.len(),
                phone_colors,
                ringing,
                connected.len(),
                s.matrices_version,
                s.toggles_version,
                battery,
                remote_prompt,
                s.laser,
                wake_adapter,
                wake_state,
                wake_wired,
            )
        });
        let (status_text, error) = match &status {
            CoreStatus::Starting => ("starting", String::new()),
            CoreStatus::Ready { device_id, name } => {
                self.as_mut().set_device_name(QString::from(name));
                self.as_mut().set_device_id(QString::from(&device_id.to_string()));
                ("ready", String::new())
            }
            CoreStatus::Failed(message) => ("failed", message.clone()),
        };
        self.as_mut().set_status(QString::from(status_text));
        self.as_mut().set_error(QString::from(&error));
        self.as_mut().set_has_devices(devices > 0);
        self.as_mut().set_phone_colors(QString::from(&phone_colors));
        self.as_mut().set_ringing_from(QString::from(&ringing));
        let (prompt_id, prompt_name) = remote_prompt.unwrap_or_default();
        self.as_mut().set_remote_prompt_device_id(QString::from(&prompt_id));
        self.as_mut().set_remote_prompt_device_name(QString::from(&prompt_name));
        match laser {
            Some((_, x, y)) => {
                self.as_mut().set_laser_x(x);
                self.as_mut().set_laser_y(y);
                self.as_mut().set_laser_active(true);
            }
            None => {
                self.as_mut().set_laser_active(false);
            }
        }
        self.as_mut().set_wake_adapter(QString::from(&wake_adapter));
        self.as_mut().set_wake_state(QString::from(wake_state));
        self.as_mut().set_wake_wired(wake_wired);
        // Truncation is fine: QML only compares revisions for equality.
        self.as_mut().set_caps_revision(caps_version as i32);
        self.as_mut().set_phone_toggles_revision(toggles_version as i32);

        if let Some(tray) = self.rust().tray.as_ref() {
            tray.set_find_phone_enabled(devices > 0);
            tray.set_tooltip(&match (devices, online) {
                (0, _) => "Nectarlink · no devices paired".to_owned(),
                (_, 0) => "Nectarlink · not connected".to_owned(),
                (_, 1) => battery.map_or_else(
                    || "Nectarlink · 1 device connected".to_owned(),
                    |b| format!("Nectarlink · {b}"),
                ),
                (_, n) => format!("Nectarlink · {n} devices connected"),
            });
        }
    }

    fn refresh_continuity(mut self: Pin<&mut Self>) {
        let view = crate::continuity_camera::view();
        self.as_mut().set_continuity_camera_busy(view.busy);
        self.as_mut().set_continuity_camera_mode(QString::from(&view.mode));
        self.as_mut().set_continuity_camera_status(QString::from(&view.status));
        self.as_mut().set_continuity_paste_prompt_visible(view.paste_prompt_visible);
        self.as_mut().set_continuity_paste_prompt_title(QString::from(&view.paste_prompt_title));
        self.as_mut().set_continuity_paste_prompt_body(QString::from(&view.paste_prompt_body));
        self.as_mut().set_continuity_paste_target_name(QString::from(&view.paste_target_name));
    }

    fn refresh_palette(mut self: Pin<&mut Self>) {
        let was_open = self.command_palette_open;
        let is_open = crate::command_palette::is_open();
        self.as_mut().set_command_palette_open(is_open);
        self.as_mut().set_command_palette_open_ms(crate::command_palette::last_open_ms());
        if is_open && !was_open {
            let results = crate::command_palette::search("");
            self.as_mut().set_command_palette_results(QString::from(&results));
        }
    }

    fn refresh_localsend_state(mut self: Pin<&mut Self>) {
        let (enabled, receiving, peers) = match core_host::node() {
            Some(node) => (node.localsend_enabled(), node.localsend_receiving(), node.localsend_peers()),
            None => (false, false, Vec::new()),
        };
        core_host::host().hub.update(|s| {
            s.localsend_enabled = enabled;
            s.localsend_receiving = receiving;
            s.localsend_peers = peers.clone();
            Changes::NONE
        });
        let entries: Vec<serde_json::Value> = peers
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id.to_string(),
                    "alias": p.alias,
                    "deviceModel": p.device_model.unwrap_or_default(),
                    "deviceType": p.device_type,
                    "fingerprint": p.fingerprint,
                    "ip": p.ip,
                    "port": p.port,
                    "protocol": p.protocol,
                })
            })
            .collect();
        let json = serde_json::Value::Array(entries).to_string();
        self.as_mut().set_localsend_enabled(enabled);
        self.as_mut().set_localsend_receiving(receiving);
        self.as_mut().set_localsend_peers_json(QString::from(&json));
    }

    pub fn toggle_localsend(mut self: Pin<&mut Self>, enabled: bool) {
        if let Some(node) = core_host::node()
            && let Err(e) = node.set_localsend_enabled(enabled)
        {
            show_message(describe(&e));
        }
        self.as_mut().refresh_localsend_state();
        core_host::host().hub.changed(Changes::LOCALSEND);
    }

    pub fn refresh_localsend(mut self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            node.refresh_localsend();
        }
        self.as_mut().refresh_localsend_state();
    }

    fn refresh_appearance(mut self: Pin<&mut Self>) {
        self.as_mut().set_system_dark(win::system_dark());
        self.as_mut().set_reduce_motion(win::reduce_motion());
        refresh_wallpaper_colors(self.qt_thread());
        self.refresh_toasts_enabled();
    }

    pub fn send_clipboard(&self, device: &QString) {
        if let Some(id) = super::parse_device(device) {
            crate::clipboard::send_now(id);
        }
    }

    pub fn copy_clipboard_history(&self, id: &QString) {
        crate::clipboard::copy_history_item(&String::from(id));
    }

    pub fn pin_clipboard_history(&self, id: &QString, pinned: bool) {
        crate::clipboard::pin_history_item(&String::from(id), pinned);
    }

    pub fn delete_clipboard_history(&self, id: &QString) {
        crate::clipboard::delete_history_item(&String::from(id));
    }

    pub fn clear_clipboard_history(&self) {
        crate::clipboard::clear_history();
    }

    pub fn run_clip_suggestion(&self) {
        crate::clipboard::run_last_suggestion();
    }

    pub fn open_link_on_phone(&self, device: &QString, text: &QString) {
        if let Some(id) = super::parse_device(device) {
            crate::links::send_to(id, String::from(text));
        }
    }

    pub fn is_handoff_text(&self, text: &QString) -> bool {
        crate::links::is_handoff_text(&String::from(text))
    }

    pub fn start_continuity_camera(&self, mode: &QString) {
        let idx = self.current_device.max(0) as usize;
        crate::continuity_camera::start_for_index(idx, &String::from(mode));
    }

    pub fn cancel_continuity_camera(&self) {
        crate::continuity_camera::cancel();
    }

    pub fn confirm_continuity_paste(&self) {
        crate::continuity_camera::confirm_paste();
    }

    pub fn dismiss_continuity_paste(&self) {
        crate::continuity_camera::dismiss_paste();
    }

    pub fn open_command_palette(mut self: Pin<&mut Self>) {
        let results = crate::command_palette::open();
        self.as_mut().set_command_palette_results(QString::from(&results));
        self.as_mut().set_command_palette_open(true);
        self.activate_requested();
    }

    pub fn close_command_palette(self: Pin<&mut Self>) {
        crate::command_palette::close();
        self.set_command_palette_open(false);
    }

    pub fn search_command_palette(self: Pin<&mut Self>, query: &QString) {
        let results = crate::command_palette::search(&String::from(query));
        self.set_command_palette_results(QString::from(&results));
    }

    pub fn note_command_palette_shown(self: Pin<&mut Self>) {
        let ms = crate::command_palette::mark_shown();
        self.set_command_palette_open_ms(ms);
    }

    pub fn run_command_palette(mut self: Pin<&mut Self>, id: &QString) {
        let action = crate::command_palette::execute(&String::from(id));
        self.as_mut().set_command_palette_open(false);
        let Some(action) = action else { return };
        use crate::command_palette::PaletteUiAction;
        match action {
            PaletteUiAction::NavigatePage { page, device_index } => {
                if let Some(idx) = device_index {
                    self.as_mut().set_current_device(idx as i32);
                }
                self.as_mut().set_current_page(QString::from(&page));
                self.activate_requested();
            }
            PaletteUiAction::StartChat { device_id, number, name } => {
                if let Ok(dev) = device_id.parse::<DeviceId>() {
                    if let Some(idx) =
                        core_host::host().hub.read(|s| s.devices.iter().position(|d| d.id == dev))
                    {
                        self.as_mut().set_current_device(idx as i32);
                    }
                    crate::messages::open_device(dev);
                    crate::messages::start_chat(dev, number, name);
                }
                self.as_mut().set_current_page(QString::from("messages"));
                self.activate_requested();
            }
            PaletteUiAction::OpenDialer { device_id, number } => {
                if let Ok(dev) = device_id.parse::<DeviceId>() {
                    open_dialer(dev, &number);
                }
            }
            PaletteUiAction::OpenHandoffFilePicker { device_index } => {
                if let Some(idx) = device_index {
                    self.as_mut().set_current_device(idx as i32);
                }
                self.as_mut().set_current_page(QString::from("home"));
                self.as_mut().activate_requested();
                self.handoff_file_picker_requested();
            }
            PaletteUiAction::OpenDoctor => {
                self.run_doctor();
            }
            PaletteUiAction::OpenPairing => {
                self.as_mut().set_current_page(QString::from("home"));
                self.activate_requested();
            }
        }
    }

    pub fn refresh_toasts_enabled(self: Pin<&mut Self>) {
        self.set_toasts_enabled(win::toast::enabled());
        core_host::spawn(core_host::refresh_wake());
    }

    pub fn ring(self: Pin<&mut Self>, device: &QString, on: bool) {
        if let Some(id) = super::parse_device(device) {
            ring_device(id, on);
        }
    }

    pub fn sync_now(self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.refresh().await });
        }
    }

    pub fn stop_ringing(self: Pin<&mut Self>) {
        sound::stop_ringing();
        core_host::host()
            .hub
            .update(|s| if s.ringing_from.take().is_some() { Changes::RINGING } else { Changes::NONE });
    }

    pub fn allow_remote_input(mut self: Pin<&mut Self>, device: &QString) {
        self.as_mut().set_device_toggle(device, &QString::from("remote_input"), true);
        core_host::host()
            .hub
            .update(|s| if s.remote_prompt.take().is_some() { Changes::REMOTE } else { Changes::NONE });
    }

    pub fn dismiss_remote_prompt(self: Pin<&mut Self>) {
        core_host::host()
            .hub
            .update(|s| if s.remote_prompt.take().is_some() { Changes::REMOTE } else { Changes::NONE });
    }

    pub fn unpair(self: Pin<&mut Self>, device: &QString) {
        let (Some(id), Some(node)) = (super::parse_device(device), core_host::node()) else { return };
        core_host::spawn(async move {
            if let Err(e) = node.unpair(id).await {
                show_message(describe(&e));
            }
        });
    }

    pub fn feature_state(&self, device: &QString, feature: &QString) -> QMap<QMapPair_QString_QVariant> {
        let feature = String::from(feature);
        let state = super::parse_device(device).and_then(|id| {
            core_host::host().hub.read(|s| s.matrices.get(&id).and_then(|m| m.state(&feature)))
        });
        feature_state_map(state)
    }

    pub fn device_toggles(&self, device: &QString) -> QList<QVariant> {
        let mut list = QList::default();
        let (Some(id), Some(node)) = (super::parse_device(device), core_host::node()) else { return list };
        for (name, on) in node.device_toggles(id).unwrap_or_default() {
            let mut item = QHash::<QHashPair_QString_QVariant>::default();
            item.insert(QString::from("name"), QVariant::from(&QString::from(name)));
            item.insert(QString::from("on"), QVariant::from(&on));
            list.append(QVariant::from(&item));
        }
        list
    }

    pub fn set_device_toggle(mut self: Pin<&mut Self>, device: &QString, name: &QString, on: bool) {
        let (Some(id), Some(node)) = (super::parse_device(device), core_host::node()) else { return };
        let name_str = String::from(name);
        if let Err(e) = node.set_device_toggle(id, &name_str, on) {
            show_message(describe(&e));
        }
        if name_str == "remote_input" {
            if !on {
                crate::remote::stop_all();
            }
            core_host::host().hub.update(|s| {
                if s.remote_prompt == Some(id) {
                    s.remote_prompt = None;
                    Changes::REMOTE
                } else {
                    Changes::NONE
                }
            });
        }
        if name_str == "storage" {
            crate::storage::sync();
        }
        let revision = self.toggles_revision.wrapping_add(1);
        self.as_mut().set_toggles_revision(revision);
    }

    pub fn phone_toggles(&self, device: &QString) -> QString {
        let Some(id) = super::parse_device(device) else { return QString::default() };
        let json = core_host::host()
            .hub
            .read(|s| s.toggles.get(&id).and_then(|t| serde_json::to_string(t).ok()))
            .unwrap_or_default();
        QString::from(&json)
    }

    pub fn set_phone_toggle(self: Pin<&mut Self>, device: &QString, id: &QString, value: &QString) {
        let (Some(device), Some(node)) = (super::parse_device(device), core_host::node()) else { return };
        let id = String::from(id);
        let raw = String::from(value);
        let parsed = match id.as_str() {
            "dnd" | "flashlight" | "wifi" | "bluetooth" => Some(PhoneToggleValue::Bool(raw == "true")),
            "volume" | "brightness" => raw.parse::<u8>().ok().map(|v| PhoneToggleValue::Level(v.min(100))),
            "ringer" => Some(PhoneToggleValue::Mode(raw)),
            _ => None,
        };
        let Some(val) = parsed else { return };
        // Optimistically update local state so buttons and sliders don't snap back while the RPC runs.
        core_host::host().hub.update(|s| {
            let Some(t) = s.toggles.get(&device).cloned() else { return Changes::NONE };
            let mut next = t;
            match (id.as_str(), &val) {
                ("dnd", PhoneToggleValue::Bool(on)) => next.dnd = *on,
                ("flashlight", PhoneToggleValue::Bool(on)) => next.flashlight = Some(*on),
                ("wifi", PhoneToggleValue::Bool(on)) => next.wifi = *on,
                ("bluetooth", PhoneToggleValue::Bool(on)) => next.bluetooth = *on,
                ("volume", PhoneToggleValue::Level(v)) => next.volume = *v,
                ("brightness", PhoneToggleValue::Level(v)) => next.brightness = *v,
                ("ringer", PhoneToggleValue::Mode(m)) => next.ringer.clone_from(m),
                _ => {}
            }
            s.set_toggles(device, Some(next))
        });
        core_host::spawn(async move {
            if let Err(e) = node.set_phone_toggle(device, id, val).await {
                let latest = node.phone_toggles(device);
                core_host::host().hub.update(|s| s.set_toggles(device, latest));
                show_message(describe(&e));
            }
        });
    }

    pub fn check_for_updates(mut self: Pin<&mut Self>) {
        if self.update_busy {
            return;
        }
        self.as_mut().set_update_busy(true);
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let message = crate::updater::check_now(true);
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_update_busy(false);
                if let Some(message) = message {
                    object.toast(QString::from(&message));
                }
            });
        });
    }

    pub fn install_update(self: Pin<&mut Self>) {
        install_update_in_background();
    }

    pub fn run_doctor(mut self: Pin<&mut Self>) {
        self.as_mut().doctor_requested();
        diagnose_in_background(self, None);
    }

    pub fn doctor_fix(self: Pin<&mut Self>, fix: &QString) {
        diagnose_in_background(self, Some(String::from(fix)));
    }

    pub fn logs_url(&self) -> QString {
        let dir = crate::logging::logs_dir(&core_host::host().data_dir);
        QString::from(&format!("file:///{}", dir.to_string_lossy().replace('\\', "/")))
    }

    pub fn open_phone_storage(&self, device: &QString) {
        let Some(id) = super::parse_device(device) else { return };
        if let Err(e) = crate::storage::open_in_explorer(id) {
            show_message(e);
        }
    }

    pub fn phone_storage_path(&self, device: &QString) -> QString {
        let Some(id) = super::parse_device(device) else { return QString::default() };
        let path =
            crate::storage::sync_root_path_for(id).map(|p| p.display().to_string()).unwrap_or_default();
        QString::from(&path)
    }

    pub fn watch_home_summary(&self, device: &QString) {
        watch_home(super::parse_device(device));
    }

    pub fn home_summary(&self, device: &QString) -> QString {
        let Some(id) = super::parse_device(device) else { return QString::default() };
        QString::from(&home_summary_json(id))
    }

    pub fn trim_working_set(&self) {
        release_home_summary();
        crate::webcam::release_idle_resources();
        crate::photos::release_idle_resources();
        crate::messages::release_idle_resources();
        crate::calls::release_idle_resources();
        super::native::ffi::trim_memory_caches();
    }

    pub fn refresh_data_retention(self: Pin<&mut Self>) {
        let summary = compute_data_retention_json();
        self.set_data_retention_summary(QString::from(&summary));
    }

    pub fn clear_notification_history(self: Pin<&mut Self>) {
        core_host::host().hub.update(|s| s.clear_notification_history() | Changes::DATA_RETENTION);
        crate::notification_store::clear_disk();
        self.refresh_data_retention();
    }

    pub fn clear_message_cache(self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            let _ = node.clear_message_cache();
        }
        crate::messages::clear_cache();
        core_host::host().hub.changed(Changes::DATA_RETENTION);
        self.refresh_data_retention();
    }

    pub fn clear_photo_thumbnails(self: Pin<&mut Self>) {
        crate::photos::clear_cache();
        core_host::host().hub.changed(Changes::DATA_RETENTION);
        self.refresh_data_retention();
    }

    pub fn clear_received_file_history(self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            let _ = node.clear_received_file_history();
        }
        core_host::host()
            .hub
            .update(|s| s.clear_transfer_history() | Changes::TIMELINE | Changes::DATA_RETENTION);
        self.refresh_data_retention();
    }

    pub fn clear_everything(mut self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            let _ = node.clear_all_local_data();
        }
        crate::clipboard::clear_history();
        core_host::host().hub.update(|s| {
            s.clear_notification_history()
                | s.clear_transfer_history()
                | Changes::CLIPBOARD
                | Changes::TIMELINE
                | Changes::DATA_RETENTION
        });
        crate::notification_store::clear_disk();
        crate::messages::clear_cache();
        crate::photos::clear_cache();
        self.as_mut().refresh_data_retention();
        self.toast(QString::from("Cleared all local history and caches"));
    }

    pub fn toggle_shelf(mut self: Pin<&mut Self>) {
        if self.shelf_open {
            self.close_shelf();
        } else {
            SHELF_OPEN.store(true, Ordering::Relaxed);
            let idx = self.current_device;
            let json = compute_shelf_items_json(idx);
            self.as_mut().set_shelf_items(QString::from(&json));
            self.as_mut().set_shelf_open(true);
            win::tray::apply_no_activate_style_by_title("Nectarlink Shelf");
        }
    }

    pub fn close_shelf(mut self: Pin<&mut Self>) {
        SHELF_OPEN.store(false, Ordering::Relaxed);
        self.as_mut().set_shelf_open(false);
        self.as_mut().set_shelf_items(QString::default());
    }

    pub fn refresh_shelf(mut self: Pin<&mut Self>) {
        if !self.shelf_open {
            return;
        }
        if let (Some(dev), _, true) = resolve_shelf_device(self.current_device) {
            refresh_shelf_photo_if_needed(dev, true);
        }
        let json = compute_shelf_items_json(self.current_device);
        self.as_mut().set_shelf_items(QString::from(&json));
    }

    pub fn style_shelf_window(&self) {
        win::tray::apply_no_activate_style_by_title("Nectarlink Shelf");
    }

    pub fn start_shelf_drag(&self, kind: &QString, id_or_path: &QString, text: &QString) -> bool {
        let kind_str = String::from(kind);
        let s = String::from(id_or_path).trim().to_owned();
        let t = String::from(text);

        if !s.is_empty() && std::path::Path::new(&s).exists() {
            return super::native::ffi::start_external_drag(&s, &t);
        }
        if (kind_str == "photo" || kind_str == "screenshot")
            && !s.is_empty()
            && let (Some(dev), _, _) = resolve_shelf_device(self.current_device)
        {
            let thumb = crate::photos::thumb_path(dev, &s);
            if let Some(dir) = thumb.parent() {
                let prefix = format!("full-{:016x}.", crate::photos::fingerprint(&s));
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix))
                            && p.exists()
                        {
                            return super::native::ffi::start_external_drag(&p.to_string_lossy(), &t);
                        }
                    }
                }
            }
            if thumb.exists() {
                return super::native::ffi::start_external_drag(&thumb.to_string_lossy(), &t);
            }
        }
        if kind_str == "clip"
            && !s.is_empty()
            && t.is_empty()
            && let Some(node) = core_host::node()
            && let Some((_, bytes)) = node.clipboard_history_image(&s)
        {
            let tmp = core_host::host().data_dir.join("cache").join("shelf-clip.png");
            if let Some(parent) = tmp.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&tmp, &bytes).is_ok() {
                return super::native::ffi::start_external_drag(&tmp.to_string_lossy(), "");
            }
        }
        super::native::ffi::start_external_drag("", &t)
    }

    pub fn drop_to_shelf(&self, urls: &QList<QVariant>, text: &QString) {
        let (Some(device), device_name, _) = resolve_shelf_device(self.current_device) else {
            show_message("No paired phone is connected.");
            return;
        };

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        let mut url_text_fallback = String::new();
        for v in urls.iter() {
            let raw = v
                .value::<QString>()
                .map(|qs| String::from(&qs))
                .or_else(|| v.value::<cxx_qt_lib::QUrl>().map(|qu| String::from(&qu.to_qstring())))
                .unwrap_or_default();
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let local = cxx_qt_lib::QUrl::from(&QString::from(raw))
                .to_local_file()
                .map(|p| String::from(&p))
                .filter(|p| !p.is_empty())
                .or_else(|| {
                    raw.strip_prefix("file:///")
                        .or_else(|| raw.strip_prefix("file://"))
                        .map(|s| s.replace('/', "\\"))
                });
            if let Some(local_str) = local {
                let pb = std::path::PathBuf::from(&local_str);
                if pb.exists() {
                    paths.push(pb);
                    continue;
                }
            }
            let pb = std::path::PathBuf::from(raw);
            if pb.exists() {
                paths.push(pb);
            } else if url_text_fallback.is_empty() {
                url_text_fallback = raw.to_owned();
            }
        }

        if !paths.is_empty() {
            crate::transfers::send(device, paths);
            show_message(format!("Sending to {device_name}…"));
            return;
        }

        let dropped_text = {
            let t = String::from(text).trim().to_owned();
            if t.is_empty() { url_text_fallback } else { t }
        };
        if !dropped_text.is_empty() {
            if crate::links::is_handoff_text(&dropped_text) {
                crate::links::send_to(device, dropped_text);
            } else if let Some(node) = core_host::node() {
                let name = device_name.clone();
                core_host::spawn(async move {
                    match node.send_clipboard(device, dropped_text).await {
                        Ok(()) => show_message(format!("Sent text to {name}.")),
                        Err(e) => show_message(describe(&e)),
                    }
                });
            }
        }
    }

    pub fn activate_shelf_item(&self, kind: &QString, action: &QString, id_or_path: &QString) {
        let kind_str = String::from(kind);
        let action_str = String::from(action);
        let s = String::from(id_or_path).trim().to_owned();

        match kind_str.as_str() {
            "photo" | "screenshot" => {
                let p = std::path::Path::new(&s);
                if !s.is_empty() && p.exists() {
                    match action_str.as_str() {
                        "folder" | "reveal" | "save" => crate::transfers::show_in_folder(p),
                        "copy" => {
                            let mime = match p
                                .extension()
                                .and_then(|e| e.to_str())
                                .map(str::to_ascii_lowercase)
                                .as_deref()
                            {
                                Some("png") => "image/png",
                                _ => "image/jpeg",
                            };
                            match std::fs::read(p)
                                .map_err(|e| e.to_string())
                                .and_then(|b| crate::win::clipboard::write_image(mime, &b))
                            {
                                Ok(()) => show_message("Copied to clipboard."),
                                Err(_) => show_message("The photo couldn't be copied to the clipboard."),
                            }
                        }
                        _ => crate::transfers::open(p),
                    }
                } else if !s.is_empty()
                    && let (Some(dev), _, _) = resolve_shelf_device(self.current_device)
                {
                    let toast_action = match action_str.as_str() {
                        "save" => Some(crate::photos::ACTION_SAVE),
                        "copy" => Some(crate::photos::ACTION_COPY),
                        _ => None,
                    };
                    crate::photos::on_toast(&format!("{dev} {s}"), toast_action);
                }
            }
            "clip" => {
                if !s.is_empty() {
                    crate::clipboard::copy_history_item(&s);
                } else {
                    show_message("Already on clipboard.");
                }
            }
            "file" => {
                let p = std::path::Path::new(&s);
                if !s.is_empty() && p.exists() {
                    if action_str == "folder" || action_str == "reveal" {
                        crate::transfers::show_in_folder(p);
                    } else {
                        crate::transfers::open(p);
                    }
                } else {
                    show_message("That file is no longer in its saved location.");
                }
            }
            _ => {}
        }
    }
}

fn resolve_shelf_device(current_device_idx: i32) -> (Option<DeviceId>, String, bool) {
    core_host::host().hub.read(|s| {
        let idx = current_device_idx.max(0) as usize;
        let chosen = s
            .devices
            .get(idx)
            .filter(|d| matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })))
            .or_else(|| s.devices.get(idx))
            .or_else(|| s.devices.first());
        match chosen {
            Some(d) => (Some(d.id), d.info.name.clone(), matches!(d.link, LinkState::Online { .. })),
            None => (None, String::new(), false),
        }
    })
}

fn compute_shelf_items_json(current_device_idx: i32) -> String {
    use nectarlink_core::{ClipboardItemKind, Direction, TimelineKind, TimelineQuery, TransferState};

    let (device_opt, device_name, online) = resolve_shelf_device(current_device_idx);
    let node = core_host::node();

    let mut latest_photo: Option<serde_json::Value> = None;
    let mut latest_photo_ts: i64 = -1;
    let mut latest_screenshot: Option<serde_json::Value> = None;
    let mut latest_screenshot_ts: i64 = -1;

    // 1. Check HOME_SUMMARY for the active phone (and trigger a one-shot fetch if needed while Shelf is open).
    if let Some(dev) = device_opt {
        let summary = home_store(|s| s.devices.get(&dev).cloned());
        match summary {
            Some(d) if d.photo_loaded => {
                if !d.photo_id.is_empty() {
                    let thumb_p = crate::photos::thumb_path(dev, &d.photo_id);
                    let thumb_url = if !d.photo_thumb.is_empty() {
                        d.photo_thumb.clone()
                    } else if thumb_p.exists() {
                        crate::icons::file_url(&thumb_p)
                    } else {
                        String::new()
                    };
                    let obj = serde_json::json!({
                        "id": d.photo_id,
                        "name": d.photo_name,
                        "date": d.photo_date,
                        "thumbUrl": thumb_url,
                        "localPath": "",
                    });
                    if d.photo_is_screenshot {
                        latest_screenshot = Some(obj);
                        latest_screenshot_ts = d.photo_date;
                    } else {
                        latest_photo = Some(obj);
                        latest_photo_ts = d.photo_date;
                    }
                }
            }
            _ if online => {
                refresh_shelf_photo_if_needed(dev, false);
            }
            _ => {}
        }
    }

    // 2. Check Timeline photo entries (saved photos / screenshots on disk).
    if let Some(ref n) = node
        && let Ok(page) = n.timeline_page(&TimelineQuery {
            kind: Some(TimelineKind::Photo),
            device: device_opt,
            limit: 25,
            ..Default::default()
        })
    {
        for entry in page.entries {
            let first_target = entry.target.lines().next().unwrap_or("").trim();
            if first_target.is_empty() {
                continue;
            }
            let path = std::path::PathBuf::from(first_target);
            if !path.exists() {
                continue;
            }
            let name = path.file_name().and_then(|f| f.to_str()).unwrap_or(&entry.title).to_owned();
            let lower_title = entry.title.to_ascii_lowercase();
            let lower_name = name.to_ascii_lowercase();
            let is_shot = lower_title.contains("screenshot")
                || lower_name.contains("screenshot")
                || entry.detail.to_ascii_lowercase().contains("screenshot");
            let obj = serde_json::json!({
                "id": path.to_string_lossy(),
                "name": name,
                "date": entry.timestamp,
                "thumbUrl": crate::icons::file_url(&path),
                "localPath": path.to_string_lossy(),
            });
            if is_shot {
                if entry.timestamp >= latest_screenshot_ts {
                    latest_screenshot_ts = entry.timestamp;
                    latest_screenshot = Some(obj);
                }
            } else if entry.timestamp >= latest_photo_ts {
                latest_photo_ts = entry.timestamp;
                latest_photo = Some(obj);
            }
        }
    }

    // 3. Scan Downloads\Nectarlink for mirror screenshots ("Screenshot ...") and saved photos.
    let dl_dir = core_host::downloads_dir();
    let mut dl_files: Vec<(i64, u64, std::path::PathBuf, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dl_dir) {
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            let ts = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            dl_files.push((ts, meta.len(), p, name));
        }
    }
    dl_files.sort_by_key(|f| std::cmp::Reverse(f.0));

    for (ts, _, p, name) in &dl_files {
        let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            continue;
        }
        let is_shot = name.to_ascii_lowercase().contains("screenshot");
        if is_shot && *ts > latest_screenshot_ts {
            latest_screenshot_ts = *ts;
            latest_screenshot = Some(serde_json::json!({
                "id": p.to_string_lossy(),
                "name": name,
                "date": *ts,
                "thumbUrl": crate::icons::file_url(p),
                "localPath": p.to_string_lossy(),
            }));
        } else if !is_shot && latest_photo.is_none() {
            latest_photo = Some(serde_json::json!({
                "id": p.to_string_lossy(),
                "name": name,
                "date": *ts,
                "thumbUrl": crate::icons::file_url(p),
                "localPath": p.to_string_lossy(),
            }));
        }
    }

    // 4. Current clip (from encrypted local clipboard history, or live Windows clipboard fallback).
    let current_clip =
        if let Some(entry) = node.as_ref().and_then(|n| n.clipboard_history(None).into_iter().next()) {
            let (kind, image_data_url) = match entry.kind {
                ClipboardItemKind::Text => ("text", String::new()),
                ClipboardItemKind::Image => {
                    ("image", crate::clipboard::thumb_for_clip(&entry.id).unwrap_or_default())
                }
            };
            serde_json::json!({
                "id": entry.id,
                "kind": kind,
                "text": entry.text,
                "imageDataUrl": image_data_url,
                "deviceName": entry.device_name,
                "localPath": "",
            })
        } else {
            match crate::win::clipboard::read() {
                crate::win::clipboard::Clip::Text(t) if !t.trim().is_empty() => serde_json::json!({
                    "id": "",
                    "kind": "text",
                    "text": t,
                    "imageDataUrl": "",
                    "deviceName": "This PC",
                    "localPath": "",
                }),
                _ => serde_json::Value::Null,
            }
        };

    // 5. Recent received files (up to 5 existing files on disk).
    let mut recent_files: Vec<serde_json::Value> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add_file =
        |recent_files: &mut Vec<serde_json::Value>, path: &std::path::Path, dev_label: &str| {
            if recent_files.len() >= 5 || !path.exists() || !path.is_file() {
                return;
            }
            let path_str = path.to_string_lossy().into_owned();
            let key = path_str.to_ascii_lowercase();
            if !seen_paths.insert(key) {
                return;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("File").to_owned();
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            recent_files.push(serde_json::json!({
                "name": name,
                "path": path_str,
                "size": size,
                "deviceName": dev_label,
            }));
        };

    let transfer_files: Vec<(std::path::PathBuf, String)> = core_host::host().hub.read(|s| {
        let mut out = Vec::new();
        for tv in &s.transfers {
            if tv.transfer.direction == Direction::Incoming
                && let TransferState::Done { ref saved } = tv.transfer.state
            {
                let dname = s.name_of(&tv.transfer.device).unwrap_or_else(|| "Phone".into());
                for p in saved {
                    out.push((p.clone(), dname.clone()));
                }
            }
        }
        out
    });
    for (p, dname) in &transfer_files {
        add_file(&mut recent_files, p, dname);
    }

    if recent_files.len() < 5
        && let Some(ref n) = node
        && let Ok(page) = n.timeline_page(&TimelineQuery {
            kind: Some(TimelineKind::File),
            limit: 20,
            ..Default::default()
        })
    {
        for entry in page.entries {
            for line in entry.target.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    add_file(&mut recent_files, std::path::Path::new(trimmed), &entry.device_name);
                }
            }
        }
    }

    for (_, _, p, _) in &dl_files {
        if recent_files.len() >= 5 {
            break;
        }
        add_file(&mut recent_files, p, if device_name.is_empty() { "Phone" } else { &device_name });
    }

    serde_json::json!({
        "hasDevice": device_opt.is_some(),
        "deviceId": device_opt.map(|d| d.to_string()).unwrap_or_default(),
        "deviceName": device_name,
        "online": online,
        "latestPhoto": latest_photo.unwrap_or(serde_json::Value::Null),
        "latestScreenshot": latest_screenshot.unwrap_or(serde_json::Value::Null),
        "currentClip": current_clip,
        "recentFiles": recent_files,
    })
    .to_string()
}

fn compute_data_retention_json() -> String {
    let counts = core_host::node().map(|n| n.data_retention_counts()).unwrap_or_default();
    let (notification_history, notification_history_enabled, transfer_records) =
        core_host::host().hub.read(|s| {
            let finished = s.transfers.iter().filter(|t| t.transfer.state.is_finished()).count();
            (s.history.len(), s.history_enabled, finished)
        });
    let notification_image_bytes = crate::notifications::cached_image_bytes();
    let (cached_sms_threads, cached_sms_messages, message_attachment_bytes) = crate::messages::cache_stats();
    let (photo_thumb_files, photo_thumb_bytes) = crate::photos::cache_stats();
    serde_json::json!({
        "clipboardItems": counts.clipboard_items,
        "clipboardBytes": counts.clipboard_bytes,
        "timelineItems": counts.timeline_items,
        "timelineRetentionDays": counts.timeline_retention_days,
        "notificationHistory": notification_history,
        "notificationHistoryEnabled": notification_history_enabled,
        "notificationImageBytes": notification_image_bytes,
        "chatThreads": counts.chat_threads,
        "chatMessages": counts.chat_messages,
        "cachedSmsThreads": cached_sms_threads,
        "cachedSmsMessages": cached_sms_messages,
        "messageAttachmentBytes": message_attachment_bytes,
        "photoThumbFiles": photo_thumb_files,
        "photoThumbBytes": photo_thumb_bytes,
        "receivedFileRecords": counts.received_file_records as usize + transfer_records,
    })
    .to_string()
}

impl Drop for AppControllerRust {
    fn drop(&mut self) {
        if let Some(cell) = CONTROLLER.get() {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

// ---- Event-driven Home card summary (never polled) ----

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HomeDeviceSummary {
    sms_loaded: bool,
    sms_busy: bool,
    unread_messages: u32,
    unread_sender: String,
    calls_loaded: bool,
    calls_busy: bool,
    missed_calls: u32,
    missed_caller: String,
    photo_loaded: bool,
    photo_busy: bool,
    photo_id: String,
    photo_name: String,
    photo_date: i64,
    photo_thumb: String,
    photo_is_video: bool,
    photo_is_screenshot: bool,
}

#[derive(Debug, Default)]
struct HomeSummaryStore {
    watched: Option<DeviceId>,
    devices: std::collections::HashMap<DeviceId, HomeDeviceSummary>,
}

static HOME_SUMMARY: Mutex<Option<HomeSummaryStore>> = Mutex::new(None);

fn home_store<T>(f: impl FnOnce(&mut HomeSummaryStore) -> T) -> T {
    f(HOME_SUMMARY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn bump_home_summary() {
    if let Some(qt) = controller() {
        let _ = qt.queue(|mut object| {
            let rev = object.home_summary_revision.wrapping_add(1);
            object.as_mut().set_home_summary_revision(rev);
            if object.shelf_open {
                let shelf = compute_shelf_items_json(object.current_device);
                object.set_shelf_items(QString::from(&shelf));
            }
        });
    }
}

fn release_home_summary() {
    home_store(|s| {
        s.watched = None;
        s.devices.clear();
    });
}

fn home_summary_json(device: DeviceId) -> String {
    home_store(|s| {
        let Some(d) = s.devices.get(&device) else { return String::new() };
        serde_json::json!({
            "smsReady": d.sms_loaded,
            "unreadMessages": d.unread_messages,
            "unreadSender": d.unread_sender,
            "callsReady": d.calls_loaded,
            "missedCalls": d.missed_calls,
            "missedCaller": d.missed_caller,
            "photoReady": d.photo_loaded,
            "photoId": d.photo_id,
            "photoName": d.photo_name,
            "photoDate": d.photo_date,
            "photoThumb": d.photo_thumb,
            "photoIsVideo": d.photo_is_video,
            "photoIsScreenshot": d.photo_is_screenshot,
        })
        .to_string()
    })
}

fn watch_home(device: Option<DeviceId>) {
    crate::command_palette::set_preferred_device(device);
    home_store(|s| s.watched = device);
    if let Some(id) = device {
        refresh_home_if_needed(id, false, false, false);
    }
}

pub(crate) fn update_home_sms(device: DeviceId, threads: &[nectarlink_core::SmsThread]) {
    let changed = home_store(|s| {
        let entry = s.devices.entry(device).or_default();
        let unread_messages: u32 = threads.iter().map(|t| t.unread).sum();
        let unread_sender =
            threads.iter().find(|t| t.unread > 0).map(crate::messages::title_of).unwrap_or_default();
        let prev = (entry.sms_loaded, entry.unread_messages, entry.unread_sender.clone());
        entry.sms_loaded = true;
        entry.sms_busy = false;
        entry.unread_messages = unread_messages;
        entry.unread_sender = unread_sender;
        prev != (entry.sms_loaded, entry.unread_messages, entry.unread_sender.clone())
    });
    if changed {
        bump_home_summary();
    }
}

pub(crate) fn update_home_calls(device: DeviceId, entries: &[nectarlink_core::CallLogEntry]) {
    let changed = home_store(|s| {
        let entry = s.devices.entry(device).or_default();
        let missed_calls = entries.iter().filter(|e| e.direction == "missed").count() as u32;
        let missed_caller = entries
            .iter()
            .find(|e| e.direction == "missed")
            .map(|e| e.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| e.number.clone()))
            .unwrap_or_default();
        let prev = (entry.calls_loaded, entry.missed_calls, entry.missed_caller.clone());
        entry.calls_loaded = true;
        entry.calls_busy = false;
        entry.missed_calls = missed_calls;
        entry.missed_caller = missed_caller;
        prev != (entry.calls_loaded, entry.missed_calls, entry.missed_caller.clone())
    });
    if changed {
        bump_home_summary();
    }
}

pub(crate) fn on_home_event(event: &nectarlink_core::NodeEvent) {
    use nectarlink_core::NodeEvent;
    let watched = home_store(|s| s.watched);
    let shelf_active = is_shelf_open();
    match event {
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } if watched == Some(*device) => {
            refresh_home_if_needed(*device, true, true, true);
        }
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } => {
            home_store(|s| {
                if let Some(d) = s.devices.get_mut(device) {
                    d.sms_busy = false;
                    d.calls_busy = false;
                    d.photo_busy = false;
                }
            });
        }
        NodeEvent::Capabilities(matrix) if watched == Some(matrix.device) => {
            refresh_home_if_needed(matrix.device, false, false, false);
        }
        NodeEvent::SmsChanged { device, .. } if watched == Some(*device) => {
            refresh_home_if_needed(*device, true, false, false);
        }
        NodeEvent::CallLogChanged { device } if watched == Some(*device) => {
            refresh_home_if_needed(*device, false, true, false);
        }
        NodeEvent::Call { device, call } if watched == Some(*device) && call.state == "ended" => {
            let dev = *device;
            core_host::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(650)).await;
                refresh_home_if_needed(dev, false, true, false);
            });
        }
        NodeEvent::PhotoAdded { device, photo } if watched == Some(*device) || shelf_active => {
            let thumb_url = crate::photos::write_thumb(*device, &photo.id, &photo.thumb)
                .or_else(|| {
                    let p = crate::photos::thumb_path(*device, &photo.id);
                    p.exists().then_some(p)
                })
                .map(|p| crate::icons::file_url(&p))
                .unwrap_or_default();
            home_store(|s| {
                let entry = s.devices.entry(*device).or_default();
                entry.photo_loaded = true;
                entry.photo_busy = false;
                entry.photo_id.clone_from(&photo.id);
                entry.photo_name.clone_from(&photo.name);
                entry.photo_date = photo.taken;
                entry.photo_thumb = thumb_url;
                entry.photo_is_video = photo.id.starts_with("video:");
                entry.photo_is_screenshot = photo.screenshot;
            });
            bump_home_summary();
        }
        NodeEvent::PhotosChanged { device } if watched == Some(*device) || shelf_active => {
            if watched == Some(*device) {
                refresh_home_if_needed(*device, false, false, true);
            } else {
                refresh_shelf_photo_if_needed(*device, true);
            }
        }
        NodeEvent::DeviceRemoved(device) => {
            home_store(|s| {
                s.devices.remove(device);
                if s.watched == Some(*device) {
                    s.watched = None;
                }
            });
        }
        _ => {}
    }
}

fn refresh_shelf_photo_if_needed(device: DeviceId, force_photo: bool) {
    let (online, photos_avail) = core_host::host().hub.read(|s| {
        let online = s.devices.iter().any(|d| d.id == device && matches!(d.link, LinkState::Online { .. }));
        let avail = s.matrices.get(&device).and_then(|m| m.state("files.recent_photos"))
            == Some(FeatureState::Available);
        (online, avail)
    });
    if !online || !photos_avail {
        return;
    }
    let do_photo = home_store(|s| {
        let d = s.devices.entry(device).or_default();
        let should = !d.photo_busy && (force_photo || !d.photo_loaded);
        if should {
            d.photo_busy = true;
        }
        should
    });
    if do_photo {
        spawn_home_photo_fetch(device);
    }
}

fn spawn_home_photo_fetch(device: DeviceId) {
    let Some(node) = core_host::node() else {
        home_store(|s| {
            if let Some(d) = s.devices.get_mut(&device) {
                d.photo_busy = false;
            }
        });
        return;
    };
    core_host::spawn(async move {
        let Ok(items) = node.photo_list(device, None, None, 1).await else {
            home_store(|s| {
                if let Some(d) = s.devices.get_mut(&device) {
                    d.photo_busy = false;
                }
            });
            return;
        };
        let Some(item) = items.into_iter().next() else {
            home_store(|s| {
                let d = s.devices.entry(device).or_default();
                d.photo_loaded = true;
                d.photo_busy = false;
                d.photo_id.clear();
                d.photo_name.clear();
                d.photo_date = 0;
                d.photo_thumb.clear();
            });
            bump_home_summary();
            return;
        };
        let mut disk_path = crate::photos::thumb_path(device, &item.id);
        if !disk_path.exists()
            && let Ok(thumbs) = node.photo_thumbs(device, vec![item.id.clone()]).await
            && let Some(first) = thumbs.into_iter().next()
            && let Some(saved) = crate::photos::write_thumb(device, &first.id, &first.data)
        {
            disk_path = saved;
        }
        let thumb_url = if disk_path.exists() { crate::icons::file_url(&disk_path) } else { String::new() };
        let lower_name = item.name.to_ascii_lowercase();
        let is_screenshot = lower_name.contains("screenshot")
            || item.album.as_deref().is_some_and(|a| a.to_ascii_lowercase().contains("screenshot"));
        let is_video = item.duration.is_some() || item.id.starts_with("video:");
        home_store(|s| {
            let d = s.devices.entry(device).or_default();
            d.photo_loaded = true;
            d.photo_busy = false;
            d.photo_id = item.id;
            d.photo_name = item.name;
            d.photo_date = item.date;
            d.photo_thumb = thumb_url;
            d.photo_is_video = is_video;
            d.photo_is_screenshot = is_screenshot;
        });
        bump_home_summary();
    });
}

fn refresh_home_if_needed(device: DeviceId, force_sms: bool, force_calls: bool, force_photo: bool) {
    let (online, sms_avail, calls_avail, photos_avail) = core_host::host().hub.read(|s| {
        let online = s.devices.iter().any(|d| d.id == device && matches!(d.link, LinkState::Online { .. }));
        let m = s.matrices.get(&device);
        let avail = |f: &str| m.and_then(|m| m.state(f)) == Some(FeatureState::Available);
        (online, avail("messages.sms"), avail("calls.log"), avail("files.recent_photos"))
    });
    if !online {
        return;
    }
    let (do_sms, do_calls, do_photo) = home_store(|s| {
        if s.watched != Some(device) {
            return (false, false, false);
        }
        let d = s.devices.entry(device).or_default();
        let do_sms = sms_avail && !d.sms_busy && (force_sms || !d.sms_loaded);
        let do_calls = calls_avail && !d.calls_busy && (force_calls || !d.calls_loaded);
        let do_photo = photos_avail && !d.photo_busy && (force_photo || !d.photo_loaded);
        if do_sms {
            d.sms_busy = true;
        }
        if do_calls {
            d.calls_busy = true;
        }
        if do_photo {
            d.photo_busy = true;
        }
        (do_sms, do_calls, do_photo)
    });
    let Some(node) = core_host::node() else { return };
    if do_sms {
        let node = node.clone();
        core_host::spawn(async move {
            match node.sms_threads(device, 40).await {
                Ok(threads) => update_home_sms(device, &threads),
                Err(_) => home_store(|s| {
                    if let Some(d) = s.devices.get_mut(&device) {
                        d.sms_busy = false;
                    }
                }),
            }
        });
    }
    if do_calls {
        let node = node.clone();
        core_host::spawn(async move {
            match node.call_log(device, None, 25).await {
                Ok(entries) => update_home_calls(device, &entries),
                Err(_) => home_store(|s| {
                    if let Some(d) = s.devices.get_mut(&device) {
                        d.calls_busy = false;
                    }
                }),
            }
        });
    }
    if do_photo {
        spawn_home_photo_fetch(device);
    }
}

/// Flattens a feature state for QML.
fn feature_state_map(state: Option<FeatureState>) -> QMap<QMapPair_QString_QVariant> {
    let mut map = QMap::<QMapPair_QString_QVariant>::default();
    let mut put = |key: &str, value: QVariant| map.insert(QString::from(key), value);
    let text = |s: &str| QVariant::from(&QString::from(s));
    let upgrade_fields = |put: &mut dyn FnMut(&str, QVariant), upgrade: &Upgrade| {
        let (action, target) = upgrade.action.describe();
        put("action", text(action));
        put("target", text(&target));
        put("minutes", QVariant::from(&i32::from(upgrade.effort.minutes())));
    };
    match state {
        None => put("state", text("unknown")),
        Some(FeatureState::Available) => put("state", text("available")),
        Some(FeatureState::Partial { limit, upgrade }) => {
            put("state", text("partial"));
            put("limit", text(limit));
            if let Some(upgrade) = upgrade {
                upgrade_fields(&mut put, &upgrade);
            }
        }
        Some(FeatureState::Locked { upgrade }) => {
            put("state", text("locked"));
            upgrade_fields(&mut put, &upgrade);
        }
        Some(FeatureState::Unsupported { reason }) => {
            put("state", text("unsupported"));
            put("reason", text(&reason.describe()));
        }
    }
    map
}
