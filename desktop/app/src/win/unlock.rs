// SPDX-License-Identifier: GPL-3.0-or-later
//! Unlock this PC with the phone (docs/adr/0009-unlock-with-phone.md), the app's
//! half on Windows:
//!
//! - registers the credential provider (`nectarlink_credprov.dll`, next to the
//!   app in Program Files) when the user turns it on, with one administrator
//!   prompt, and removes it when it's off;
//! - serves the unlock pipe the provider asks on: only SYSTEM can open it, only
//!   LogonUI in this session is answered, and only while the session is locked;
//! - enrolls the signed-in Windows user with a phone, after checking the
//!   password with Windows.

#![allow(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use nectarlink_core::{DeviceId, Error, unlock::Secret};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_LOGON_FAILURE, ERROR_PATH_NOT_FOUND,
            ERROR_PIPE_CONNECTED, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree,
        },
        Security::{
            Authentication::Identity::{
                EXTENDED_NAME_FORMAT, GetUserNameExW, NameDisplay, NameSamCompatible,
            },
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            GetTokenInformation, IsWellKnownSid, LOGON32_LOGON_INTERACTIVE, LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT, LogonUserW, PSECURITY_DESCRIPTOR, RevertToSelf, SECURITY_ATTRIBUTES,
            TOKEN_QUERY, TOKEN_USER, TokenUser, WinLocalSystemSid,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
                GetNamedPipeClientSessionId, ImpersonateNamedPipeClient, PIPE_READMODE_MESSAGE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
            },
            Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW},
            RemoteDesktop::{
                ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE, WTSFreeMemory, WTSINFOEXW,
                WTSQuerySessionInformationW, WTSSessionInfoEx,
            },
            Threading::{
                GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, OpenProcessToken, OpenThreadToken,
            },
        },
    },
    core::{HSTRING, PCWSTR, PWSTR},
};
use wire::CLSID_TEXT;

/// The unlock pipe's messages, shared with the credential provider by source.
#[path = "../../../credprov/src/wire.rs"]
#[allow(dead_code)]
mod wire;

pub const REGISTER_ARG: &str = "--register-unlock";
pub const UNREGISTER_ARG: &str = "--unregister-unlock";

const CLASSES: &str = r"SOFTWARE\Classes\CLSID";
const PROVIDERS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers";
/// `WTS_SESSIONSTATE_LOCK` in `WTSINFOEX_LEVEL1_W::SessionFlags`.
const SESSION_LOCKED: i32 = 0;

fn dll_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("nectarlink_credprov.dll"))
}

/// Whether `path` is under Program Files, where only administrators can write.
/// A DLL that LogonUI (SYSTEM) loads from anywhere a user can change would let
/// that user become SYSTEM, so nothing else is ever registered.
fn protected(path: &Path) -> bool {
    let Some(program_files) = std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles"))
    else {
        return false;
    };
    let base = PathBuf::from(program_files);
    path.is_file()
        && path.parent().is_some_and(|dir| {
            dir.to_string_lossy().to_lowercase().starts_with(&(base.to_string_lossy().to_lowercase() + "\\"))
        })
}

fn reg_string(subkey: &str, name: Option<&str>) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let name = name.map(HSTRING::from);
    // SAFETY: `buf` and `size` describe a valid buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(subkey),
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Whether the credential provider is registered, for this install's DLL.
pub fn is_registered() -> bool {
    let (Some(dll), Some(registered)) =
        (dll_path(), reg_string(&format!(r"{CLASSES}\{CLSID_TEXT}\InprocServer32"), None))
    else {
        return false;
    };
    reg_string(&format!(r"{PROVIDERS}\{CLSID_TEXT}"), None).is_some()
        && PathBuf::from(registered).eq(&dll)
        && protected(&dll)
}

/// Registers the credential provider (run elevated, with [`REGISTER_ARG`]).
pub fn register_hklm() -> Result<(), String> {
    let dll = dll_path().ok_or("can't find nectarlink_credprov.dll")?;
    if !protected(&dll) {
        return Err(format!("{} isn't in Program Files; not registering it", dll.display()));
    }
    let dll = dll.to_string_lossy();
    let class = format!(r"{CLASSES}\{CLSID_TEXT}");
    crate::win::vcam::create_and_set(HKEY_LOCAL_MACHINE, &class, &[(None, "NectarLink unlock with phone")])?;
    crate::win::vcam::create_and_set(
        HKEY_LOCAL_MACHINE,
        &format!(r"{class}\InprocServer32"),
        &[(None, dll.as_ref()), (Some("ThreadingModel"), "Apartment")],
    )?;
    crate::win::vcam::create_and_set(
        HKEY_LOCAL_MACHINE,
        &format!(r"{PROVIDERS}\{CLSID_TEXT}"),
        &[(None, "NectarLink")],
    )
}

