// SPDX-License-Identifier: GPL-3.0-or-later
//! The Rust side of the proof: a QObject that QML binds to, fed from a
//! background thread the way `nectarlink-core` events will be. All updates
//! cross into Qt through `qt_thread().queue(...)`, the single pattern the
//! real `nectarlink-qt` bridge will use.

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    // Expose snake_case Rust names to QML as camelCase (deviceName, startStress...).
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, device_name)]
        #[qproperty(i32, battery)]
        #[qproperty(bool, charging)]
        #[qproperty(i32, rtt_ms)]
        #[qproperty(i32, events_per_second)]
        #[qproperty(f64, events_total)]
        #[qproperty(f64, working_set_mb)]
        #[qproperty(f64, startup_ms)]
        #[qproperty(bool, stress_running)]
        type DeviceModel = super::DeviceModelRust;

        /// Starts simulated core events at `rate` per second.
        #[qinvokable]
        fn start_stress(self: Pin<&mut DeviceModel>, rate: i32);

        #[qinvokable]
        fn stop_stress(self: Pin<&mut DeviceModel>);

        /// Called by QML when the first frame is on screen.
        #[qinvokable]
        fn first_frame(self: Pin<&mut DeviceModel>);

        #[qinvokable]
        fn refresh_memory(self: Pin<&mut DeviceModel>);

        /// A phone notification arrived.
        #[qsignal]
        fn notification(self: Pin<&mut DeviceModel>, app: QString, title: QString, body: QString);
    }

    impl cxx_qt::Threading for DeviceModel {}
}

/// Process start, set in `main` before Qt starts.
pub static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

pub struct DeviceModelRust {
    device_name: QString,
    battery: i32,
    charging: bool,
    rtt_ms: i32,
    events_per_second: i32,
    events_total: f64,
    working_set_mb: f64,
    startup_ms: f64,
    stress_running: bool,
    stop: Option<Arc<AtomicBool>>,
}

impl Default for DeviceModelRust {
    fn default() -> Self {
        Self {
            device_name: QString::from("Pixel 9"),
            battery: 82,
            charging: true,
            rtt_ms: 12,
            events_per_second: 0,
            events_total: 0.0,
            working_set_mb: 0.0,
            startup_ms: 0.0,
            stress_running: false,
            stop: None,
        }
    }
}

const SAMPLE_NOTIFICATIONS: &[(&str, &str, &str)] = &[
    ("WhatsApp", "Mom", "Dinner at 8? Bring the charger you borrowed."),
    ("Google", "Sign-in code", "Your verification code is 482 913"),
    ("Swiggy", "On the way", "Your order arrives in 12 minutes"),
    ("Calendar", "Design review", "With Aanya at 4:30 PM"),
    ("Discord", "#general", "6 new messages"),
    ("Gmail", "Rahul", "Pushed the fix, can you check?"),
];

/// State waiting to be applied on the Qt thread.
#[derive(Default)]
struct PendingUi {
    battery: i32,
    rtt_ms: i32,
    events_total: u64,
    events_per_second: i32,
    /// Events that must not be merged are batched, bounded.
    notifications: Vec<(&'static str, &'static str, &'static str)>,
}

/// Coalesces updates from background threads into at most one queued
/// closure at a time, so a flood of events can never pile up work (or
/// memory) on the Qt thread. The UI always applies the latest state.
struct UiPump {
    qt: CxxQtThread<qobject::DeviceModel>,
    pending: Arc<Mutex<PendingUi>>,
    scheduled: Arc<AtomicBool>,
}

/// Keep at most this many unseen notifications; older ones are dropped
/// from the live view (the app's history keeps them all).
const MAX_PENDING_NOTIFICATIONS: usize = 50;

impl UiPump {
    fn new(qt: CxxQtThread<qobject::DeviceModel>) -> Self {
        Self { qt, pending: Arc::default(), scheduled: Arc::default() }
    }

