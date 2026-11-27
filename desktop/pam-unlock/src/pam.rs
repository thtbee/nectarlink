// SPDX-License-Identifier: GPL-3.0-or-later
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
const PAM_PERM_DENIED: c_int = 6;
const PAM_AUTH_ERR: c_int = 7;
const PAM_AUTHINFO_UNAVAIL: c_int = 9;
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
    // Linux-PAM owns the handle and the module arguments for this call.
    match std::panic::catch_unwind(|| unsafe { authenticate(pamh, argc, argv, deadline) }) {
        Ok(Ok(())) => PAM_SUCCESS,
        Ok(Err(Failure::Unavailable)) => PAM_AUTHINFO_UNAVAIL,
        _ => PAM_AUTH_ERR,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pam_sm_setcred(
    pamh: *mut PamHandle,
    _flags: c_int,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    // This authentication mechanism grants no supplementary credentials.
    pam_result(PAM_AUTH_ERR, || unsafe { scoped_uid(pamh, argc, argv) }.map(|_| ()))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pam_sm_acct_mgmt(
    pamh: *mut PamHandle,
    _flags: c_int,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    // Apply the same service and UID scope in the account phase.
    pam_result(PAM_PERM_DENIED, || unsafe { scoped_uid(pamh, argc, argv) }.map(|_| ()))
}

fn pam_result(error: c_int, check: impl FnOnce() -> Result<(), ()> + std::panic::UnwindSafe) -> c_int {
    match std::panic::catch_unwind(check) {
        Ok(Ok(())) => PAM_SUCCESS,
        _ => error,
    }
}

enum Failure {
    /// Nothing is enrolled for this account, or NectarLink isn't serving the session.
    /// Only reported before the phone signs, so a failed security check never is.
    Unavailable,
    Denied,
}

impl From<()> for Failure {
    fn from((): ()) -> Self {
        Self::Denied
    }
}

fn availability(error: io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => Failure::Unavailable,
        _ => Failure::Denied,
    }
}

#[derive(Default)]
struct Arguments<'a> {
    enrollment_dir: Option<&'a str>,
    runtime_dir: Option<&'a str>,
    /// One more PAM service the administrator lets run this module, such as
    /// `kde-fingerprint`, which KDE's lock screen starts beside the password.
    allow_service: Option<&'a str>,
}

impl<'a> Arguments<'a> {
    fn parse(args: impl IntoIterator<Item = &'a str>) -> Result<Self, ()> {
        let mut parsed = Self::default();
        for arg in args {
            let (name, value) = arg.split_once('=').ok_or(())?;
            let slot = match name {
                "enrollment_dir" => &mut parsed.enrollment_dir,
                "runtime_dir" => &mut parsed.runtime_dir,
                "allow_service" => &mut parsed.allow_service,
                _ => return Err(()),
            };
            if slot.is_some() || value.is_empty() {
                return Err(());
            }
            *slot = Some(value);
        }
        Ok(parsed)
    }
}

// Linux-PAM keeps the module arguments alive for the duration of the call.
unsafe fn module_arguments<'a>(argc: c_int, argv: *const *const c_char) -> Result<Arguments<'a>, ()> {
    if !(0..=3).contains(&argc) || (argc > 0 && argv.is_null()) {
        return Err(());
    }
    let mut args = Vec::with_capacity(argc as usize);
    for index in 0..argc as usize {
        let arg = unsafe { argv.add(index).read() };
        if arg.is_null() {
            return Err(());
        }
        args.push(unsafe { CStr::from_ptr(arg) }.to_str().map_err(|_| ())?);
    }
    Arguments::parse(args)
}

unsafe fn scoped_uid(pamh: *const PamHandle, argc: c_int, argv: *const *const c_char) -> Result<u32, ()> {
    let args = unsafe { module_arguments(argc, argv) }?;
    unsafe { local_pam_uid(pamh, args.allow_service) }
}

unsafe fn authenticate(
    pamh: *const PamHandle,
    argc: c_int,
    argv: *const *const c_char,
    deadline: Instant,
) -> Result<(), Failure> {
    let args = unsafe { module_arguments(argc, argv) }?;
    let uid = unsafe { local_pam_uid(pamh, args.allow_service) }?;
    let enrollment_dir = Path::new(args.enrollment_dir.unwrap_or(DEFAULT_ENROLLMENT_DIR));
    let runtime_dir = args.runtime_dir.map_or_else(|| PathBuf::from(format!("/run/user/{uid}")), PathBuf::from);
    let store = EnrollmentStore::open(enrollment_dir).map_err(availability)?;
    let (enrollment, generation) = store.load(uid).map_err(availability)?;
    let mut nonce = [0; 32];
    SystemRandom::new().fill(&mut nonce).map_err(|_| Failure::Denied)?;
    let challenge = LinuxUnlockChallenge { pc: enrollment.pc, phone: enrollment.phone, uid, nonce };
    let signature = request_signature(&runtime_dir, &challenge, deadline).map_err(availability)?;
    // From here on every failure is a denial, a revoked enrollment included.
    if !verify_signature(&enrollment.public_key, &challenge, &signature) {
        return Err(Failure::Denied);
    }
    let current_store = EnrollmentStore::open(enrollment_dir).map_err(|_| Failure::Denied)?;
    let (current, current_generation) = current_store.load(uid).map_err(|_| Failure::Denied)?;
    if current == enrollment && current_generation == generation && Instant::now() < deadline {
        Ok(())
    } else {
        Err(Failure::Denied)
    }
}

unsafe fn local_pam_uid(pamh: *const PamHandle, allow_service: Option<&str>) -> Result<u32, ()> {
    let service = unsafe { item(pamh, PAM_SERVICE) }?.ok_or(())?.to_bytes();
    let remote = unsafe { item(pamh, PAM_RHOST) }?;
    let allowed = service == SERVICE || allow_service.is_some_and(|name| service == name.as_bytes());
    if !allowed || remote.is_some_and(|value| !value.is_empty()) {
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
                if entry.pw_name.is_null() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "account has no name"));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_arguments_are_named_once_and_never_empty() {
        let args = Arguments::parse(["allow_service=kde-fingerprint", "runtime_dir=/run/user/1000"]).unwrap();
        assert_eq!(args.allow_service, Some("kde-fingerprint"));
        assert_eq!(args.runtime_dir, Some("/run/user/1000"));
        assert_eq!(args.enrollment_dir, None);
        assert!(Arguments::parse(["allow_service=a", "allow_service=b"]).is_err());
        assert!(Arguments::parse(["allow_service="]).is_err());
        assert!(Arguments::parse(["allow_service"]).is_err());
        assert!(Arguments::parse(["debug=1"]).is_err());
    }

    #[test]
    fn only_a_missing_enrollment_or_relay_is_unavailable() {
        assert!(matches!(availability(io::ErrorKind::NotFound.into()), Failure::Unavailable));
        assert!(matches!(availability(io::ErrorKind::ConnectionRefused.into()), Failure::Unavailable));
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidData, io::ErrorKind::TimedOut] {
            assert!(matches!(availability(kind.into()), Failure::Denied));
        }
    }

    #[test]
    fn a_missing_account_name_fails_closed() {
        let error = lookup_account(|entry, _, _, result| {
            // Simulate an NSS result with a null pw_name. All fields allow zero.
            unsafe {
                entry.write(std::mem::zeroed());
                result.write(entry);
            }
            0
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
