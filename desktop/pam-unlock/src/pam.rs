// SPDX-License-Identifier: MPL-2.0
#![allow(unsafe_code)]

use std::{
    ffi::{CStr, c_char, c_int, c_void},
    io,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, Instant},
};

use nectarlink_protocol::LinuxUnlockChallenge;
use ring::rand::{SecureRandom, SystemRandom};
use rustix::process::{geteuid, getuid};

use crate::{
    relay::{request_signature, verify_signature},
    store::{DEFAULT_ENROLLMENT_DIR, EnrollmentStore},
};

const PAM_SUCCESS: c_int = 0;
const PAM_AUTH_ERR: c_int = 7;
const PAM_SERVICE: c_int = 1;
const PAM_USER: c_int = 2;
const PAM_RHOST: c_int = 4;
const SERVICE: &[u8] = b"nectarlink-phone-lock";

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pam_get_item(pamh: *const PamHandle, item_type: c_int, item: *mut *const c_void) -> c_int;
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pam_sm_authenticate(
    pamh: *mut PamHandle,
    _flags: c_int,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    let deadline = Instant::now() + Duration::from_secs(30);
    std::panic::catch_unwind(|| {
        // Linux-PAM owns the handle and the module arguments for this call.
        unsafe { authenticate(pamh, argc, argv, deadline) }.map_or(PAM_AUTH_ERR, |()| PAM_SUCCESS)
    })
    .unwrap_or(PAM_AUTH_ERR)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pam_sm_setcred(
    pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    // This authentication mechanism grants no supplementary credentials.
    if unsafe { local_pam_uid(pamh) }.is_ok() { PAM_SUCCESS } else { PAM_AUTH_ERR }
}

unsafe fn authenticate(
    pamh: *const PamHandle,
    argc: c_int,
    argv: *const *const c_char,
    deadline: Instant,
) -> Result<(), ()> {
    let uid = unsafe { local_pam_uid(pamh) }?;
    if !(0..=2).contains(&argc) || (argc > 0 && argv.is_null()) {
        return Err(());
    }
    let mut enrollment_dir = None;
    let mut runtime_dir = None;
    for index in 0..argc as usize {
        let arg = unsafe { argv.add(index).read() };
        if arg.is_null() {
            return Err(());
        }
        let arg = unsafe { CStr::from_ptr(arg) }.to_str().map_err(|_| ())?;
        let (name, path) = arg.split_once('=').ok_or(())?;
        match name {
            "enrollment_dir" if enrollment_dir.is_none() => enrollment_dir = Some(path),
            "runtime_dir" if runtime_dir.is_none() => runtime_dir = Some(PathBuf::from(path)),
            _ => return Err(()),
        }
    }
    let enrollment_dir = Path::new(enrollment_dir.unwrap_or(DEFAULT_ENROLLMENT_DIR));
    let runtime_dir = runtime_dir.unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    let store = EnrollmentStore::open(enrollment_dir).map_err(|_| ())?;
    let (enrollment, generation) = store.load(uid).map_err(|_| ())?;
    let mut nonce = [0; 32];
    SystemRandom::new().fill(&mut nonce).map_err(|_| ())?;
    let challenge = LinuxUnlockChallenge { pc: enrollment.pc, phone: enrollment.phone, uid, nonce };
    let signature = request_signature(&runtime_dir, &challenge, deadline).map_err(|_| ())?;
    if !verify_signature(&enrollment.public_key, &challenge, &signature) {
        return Err(());
    }
    let current_store = EnrollmentStore::open(enrollment_dir).map_err(|_| ())?;
    let (current, current_generation) = current_store.load(uid).map_err(|_| ())?;
    if current == enrollment && current_generation == generation && Instant::now() < deadline {
        Ok(())
    } else {
        Err(())
    }
}

unsafe fn local_pam_uid(pamh: *const PamHandle) -> Result<u32, ()> {
    let service = unsafe { item(pamh, PAM_SERVICE) }?.ok_or(())?;
    let remote = unsafe { item(pamh, PAM_RHOST) }?;
    if service.to_bytes() != SERVICE || remote.is_some_and(|value| !value.is_empty()) {
        return Err(());
    }
    let account = unsafe { item(pamh, PAM_USER) }?.ok_or(())?;
    let (uid, _) = lookup_account(|entry, buffer, length, result| unsafe {
        libc::getpwnam_r(account.as_ptr(), entry, buffer, length, result)
    })
    .map_err(|_| ())?;
    if uid == 0 || uid != getuid().as_raw() || uid != geteuid().as_raw() {
        return Err(());
    }
    Ok(uid)
}

// The caller must keep its Linux-PAM handle alive while borrowing an item.
unsafe fn item<'a>(pamh: *const PamHandle, key: c_int) -> Result<Option<&'a CStr>, ()> {
    let mut result = ptr::null();
    if pamh.is_null() || unsafe { pam_get_item(pamh, key, &mut result) } != PAM_SUCCESS {
        return Err(());
    }
    Ok((!result.is_null()).then(|| unsafe { CStr::from_ptr(result.cast()) }))
}

pub fn account_name(uid: u32) -> io::Result<String> {
    lookup_account(|entry, buffer, length, result| unsafe {
        libc::getpwuid_r(uid, entry, buffer, length, result)
    })
    .map(|(_, name)| name)
}

fn lookup_account(
    lookup: impl Fn(*mut libc::passwd, *mut c_char, usize, *mut *mut libc::passwd) -> c_int,
) -> io::Result<(u32, String)> {
    let mut buffer = vec![0; 1024];
    loop {
        let mut result = ptr::null_mut();
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let code = lookup(entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result);
        match code {
            0 if !result.is_null() => {
                let entry = unsafe { entry.assume_init() };
                let name = unsafe { CStr::from_ptr(entry.pw_name) }
                    .to_str()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                return Ok((entry.pw_uid, name.to_owned()));
            }
            0 => return Err(io::Error::new(io::ErrorKind::NotFound, "unknown account")),
            libc::ERANGE if buffer.len() < 1024 * 1024 => buffer.resize(buffer.len() * 2, 0),
            _ => return Err(io::Error::from_raw_os_error(code)),
        }
    }
}
