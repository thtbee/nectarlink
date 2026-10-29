// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Camera Extension gate. No automatic installation or helper download.

pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

/// The legacy upstream stream carries no request/session capture authority.
/// Keep admission closed until owned Start/Stop/Config, original-session
/// ownership, and Android terminal cleanup are implemented and reviewed.
/// Signed extension presence or a local Asking peer cannot replace that proof.
pub const fn stream_admission_ready() -> bool {
    false
}

#[cfg(feature = "macos-native-camera")]
mod native {
    #![allow(unsafe_code)]
    use std::{
        ffi::c_void,
        sync::atomic::{AtomicU8, Ordering},
    };
    // Activation success still needs a live exact-format device observation.
    static STATUS: AtomicU8 = AtomicU8::new(0);
    unsafe extern "C" {
        fn nl_camera_payload_valid() -> i32;
        fn nl_camera_is_registered() -> i32;
        fn nl_camera_request_activation(
            enable: i32,
            callback: unsafe extern "C" fn(*mut c_void, i32, i64),
            context: *mut c_void,
        );
    }
    pub fn is_registered() -> bool {
        if matches!(STATUS.load(Ordering::Acquire), 1 | 2 | 4 | 5 | 6) {
            return false;
        }
        // SAFETY: signature/metadata queries never install or open a stream.
        unsafe { nl_camera_payload_valid() == 1 && nl_camera_is_registered() == 1 }
    }
    struct Activation {
        enabled: bool,
        done: Option<Box<dyn FnOnce(bool) + Send>>,
    }
    unsafe extern "C" fn event(context: *mut c_void, status: i32, _error: i64) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if context.is_null() {
                return;
            }
            // SAFETY: native delegate borrows for pending results and returns
            // this exact context once for a terminal completion/failure.
            let activation = unsafe { &mut *context.cast::<Activation>() };
            match status {
                0 | 1 => {
                    STATUS.store(if status == 0 { 1 } else { 2 }, Ordering::Release);
                    return;
                }
                2 => STATUS.store(if activation.enabled { 3 } else { 6 }, Ordering::Release),
                3 => STATUS.store(4, Ordering::Release),
                _ => STATUS.store(5, Ordering::Release),
            }
            // SAFETY: terminal callback takes the one allocation back.
            let mut activation = unsafe { Box::from_raw(context.cast::<Activation>()) };
            if let Some(done) = activation.done.take() {
                done(is_registered());
            }
            crate::core_host::refresh_capabilities();
        }));
    }
    pub fn run(arg: &'static str, done: impl FnOnce(bool) + Send + 'static) {
        let enabled = arg == super::REGISTER_ARG;
        if !enabled && arg != super::UNREGISTER_ARG {
            done(false);
            return;
        }
        // A later explicit Setup action checks publication after activation.
        if enabled && STATUS.load(Ordering::Acquire) == 3 {
            done(is_registered());
            crate::core_host::refresh_capabilities();
            return;
        }
        // SAFETY: side-effect-free strict same-team signed bundle validation.
        if matches!(STATUS.load(Ordering::Acquire), 1 | 2 | 4) || unsafe { nl_camera_payload_valid() != 1 } {
            done(false);
            return;
        }
        if !enabled {
            crate::webcam::stop();
        }
        STATUS.store(1, Ordering::Release);
        crate::core_host::refresh_capabilities();
        let context = Box::into_raw(Box::new(Activation { enabled, done: Some(Box::new(done)) }));
        // SAFETY: delegate retains context until the terminal callback; only
        // the explicit shared Setup/Remove action reaches this function.
        unsafe { nl_camera_request_activation(i32::from(enabled), event, context.cast()) };
    }
}

pub fn is_registered() -> bool {
    #[cfg(feature = "macos-native-camera")]
    {
        native::is_registered()
    }
    #[cfg(not(feature = "macos-native-camera"))]
    {
        false
    }
}

pub fn register_hklm() -> Result<(), String> {
    Err("Use the app's explicit camera extension setup action.".into())
}
pub fn unregister_hklm() -> Result<(), String> {
    Err("Use the app's explicit camera extension remove action.".into())
}

pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    #[cfg(feature = "macos-native-camera")]
    native::run(arg, on_done);
    #[cfg(not(feature = "macos-native-camera"))]
    {
        let _ = arg;
        on_done(false);
    }
}

#[derive(Debug, Default)]
pub struct VirtualCameraHandle;
impl VirtualCameraHandle {
    pub fn start() -> Result<Self, String> {
        is_registered().then_some(Self).ok_or_else(|| "The signed camera extension is not ready.".into())
    }
}
