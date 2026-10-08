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
        type AppController = super::AppControllerRust;

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
        /// Releases idle UI caches and trims the working set when closed to tray.
        #[qinvokable]
        fn trim_working_set(self: &AppController);

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
    tray: Option<tray::Tray>,
}

/// The controller's thread handle, so other threads (second launch) can
/// reach it.
static CONTROLLER: OnceLock<Mutex<Option<CxxQtThread<qobject::AppController>>>> = OnceLock::new();

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
        self.as_mut().refresh_appearance();
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
        super::subscribe(qt.clone(), Changes::CLIPBOARD, |object| {
            let json = crate::clipboard::history_json();
            object.set_clipboard_history(QString::from(&json));
        });

        let labels = tray::MenuLabels {
            open: "Open Nectarlink".into(),
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
            (
                s.core_status(),
                s.devices.len(),
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

    pub fn trim_working_set(&self) {
        crate::webcam::release_idle_resources();
        crate::photos::release_idle_resources();
        crate::messages::release_idle_resources();
        crate::calls::release_idle_resources();
        super::native::ffi::trim_memory_caches();
    }
}

impl Drop for AppControllerRust {
    fn drop(&mut self) {
        if let Some(cell) = CONTROLLER.get() {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
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
