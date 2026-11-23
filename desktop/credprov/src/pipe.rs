// SPDX-License-Identifier: GPL-3.0-or-later
//! The credential provider's side of the unlock pipe: ask the signed-in user's
//! NectarLink for the password, and wait at most [`wire::ANSWER_TIMEOUT_MS`]
//! so the lock screen never hangs.
//!
//! It only talks to the NectarLink installed next to this DLL (checked by the
//! pipe server's process image), run by the account signed in to this session
//! in this session (pipe names are machine-wide, so another account could
//! otherwise serve this session's pipe), and connects at identification level,
//! so whatever holds the pipe can't act as SYSTEM.

#![allow(unsafe_code)]

use std::ffi::c_void;

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, HMODULE, LocalFree, WAIT_OBJECT_0,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING,
            ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
        },
        System::{
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            LibraryLoader::{
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                GetModuleFileNameW, GetModuleHandleExW,
            },
            Pipes::{
                GetNamedPipeServerProcessId, PIPE_READMODE_MESSAGE, SetNamedPipeHandleState, WaitNamedPipeW,
            },
            RemoteDesktop::ProcessIdToSessionId,
            Threading::{
                CreateEventW, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
            },
        },
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

use crate::wire::{self, Status};

/// Why asking didn't get an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskError {
    /// NectarLink isn't running in this session (or isn't ours).
    NotRunning,
    /// It didn't answer in time.
    Timeout,
    /// The pipe broke, or the answer wasn't one.
    Broken,
}

/// The executable the pipe's server must be: `nectarlink-desktop.exe` in the
/// folder this DLL was loaded from.
fn expected_server() -> Option<String> {
    let mut module = HMODULE::default();
    let anchor = expected_server as *const c_void;
    // SAFETY: `anchor` is an address inside this DLL; the handle isn't kept.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(anchor.cast()),
            &mut module,
        )
        .ok()?;
    }
    let mut path = [0u16; 1024];
    // SAFETY: `path` is a valid buffer of its length.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut path) } as usize;
    if len == 0 || len >= path.len() {
        return None;
    }
    let dll = std::path::PathBuf::from(String::from_utf16_lossy(&path[..len]));
    Some(dll.with_file_name("nectarlink-desktop.exe").to_string_lossy().to_lowercase())
}

fn process_image(pid: u32) -> Option<String> {
    // SAFETY: plain query access to a process; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut path = [0u16; 1024];
    let mut len = path.len() as u32;
    // SAFETY: `path` and `len` describe a valid buffer.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len)
    };
    // SAFETY: `process` was opened above.
    let _ = unsafe { CloseHandle(process) };
    ok.ok()?;
    Some(String::from_utf16_lossy(&path[..len as usize]).to_lowercase())
}

/// The security identifier of the account process `pid` runs as.
fn process_user(pid: u32) -> Option<String> {
    // SAFETY: plain query access to a process; the handle is owned by `Handle`.
    let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let mut token = HANDLE::default();
    // SAFETY: `process` is valid; `token` is closed by `Handle`.
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) }.ok()?;
    let token = Handle(token);
    // TOKEN_USER plus the SID it points to; u64s keep it aligned.
    let mut buf = [0u64; 64];
    let mut len = 0u32;
    // SAFETY: `buf` is a writable buffer of the size given.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            std::mem::size_of_val(&buf) as u32,
            &mut len,
        )
    }
    .ok()?;
    // SAFETY: Windows wrote a TOKEN_USER at the start of `buf`, aligned for it.
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
    let mut text = PWSTR::null();
    // SAFETY: the SID points into `buf`, alive here; `text` is freed below.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }.ok()?;
    // SAFETY: a valid string from ConvertSidToStringSidW.
    let sid = unsafe { text.to_string() }.ok();
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    sid
}

