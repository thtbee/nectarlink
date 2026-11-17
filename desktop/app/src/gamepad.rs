// SPDX-License-Identifier: GPL-3.0-or-later
//! A game controller on the PC for each phone that opens its Gamepad (#442,
//! `docs/protocol/remote.md` §3.1). On Windows it's an Xbox 360 controller
//! through the ViGEmBus driver, when it's installed; on Linux, a `/dev/uinput`
//! device that games and Steam take for one.
//!
//! The phone sends the whole controller with every change and again every
//! 100 ms while it's open. Updates travel in datagrams that can be lost, so
//! when none comes for [`RELEASE_AFTER`] the controller lets go of everything,
//! and with none for [`UNPLUG_AFTER`] it's unplugged.

use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, PadState};

/// No update for this long: everything is let go (a phone that stopped
/// sending mid-press doesn't keep a button held).
const RELEASE_AFTER: Duration = Duration::from_millis(1000);
/// No update for this long: the controller is unplugged.
const UNPLUG_AFTER: Duration = Duration::from_secs(60);
/// How often the worker looks for controllers gone quiet.
const TICK: Duration = Duration::from_millis(250);
/// How long a reliable update waits for the controller to be plugged in.
const PLUG_TIMEOUT: Duration = Duration::from_secs(5);
/// One controller per phone, and never more than this (XInput has 4 slots).
const MAX_PADS: usize = 4;

enum Cmd {
    Set { peer: DeviceId, pad: PadState, reply: Option<Sender<Result<(), String>>> },
    Off { peer: DeviceId },
    StopAll,
}

fn worker() -> &'static Sender<Cmd> {
    static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("gamepad".into()).spawn(move || run(&rx));
        tx
    })
}