    /// Applies `f` to the pending state and schedules a flush if none is
    /// queued. Returns false once the QObject has been destroyed.
    fn update(&self, f: impl FnOnce(&mut PendingUi)) -> bool {
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut pending);
            let excess = pending.notifications.len().saturating_sub(MAX_PENDING_NOTIFICATIONS);
            pending.notifications.drain(..excess);
        }
        if self.scheduled.swap(true, Ordering::AcqRel) {
            return true; // a flush is already queued and will see this update
        }
        let pending = self.pending.clone();
        let scheduled = self.scheduled.clone();
        self.qt
            .queue(move |mut model| {
                // Clear first so updates arriving during the flush schedule another.
                scheduled.store(false, Ordering::Release);
                // Scalars are the latest values; only the notification batch is drained.
                let (battery, rtt_ms, events_total, events_per_second, notifications) = {
                    let mut state = pending.lock().unwrap_or_else(|e| e.into_inner());
                    let batch = std::mem::take(&mut state.notifications);
                    (state.battery, state.rtt_ms, state.events_total, state.events_per_second, batch)
                };
                model.as_mut().set_battery(battery);
                model.as_mut().set_rtt_ms(rtt_ms);
                model.as_mut().set_events_total(events_total as f64);
                model.as_mut().set_events_per_second(events_per_second);
                for (app, title, body) in notifications {
                    model.as_mut().notification(
                        QString::from(app),
                        QString::from(title),
                        QString::from(body),
                    );
                }
            })
            .is_ok()
    }
}

impl qobject::DeviceModel {
    pub fn start_stress(mut self: Pin<&mut Self>, rate: i32) {
        self.as_mut().stop_stress();
        let rate = rate.clamp(1, 100_000) as u64;
        let stop = Arc::new(AtomicBool::new(false));
        self.as_mut().rust_mut().stop = Some(stop.clone());
        self.as_mut().set_stress_running(true);

        let pump = UiPump::new(self.qt_thread());
        thread::spawn(move || {
            let interval = Duration::from_nanos(1_000_000_000 / rate);
            let mut next = Instant::now();
            let mut window_start = Instant::now();
            let mut sent: u64 = 0;
            let mut window_sent: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                sent += 1;
                window_sent += 1;
                let n = sent;
                let per_second = if window_start.elapsed() >= Duration::from_secs(1) {
                    let eps = window_sent as i32;
                    window_sent = 0;
                    window_start = Instant::now();
                    Some(eps)
                } else {
                    None
                };
                let alive = pump.update(|state| {
                    state.battery = ((n / 50) % 100) as i32;
                    state.rtt_ms = 8 + (n % 9) as i32;
                    state.events_total = n;
                    if let Some(eps) = per_second {
                        state.events_per_second = eps;
                    }
                    if n % 2_500 == 0 {
                        let (app, title, body) =
                            SAMPLE_NOTIFICATIONS[(n as usize / 2_500) % SAMPLE_NOTIFICATIONS.len()];
                        state.notifications.push((app, title, body));
                    }
                });
                if !alive {
                    return; // the QObject is gone
                }
                next += interval;
                let now = Instant::now();
                if next > now {
                    thread::sleep(next - now);
                } else if now - next > Duration::from_millis(250) {
                    next = now; // fell behind; don't burst to catch up
                }
            }
            pump.update(|state| state.events_per_second = 0);
        });
    }

    pub fn stop_stress(mut self: Pin<&mut Self>) {
        if let Some(stop) = self.as_mut().rust_mut().stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.set_stress_running(false);
    }

    pub fn first_frame(mut self: Pin<&mut Self>) {
        if self.startup_ms > 0.0 {
            return;
        }
        if let Some(started) = STARTED.get() {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            println!("first frame after {ms:.0} ms");
            self.as_mut().set_startup_ms(ms);
        }
        self.refresh_memory();
    }

    pub fn refresh_memory(self: Pin<&mut Self>) {
        let mb = working_set_mb();
        self.set_working_set_mb(mb);
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn working_set_mb() -> f64 {
    use windows::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: valid pseudo-handle and a correctly sized, writable struct.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok.is_ok() { counters.WorkingSetSize as f64 / (1024.0 * 1024.0) } else { 0.0 }
}

#[cfg(not(windows))]
fn working_set_mb() -> f64 {
    0.0
}