/// Whether the pipe's server, process `pid`, is the NectarLink next to this DLL,
/// run by `user` in `session`.
fn trusted_server(pid: u32, session: u32, user: &str) -> bool {
    let mut their_session = u32::MAX;
    // SAFETY: valid out pointer.
    let in_session =
        unsafe { ProcessIdToSessionId(pid, &mut their_session) }.is_ok() && their_session == session;
    in_session
        && matches!((process_image(pid), expected_server()), (Some(actual), Some(expected)) if actual == expected)
        && process_user(pid).is_some_and(|sid| sid.eq_ignore_ascii_case(user))
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned here and closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Runs one overlapped read or write on `pipe`, waiting at most `timeout_ms`.
fn overlapped(
    pipe: HANDLE,
    timeout_ms: u32,
    start: impl FnOnce(&mut OVERLAPPED) -> windows::core::Result<()>,
) -> Result<u32, AskError> {
    // SAFETY: an unnamed manual-reset event, closed by `Handle`.
    let event = Handle(unsafe { CreateEventW(None, true, false, None) }.map_err(|_| AskError::Broken)?);
    let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
    if let Err(e) = start(&mut ov) {
        // ERROR_IO_PENDING is the normal "started" answer.
        if e.code() != windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() {
            return Err(AskError::Broken);
        }
    }
    // SAFETY: `event` is valid for the wait.
    if unsafe { WaitForSingleObject(event.0, timeout_ms) } != WAIT_OBJECT_0 {
        // SAFETY: cancel this one operation, then wait for it to really stop
        // before `ov` and its buffer go away.
        unsafe {
            let _ = CancelIoEx(pipe, Some(&ov));
            let mut n = 0;
            let _ = GetOverlappedResult(pipe, &ov, &mut n, true);
        }
        return Err(AskError::Timeout);
    }
    let mut n = 0;
    // SAFETY: the operation finished; `ov` is the one it used.
    unsafe { GetOverlappedResult(pipe, &ov, &mut n, false) }.map_err(|_| AskError::Broken)?;
    Ok(n)
}

/// Whether this session's NectarLink is serving the unlock pipe (busy counts).
pub fn server_present() -> bool {
    let mut session = 0u32;
    // SAFETY: valid out pointer.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.is_err() {
        return false;
    }
    let name = HSTRING::from(wire::pipe_name(session));
    // SAFETY: `name` outlives the call.
    if unsafe { WaitNamedPipeW(&name, 1) }.as_bool() {
        return true;
    }
    // Busy or timed out: it's there. Not found: it isn't.
    let error = windows::core::Error::from_thread();
    error.code() != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult()
}

/// Asks this session's NectarLink to unlock `user` (a security identifier). The
/// returned password is UTF-8: wipe it after use.
pub fn ask(user: &str, from_phone: bool) -> Result<(Status, Vec<u8>), AskError> {
    let request = if from_phone { wire::phone_request(user) } else { wire::request(user) };
    exchange(&request.ok_or(AskError::Broken)?, wire::ANSWER_TIMEOUT_MS, 2_000)
}

pub fn phone_request_waiting() -> Result<bool, AskError> {
    exchange(wire::PEEK_PHONE_REQUEST, 500, 20).map(|(status, mut body)| {
        wire::wipe(&mut body);
        status == Status::Ready
    })
}

fn exchange(request: &[u8], timeout: u32, connect_timeout: u32) -> Result<(Status, Vec<u8>), AskError> {
    let pipe = connect(connect_timeout)?;
    // SAFETY (both closures): `request` and `answer` outlive the operations,
    // which `overlapped` waits for (or cancels and waits for) before returning.
    overlapped(pipe.0, timeout.min(5_000), |ov| unsafe { WriteFile(pipe.0, Some(request), None, Some(ov)) })?;
    let mut answer = vec![0u8; 1 + 4 + wire::MAX_PASSWORD];
    let read =
        overlapped(pipe.0, timeout, |ov| unsafe { ReadFile(pipe.0, Some(&mut answer), None, Some(ov)) });
    let result = match read {
        Ok(n) => wire::parse_answer(&answer[..n as usize]).ok_or(AskError::Broken),
        Err(e) => Err(e),
    };
    wire::wipe(&mut answer);
    result
}

/// Tells this session's NectarLink that Windows refused `user`'s password from
/// the phone, so it stops offering one that no longer works.
pub fn tell_rejected(user: &str) -> Result<(), AskError> {
    let notice = wire::rejected(user).ok_or(AskError::Broken)?;
    let pipe = connect(2_000)?;
    // SAFETY: `notice` outlives the operation, which `overlapped` waits for.
    overlapped(pipe.0, 5_000, |ov| unsafe { WriteFile(pipe.0, Some(&notice), None, Some(ov)) })?;
    Ok(())
}

/// Opens this session's unlock pipe, served by the NectarLink installed next to
/// this DLL and nothing else.
fn connect(wait: u32) -> Result<Handle, AskError> {
    let mut session = 0u32;
    // SAFETY: valid out pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.map_err(|_| AskError::NotRunning)?;
    let name = HSTRING::from(wire::pipe_name(session));
    // SAFETY: `name` outlives the calls.
    unsafe {
        if !WaitNamedPipeW(&name, wait).as_bool() {
            return Err(AskError::NotRunning);
        }
    }
    // SAFETY: opening a pipe by name; the handle is closed by `Handle`.
    let pipe = Handle(
        unsafe {
            CreateFileW(
                &name,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(
                    FILE_FLAG_OVERLAPPED.0 | SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0,
                ),
                None,
            )
        }
        .map_err(|_| AskError::NotRunning)?,
    );
    // Only NectarLink, installed next to this DLL and run by this session's
    // account in this session, gets asked.
    let mut server = 0u32;
    // SAFETY: `pipe` is a connected pipe client handle.
    unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server) }.map_err(|_| AskError::NotRunning)?;
    let user = crate::session::signed_in_sid().ok_or(AskError::NotRunning)?;
    if !trusted_server(server, session, &user) {
        return Err(AskError::NotRunning);
    }
    let mode = PIPE_READMODE_MESSAGE;
    // SAFETY: `pipe` is valid; `mode` outlives the call.
    unsafe { SetNamedPipeHandleState(pipe.0, Some(&mode), None, None) }.map_err(|_| AskError::Broken)?;
    Ok(pipe)
}