/// Whether this PC can have a controller at all (the platform has a way;
/// a missing Windows driver is told to the phone when it tries).
pub fn is_supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// Sets `peer`'s controller to `pad`, plugging it in first if needed. With
/// `wait`, returns once that's done: `Err` is [`nectarlink_core::PAD_DRIVER_MISSING`]
/// when the driver isn't there, or why it failed.
pub fn set(peer: DeviceId, pad: PadState, wait: bool) -> Result<(), String> {
    if !wait {
        let _ = worker().send(Cmd::Set { peer, pad, reply: None });
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    worker().send(Cmd::Set { peer, pad, reply: Some(tx) }).map_err(|_| "controller worker stopped".to_owned())?;
    rx.recv_timeout(PLUG_TIMEOUT).unwrap_or_else(|_| Err("the controller took too long to plug in".into()))
}

/// Unplugs `peer`'s controller.
pub fn off(peer: DeviceId) {
    let _ = worker().send(Cmd::Off { peer });
}

/// Unplugs every controller (remote input turned off, or the app is closing).
pub fn stop_all() {
    let _ = worker().send(Cmd::StopAll);
}

struct Slot {
    device: backend::Pad,
    pad: PadState,
    seen: Instant,
    released: bool,
}

fn run(rx: &Receiver<Cmd>) {
    let mut pads: HashMap<DeviceId, Slot> = HashMap::new();
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Cmd::Set { peer, pad, reply }) => {
                let result = apply(&mut pads, peer, pad);
                if let Err(e) = &result {
                    tracing::debug!("controller for {peer}: {e}");
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Ok(Cmd::Off { peer }) => {
                pads.remove(&peer);
            }
            Ok(Cmd::StopAll) => pads.clear(),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        pads.retain(|_, slot| now.duration_since(slot.seen) < UNPLUG_AFTER);
        for slot in pads.values_mut() {
            if !slot.released && now.duration_since(slot.seen) >= RELEASE_AFTER {
                slot.released = true;
                if slot.pad != PadState::default() {
                    slot.pad = PadState::default();
                    let _ = slot.device.update(&slot.pad);
                }
            }
        }
    }
}

fn apply(pads: &mut HashMap<DeviceId, Slot>, peer: DeviceId, pad: PadState) -> Result<(), String> {
    if !pad.is_valid() {
        return Err("invalid controller state".into());
    }
    let now = Instant::now();
    if let Some(slot) = pads.get_mut(&peer) {
        slot.seen = now;
        slot.released = false;
        if slot.pad != pad {
            slot.pad = pad;
            slot.device.update(&pad)?;
        }
        return Ok(());
    }
    if pads.len() >= MAX_PADS {
        return Err("already as many controllers as the PC takes".into());
    }
    let mut device = backend::Pad::plug_in()?;
    device.update(&pad)?;
    pads.insert(peer, Slot { device, pad, seen: now, released: false });
    Ok(())
}

#[cfg(windows)]
mod backend {
    //! Xbox 360 controller through ViGEmBus (BSD-3-Clause driver, signed by
    //! Microsoft; its 1.22 release is the last and has no updater).
    use nectarlink_core::{PAD_DRIVER_MISSING, PadState};
    use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

    pub struct Pad(Xbox360Wired<Client>);

    impl Pad {
        pub fn plug_in() -> Result<Self, String> {
            let client = Client::connect().map_err(|e| match e {
                vigem_client::Error::BusNotFound => PAD_DRIVER_MISSING.to_owned(),
                other => format!("couldn't reach the controller driver: {other}"),
            })?;
            let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
            target.plugin().map_err(|e| format!("couldn't plug in a controller: {e}"))?;
            target.wait_ready().map_err(|e| format!("the controller didn't get ready: {e}"))?;
            Ok(Self(target))
        }

        pub fn update(&mut self, pad: &PadState) -> Result<(), String> {
            let report = XGamepad {
                buttons: XButtons(pad.buttons),
                left_trigger: pad.lt,
                right_trigger: pad.rt,
                thumb_lx: pad.lx,
                thumb_ly: pad.ly,
                thumb_rx: pad.rx,
                thumb_ry: pad.ry,
            };
            self.0.update(&report).map_err(|e| format!("controller update failed: {e}"))
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    //! A `/dev/uinput` controller that says it's an Xbox 360 one, so SDL,
    //! Steam and games map its buttons without asking.
    use std::ffi::{c_int, c_ulong, c_void};

    use nectarlink_core::PadState;

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_ABS: u16 = 0x03;
    const SYN_REPORT: u16 = 0x00;

    const BTN_SOUTH: u16 = 0x130; // A
    const BTN_EAST: u16 = 0x131; // B
    const BTN_NORTH: u16 = 0x133; // Y (evdev's X/Y naming follows positions)
    const BTN_WEST: u16 = 0x134; // X
    const BTN_TL: u16 = 0x136;
    const BTN_TR: u16 = 0x137;
    const BTN_SELECT: u16 = 0x13a;
    const BTN_START: u16 = 0x13b;
    const BTN_MODE: u16 = 0x13c;
    const BTN_THUMBL: u16 = 0x13d;
    const BTN_THUMBR: u16 = 0x13e;

    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;
    const ABS_Z: u16 = 0x02;
    const ABS_RX: u16 = 0x03;
    const ABS_RY: u16 = 0x04;
    const ABS_RZ: u16 = 0x05;
    const ABS_HAT0X: u16 = 0x10;
    const ABS_HAT0Y: u16 = 0x11;

    const BUS_USB: u16 = 0x03;
    const UI_DEV_CREATE: c_ulong = 0x5501;
    const UI_DEV_DESTROY: c_ulong = 0x5502;
    const UI_DEV_SETUP: c_ulong = 0x405c_5503;
    const UI_ABS_SETUP: c_ulong = 0x401c_5504;
    const UI_SET_EVBIT: c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: c_ulong = 0x4004_5565;
    const UI_SET_ABSBIT: c_ulong = 0x4004_5567;

    /// Which XInput bit is which evdev button.
    const BUTTONS: [(u16, u16); 11] = [
        (PadState::A, BTN_SOUTH),
        (PadState::B, BTN_EAST),
        (PadState::X, BTN_WEST),
        (PadState::Y, BTN_NORTH),
        (PadState::LB, BTN_TL),
        (PadState::RB, BTN_TR),
        (PadState::BACK, BTN_SELECT),
        (PadState::START, BTN_START),
        (PadState::GUIDE, BTN_MODE),
        (PadState::LTHUMB, BTN_THUMBL),
        (PadState::RTHUMB, BTN_THUMBR),
    ];

    #[repr(C)]
    struct InputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }

    #[repr(C)]
    struct UinputSetup {
        id: InputId,
        name: [u8; 80],
        ff_effects_max: u32,
    }

    #[repr(C)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }

    #[repr(C)]
    struct UinputAbsSetup {
        code: u16,
        absinfo: AbsInfo,
    }

    #[repr(C)]
    struct InputEvent {
        tv_sec: libc::time_t,
        tv_usec: libc::suseconds_t,
        type_: u16,
        code: u16,
        value: i32,
    }

    pub struct Pad {
        fd: c_int,
        last: Option<PadState>,
    }

    impl Pad {
        pub fn plug_in() -> Result<Self, String> {
            let mut fd = -1;
            for path in [c"/dev/uinput", c"/dev/input/uinput"] {
                // SAFETY: `path` is a valid null-terminated C string.
                fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
                if fd >= 0 {
                    break;
                }
            }
            if fd < 0 {
                return Err("can't open /dev/uinput".into());
            }
            let pad = Self { fd, last: None };
            // SAFETY: `fd` is an open `/dev/uinput` descriptor owned by `pad`
            // (closed by its Drop on any early return); each ioctl gets the
            // argument type the kernel expects for it.
            unsafe {
                let ok = libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_SYN)) >= 0
                    && libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_KEY)) >= 0
                    && libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_ABS)) >= 0
                    && BUTTONS.iter().all(|(_, b)| libc::ioctl(fd, UI_SET_KEYBIT, c_int::from(*b)) >= 0);
                if !ok {
                    return Err("couldn't set up the controller's buttons".into());
                }
                let axes = [
                    (ABS_X, -32768, 32767, 16, 128),
                    (ABS_Y, -32768, 32767, 16, 128),
                    (ABS_RX, -32768, 32767, 16, 128),
                    (ABS_RY, -32768, 32767, 16, 128),
                    (ABS_Z, 0, 255, 0, 0),
                    (ABS_RZ, 0, 255, 0, 0),
                    (ABS_HAT0X, -1, 1, 0, 0),
                    (ABS_HAT0Y, -1, 1, 0, 0),
                ];
                for (code, minimum, maximum, fuzz, flat) in axes {
                    let setup = UinputAbsSetup {
                        code,
                        absinfo: AbsInfo { value: 0, minimum, maximum, fuzz, flat, resolution: 0 },
                    };
                    if libc::ioctl(fd, UI_SET_ABSBIT, c_int::from(code)) < 0
                        || libc::ioctl(fd, UI_ABS_SETUP, &setup) < 0
                    {
                        return Err("couldn't set up the controller's sticks".into());
                    }
                }
                // An Xbox 360 controller's IDs: the mapping everyone knows.
                let mut setup = UinputSetup {
                    id: InputId { bustype: BUS_USB, vendor: 0x045e, product: 0x028e, version: 0x0110 },
                    name: [0u8; 80],
                    ff_effects_max: 0,
                };
                let name = b"NectarLink Controller";
                setup.name[..name.len()].copy_from_slice(name);
                if libc::ioctl(fd, UI_DEV_SETUP, &setup) < 0 || libc::ioctl(fd, UI_DEV_CREATE) < 0 {
                    return Err("couldn't create the controller".into());
                }
            }
            Ok(pad)
        }

        fn emit(&self, type_: u16, code: u16, value: i32) -> bool {
            let ev = InputEvent { tv_sec: 0, tv_usec: 0, type_, code, value };
            let size = std::mem::size_of::<InputEvent>();
            // SAFETY: `ev` is a repr(C) `input_event` written whole to `self.fd`.
            let written = unsafe { libc::write(self.fd, (&raw const ev).cast::<c_void>(), size) };
            written == size as isize
        }

        pub fn update(&mut self, pad: &PadState) -> Result<(), String> {
            let last = self.last;
            let changed = |f: fn(&PadState) -> i32| last.is_none_or(|l| f(&l) != f(pad));
            let mut ok = true;
            for (bit, code) in BUTTONS {
                if last.is_none_or(|l| (l.buttons & bit) != (pad.buttons & bit)) {
                    ok &= self.emit(EV_KEY, code, i32::from(pad.buttons & bit != 0));
                }
            }
            let hat_x = |p: &PadState| hat(p.buttons, PadState::LEFT, PadState::RIGHT);
            // XInput's up is evdev's -1.
            let hat_y = |p: &PadState| hat(p.buttons, PadState::UP, PadState::DOWN);
            let axes: [(u16, fn(&PadState) -> i32); 8] = [
                (ABS_X, |p| i32::from(p.lx)),
                // evdev's y grows downwards; XInput's grows up.
                (ABS_Y, |p| flip(p.ly)),
                (ABS_RX, |p| i32::from(p.rx)),
                (ABS_RY, |p| flip(p.ry)),
                (ABS_Z, |p| i32::from(p.lt)),
                (ABS_RZ, |p| i32::from(p.rt)),
                (ABS_HAT0X, hat_x),
                (ABS_HAT0Y, hat_y),
            ];
            for (code, value) in axes {
                if changed(value) {
                    ok &= self.emit(EV_ABS, code, value(pad));
                }
            }
            ok &= self.emit(EV_SYN, SYN_REPORT, 0);
            self.last = Some(*pad);
            if ok { Ok(()) } else { Err("controller update failed".into()) }
        }
    }

    fn hat(buttons: u16, minus: u16, plus: u16) -> i32 {
        i32::from(buttons & plus != 0) - i32::from(buttons & minus != 0)
    }

    fn flip(y: i16) -> i32 {
        (-i32::from(y)).min(32767)
    }

    impl Drop for Pad {
        fn drop(&mut self) {
            // SAFETY: `self.fd` is a uinput descriptor owned by `self`.
            unsafe {
                let _ = libc::ioctl(self.fd, UI_DEV_DESTROY);
                let _ = libc::close(self.fd);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dpad_and_sticks_point_the_evdev_way() {
            assert_eq!(hat(PadState::UP, PadState::UP, PadState::DOWN), -1);
            assert_eq!(hat(PadState::DOWN, PadState::UP, PadState::DOWN), 1);
            assert_eq!(hat(PadState::UP | PadState::DOWN, PadState::UP, PadState::DOWN), 0);
            assert_eq!(hat(PadState::RIGHT, PadState::LEFT, PadState::RIGHT), 1);
            assert_eq!(flip(32767), -32767);
            assert_eq!(flip(-32768), 32767);
            assert_eq!(flip(0), 0);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod backend {
    //! macOS: a virtual controller needs a system extension Apple has to
    //! allow first, so there's none yet.
    use nectarlink_core::PadState;

    pub struct Pad;

    impl Pad {
        pub fn plug_in() -> Result<Self, String> {
            Err("no game controller on this system yet".into())
        }

        pub fn update(&mut self, _pad: &PadState) -> Result<(), String> {
            Ok(())
        }
    }
}