/// Removes the credential provider (run elevated, with [`UNREGISTER_ARG`]; the
/// uninstaller does it too).
pub fn unregister_hklm() -> Result<(), String> {
    for subkey in [format!(r"{PROVIDERS}\{CLSID_TEXT}"), format!(r"{CLASSES}\{CLSID_TEXT}")] {
        // SAFETY: deleting a key by name.
        let status = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(subkey.as_str())) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND {
            return Err(format!("RegDeleteTreeW({subkey}) failed: {}", status.0));
        }
    }
    Ok(())
}

// ---- Who is signed in ----

/// The signed-in user's security identifier.
pub fn current_user_sid() -> Option<String> {
    let mut token = HANDLE::default();
    // SAFETY: this process's pseudo handle; the token is closed below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut buf = vec![0u8; 256];
    let mut len = 0u32;
    // SAFETY: `buf` has `buf.len()` bytes.
    let ok = unsafe {
        GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), buf.len() as u32, &mut len)
    };
    // SAFETY: opened above, closed once.
    let _ = unsafe { CloseHandle(token) };
    ok.ok()?;
    // SAFETY: GetTokenInformation filled `buf` with a TOKEN_USER.
    let user = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<TOKEN_USER>()) };
    let mut text = PWSTR::null();
    // SAFETY: the SID points into `buf`, still alive; `text` is freed below.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }.ok()?;
    // SAFETY: a valid string from ConvertSidToStringSidW.
    let sid = unsafe { text.to_string() }.ok();
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    sid
}

fn user_name(format: EXTENDED_NAME_FORMAT) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` and `len` describe a valid buffer.
    unsafe { GetUserNameExW(format, Some(PWSTR(buf.as_mut_ptr())), &mut len) }.then_some(())?;
    Some(String::from_utf16_lossy(&buf[..len as usize])).filter(|s| !s.is_empty())
}

/// The name the phone shows: the account's display name, or `PC\user`.
pub fn current_user_name() -> String {
    user_name(NameDisplay)
        .or_else(|| user_name(NameSamCompatible))
        .or_else(|| std::env::var("USERNAME").ok())
        .unwrap_or_default()
}

/// Checks `password` with Windows for the signed-in user. `Ok(false)` when
/// it's wrong; `Err` when Windows couldn't say (the lock screen will).
pub fn password_ok(password: &str) -> Result<bool, String> {
    let qualified = user_name(NameSamCompatible).ok_or("can't tell who's signed in")?;
    let (domain, user) = wire::split_qualified(&qualified);
    let (domain, user) = (HSTRING::from(domain), HSTRING::from(user));
    let mut wide: Vec<u16> = password.encode_utf16().chain(std::iter::once(0)).collect();
    let mut last = String::new();
    // A network check first (no session made); Microsoft accounts may only take
    // an interactive one.
    for kind in [LOGON32_LOGON_NETWORK, LOGON32_LOGON_INTERACTIVE] {
        let mut token = HANDLE::default();
        // SAFETY: NUL-terminated strings that outlive the call; the token is closed.
        let result = unsafe {
            LogonUserW(&user, &domain, PCWSTR(wide.as_ptr()), kind, LOGON32_PROVIDER_DEFAULT, &mut token)
        };
        match result {
            Ok(()) => {
                // SAFETY: a token LogonUserW returned.
                let _ = unsafe { CloseHandle(token) };
                wide.fill(0);
                return Ok(true);
            }
            Err(e) if e.code() == ERROR_LOGON_FAILURE.to_hresult() => {
                wide.fill(0);
                return Ok(false);
            }
            Err(e) => last = e.message(),
        }
    }
    wide.fill(0);
    Err(last)
}

// ---- The pipe the credential provider asks on ----

fn this_session() -> Option<u32> {
    let mut session = 0;
    // SAFETY: valid out pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.ok()?;
    Some(session)
}

fn session_locked(session: u32) -> bool {
    let mut info = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: Windows allocates `info`, freed with WTSFreeMemory below.
    if unsafe {
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            session,
            WTSSessionInfoEx,
            &mut info,
            &mut len,
        )
    }
    .is_err()
    {
        return false;
    }
    let locked = (len as usize) >= std::mem::size_of::<WTSINFOEXW>() && {
        // SAFETY: Windows filled `info` with a WTSINFOEXW of at least that size.
        let ex = unsafe { std::ptr::read_unaligned(info.0.cast::<WTSINFOEXW>()) };
        // SAFETY: level 1 is the only level Windows returns.
        ex.Level == 1 && unsafe { ex.Data.WTSInfoExLevel1.SessionFlags } == SESSION_LOCKED
    };
    // SAFETY: allocated by WTSQuerySessionInformationW.
    unsafe { WTSFreeMemory(info.0.cast()) };
    locked
}

/// Whether the pipe's client is LogonUI in `session`: running as SYSTEM (as
/// the pipe's ACL already demands), in this session, from `logonui.exe`. This
/// app runs as the user, so it can't open LogonUI's process to read its path;
/// it reads the client's token while impersonating it at identification level
/// (all the credential provider allows) instead. Call after reading from the
/// pipe: Windows impersonates a client only once it has written.
fn is_logonui(pipe: HANDLE, pid: u32, session: u32) -> bool {
    // From the pipe, not the process: asking for a SYSTEM process's session
    // needs a handle to it, which this app can't have.
    let mut their_session = u32::MAX;
    // SAFETY: a connected server handle and a valid out pointer.
    if unsafe { GetNamedPipeClientSessionId(pipe, &mut their_session) }.is_err() || their_session != session {
        tracing::warn!("unlock pipe: the caller is in another session");
        return false;
    }
    if !client_is_system(pipe) {
        tracing::warn!("unlock pipe: the caller isn't SYSTEM");
        return false;
    }
    if !process_name(pid).is_some_and(|name| name.eq_ignore_ascii_case("logonui.exe")) {
        tracing::warn!("unlock pipe: the caller isn't LogonUI");
        return false;
    }
    true
}

/// Whether the pipe's client runs as SYSTEM.
fn client_is_system(pipe: HANDLE) -> bool {
    // SAFETY: a connected pipe the client has written to; reverted below.
    if unsafe { ImpersonateNamedPipeClient(pipe) }.is_err() {
        return false;
    }
    let mut token = HANDLE::default();
    // SAFETY: this thread's impersonation token, read as the app itself.
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) };
    // SAFETY: ends the impersonation started above, whatever happened.
    let reverted = unsafe { RevertToSelf() };
    if reverted.is_err() {
        // Never carry on as someone else.
        std::process::abort();
    }
    if opened.is_err() {
        return false;
    }
    let mut buf = [0u8; 256];
    let mut len = 0u32;
    // SAFETY: `buf` is large enough for a TOKEN_USER and its SID.
    let read = unsafe {
        GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), buf.len() as u32, &mut len)
    };
    // SAFETY: opened above.
    let _ = unsafe { CloseHandle(token) };
    if read.is_err() {
        return false;
    }
    // SAFETY: Windows filled `buf` with a TOKEN_USER.
    let user = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<TOKEN_USER>()) };
    // SAFETY: the SID points into `buf`, still alive.
    unsafe { IsWellKnownSid(user.User.Sid, WinLocalSystemSid) }.as_bool()
}

/// The file name of process `pid`'s program, without opening the process.
fn process_name(pid: u32) -> Option<String> {
    // SAFETY: a snapshot handle, closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut entry =
        PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut name = None;
    // SAFETY: `entry` is a valid, sized PROCESSENTRY32W.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32ProcessID == pid {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            name = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
            break;
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: opened above.
    let _ = unsafe { CloseHandle(snapshot) };
    name
}

static SERVING: AtomicBool = AtomicBool::new(false);

/// Starts serving the unlock pipe if the credential provider is registered and
/// it isn't served yet.
pub fn serve() {
    if !is_registered() || SERVING.swap(true, Ordering::SeqCst) {
        return;
    }
    let started = std::thread::Builder::new().name("unlock-pipe".into()).spawn(|| {
        if let Err(e) = run_pipe() {
            tracing::warn!("unlock pipe stopped: {e}");
        }
        SERVING.store(false, Ordering::SeqCst);
    });
    if started.is_err() {
        SERVING.store(false, Ordering::SeqCst);
    }
}

struct Pipe(HANDLE);

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: owned here, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn run_pipe() -> Result<(), String> {
    let session = this_session().ok_or("no session")?;
    let user = current_user_sid().ok_or("no user")?;
    // Only SYSTEM (LogonUI) may open it: never the user's own processes, so
    // nothing running as the user can ask for the password.
    let sddl = HSTRING::from("D:P(A;;GA;;;SY)");
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `descriptor` is freed with LocalFree below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut descriptor, None)
    }
    .map_err(|e| e.message())?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = HSTRING::from(wire::pipe_name(session));
    // First instance only: if anything else made a pipe by this name, don't
    // share it. One instance: one unlock at a time.
    // SAFETY: `attributes` and `name` outlive the call; the handle is owned by `Pipe`.
    let pipe = unsafe {
        CreateNamedPipeW(
            &name,
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            4096,
            4096,
            0,
            Some(&attributes),
        )
    };
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    if pipe.is_invalid() {
        return Err("can't create the unlock pipe (another one exists?)".into());
    }
    let pipe = Pipe(pipe);
    loop {
        // SAFETY: a valid server handle; blocks until a client connects.
        // A client that connected before this call is ERROR_PIPE_CONNECTED: fine.
        if let Err(e) = unsafe { ConnectNamedPipe(pipe.0, None) }
            && e.code() != ERROR_PIPE_CONNECTED.to_hresult()
        {
            return Err(format!("ConnectNamedPipe failed: {}", e.message()));
        }
        serve_one(pipe.0, session, &user);
        // SAFETY: valid handle; lets the client read before disconnecting.
        unsafe {
            let _ = FlushFileBuffers(pipe.0);
            let _ = DisconnectNamedPipe(pipe.0);
        }
    }
}

fn serve_one(pipe: HANDLE, session: u32, user: &str) {
    let mut request = [0u8; 256];
    let mut read = 0u32;
    // SAFETY: `request` is a valid buffer.
    if unsafe { ReadFile(pipe, Some(&mut request), Some(&mut read), None) }.is_err() {
        return;
    }
    let mut client = 0u32;
    // SAFETY: a connected server handle.
    if unsafe { GetNamedPipeClientProcessId(pipe, &mut client) }.is_err()
        || !is_logonui(pipe, client, session)
    {
        tracing::warn!("unlock pipe: refused a caller that isn't this session's LogonUI");
        return;
    }
    if let Some(rejected) = wire::parse_rejected(&request[..read as usize]) {
        if rejected == user {
            forget_rejected(user);
        }
        return;
    }
    let (status, mut password) = match wire::parse_request(&request[..read as usize]) {
        Some(asked) if asked == user && session_locked(session) => answer(user),
        Some(_) => (wire::Status::NotEnrolled, Vec::new()),
        None => (wire::Status::Failed, Vec::new()),
    };
    // The phone takes a while: if the session was unlocked some other way
    // meanwhile, the password isn't needed and isn't handed out.
    let (status, mut password) = if status == wire::Status::Ok && !session_locked(session) {
        wire::wipe(&mut password);
        (wire::Status::Failed, Vec::new())
    } else {
        (status, password)
    };
    let mut reply = wire::answer(status, Some(&password));
    wire::wipe(&mut password);
    let mut written = 0u32;
    // SAFETY: `reply` is a valid buffer.
    let _ = unsafe { WriteFile(pipe, Some(&reply), Some(&mut written), None) };
    wire::wipe(&mut reply);
}

/// Windows refused the password a phone gave for `user`: it was changed since
/// setting up. Forgets it on every phone, so the lock screen stops offering it,
/// and says how to set it up again.
fn forget_rejected(user: &str) {
    let Some(node) = crate::core_host::node() else { return };
    let phones: Vec<(DeviceId, String)> = node
        .paired_devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| node.unlock_users(d.id).is_ok_and(|users| users.iter().any(|(sid, _)| sid == user)))
        .map(|d| (d.id, d.info.name.clone()))
        .collect();
    let user = user.to_owned();
    crate::core_host::host().runtime.spawn(async move {
        for (phone, name) in phones {
            if let Err(e) = node.unlock_forget(phone, Some(&user)).await {
                tracing::warn!(error = %e, "couldn't forget an out-of-date unlock password");
                continue;
            }
            tracing::info!("Windows refused the unlock password; it was forgotten");
            crate::win::toast::show(crate::win::toast::Toast {
                device: phone.to_string(),
                key: "unlock-rejected".into(),
                title: crate::i18n::tr!("Unlock", "Set up unlocking with %1 again", name),
                body: crate::i18n::tr!(
                    "Unlock",
                    "Windows didn't accept the password saved for it, so it was probably changed. Set it up again under Unlock this PC with the phone."
                ),
                attribution: "NectarLink".into(),
                icon: None,
                image: None,
                actions: Vec::new(),
                reply: None,
                silent: false,
                progress: None,
                call: false,
            });
        }
    });
}

/// Asks the phone `user` is enrolled with for the password.
fn answer(user: &str) -> (wire::Status, Vec<u8>) {
    let Some(node) = crate::core_host::node() else { return (wire::Status::Failed, Vec::new()) };
    let phones: Vec<DeviceId> = node
        .paired_devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| node.unlock_users(d.id).is_ok_and(|users| users.iter().any(|(sid, _)| sid == user)))
        .map(|d| d.id)
        .collect();
    if phones.is_empty() {
        return (wire::Status::NotEnrolled, Vec::new());
    }
    let mut last = wire::Status::PhoneAway;
    for phone in phones {
        let asked = crate::core_host::host().runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(35), node.unlock_password(phone, user)).await
        });
        match asked {
            Ok(Ok(password)) => return (wire::Status::Ok, password.expose().to_vec()),
            Ok(Err(Error::Offline)) => last = wire::Status::PhoneAway,
            Ok(Err(Error::NotFound)) => last = wire::Status::NotEnrolled,
            Ok(Err(Error::Denied | Error::Timeout)) | Err(_) => return (wire::Status::Declined, Vec::new()),
            Ok(Err(_)) => last = wire::Status::Failed,
        }
    }
    (last, Vec::new())
}

// ---- Enrolling ----

/// How setting up with a phone ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enrolled {
    Done,
    WrongPassword,
    Declined,
    PhoneAway,
    /// The phone has it off, or can't keep keys behind a fingerprint.
    PhoneCant,
    Failed,
}

/// Sets up the signed-in Windows user to unlock with `phone`: checks the
/// password with Windows, then the phone's owner agrees on the phone.
pub async fn enroll(phone: DeviceId, password: String) -> Enrolled {
    let mut bytes = password.into_bytes();
    let copy = bytes.clone();
    let checked = tokio::task::spawn_blocking(move || {
        let mut copy = copy;
        let result = std::str::from_utf8(&copy).map_err(|e| e.to_string()).and_then(password_ok);
        wire::wipe(&mut copy);
        result
    })
    .await;
    if matches!(checked, Ok(Ok(false))) {
        wire::wipe(&mut bytes);
        return Enrolled::WrongPassword;
    }
    if let Ok(Err(e)) = checked {
        tracing::warn!("couldn't check the Windows password here: {e}");
    }
    let (Some(node), Some(sid)) = (crate::core_host::node(), current_user_sid()) else {
        wire::wipe(&mut bytes);
        return Enrolled::Failed;
    };
    let secret = Secret::new(bytes.clone());
    wire::wipe(&mut bytes);
    let name: String = current_user_name().chars().filter(|c| !c.is_control()).take(60).collect();
    match node.unlock_enroll(phone, &sid, if name.is_empty() { "Windows" } else { &name }, secret).await {
        Ok(()) => Enrolled::Done,
        Err(Error::Denied) => Enrolled::Declined,
        Err(Error::Offline | Error::Timeout) => Enrolled::PhoneAway,
        Err(Error::Unsupported) => Enrolled::PhoneCant,
        Err(_) => Enrolled::Failed,
    }
}

/// Whether the signed-in user is set up to unlock with `phone`.
pub fn enrolled_with(phone: DeviceId) -> bool {
    let (Some(node), Some(sid)) = (crate::core_host::node(), current_user_sid()) else { return false };
    node.unlock_users(phone).is_ok_and(|users| users.iter().any(|(s, _)| *s == sid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_program_files_counts_as_protected() {
        let exe = std::env::current_exe().unwrap();
        // Tests run from the build folder, which any user can write.
        assert!(!protected(&exe));
        assert!(!protected(Path::new(r"C:\Users\Public\nectarlink_credprov.dll")));
    }

    #[test]
    fn the_signed_in_user_has_a_sid() {
        let sid = current_user_sid().unwrap();
        assert!(wire::user_ok(&sid), "{sid}");
        assert!(!current_user_name().is_empty());
    }

    // No test tries a wrong password: each failed check counts toward Windows'
    // account lockout on the PC running the tests.
}
