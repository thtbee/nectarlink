// SPDX-License-Identifier: GPL-3.0-or-later
//! The credential provider's side of the unlock pipe: ask the signed-in user's
//! NectarLink for the password, and wait at most [`wire::ANSWER_TIMEOUT_MS`]
//! so the lock screen never hangs.
//!
//! It only talks to the NectarLink installed next to this DLL (checked by the
//! pipe server's process image), and connects at identification level, so
//! whatever holds the pipe can't act as SYSTEM.

#![allow(unsafe_code)]

use std::ffi::c_void;

use windows::{
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HMODULE, WAIT_OBJECT_0},
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
                CreateEventW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
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

/// Asks this session's NectarLink to unlock `user` (a security identifier). The
/// returned password is UTF-8: wipe it after use.
pub fn ask(user: &str) -> Result<(Status, Vec<u8>), AskError> {
    let request = wire::request(user).ok_or(AskError::Broken)?;
    let mut session = 0u32;
    // SAFETY: valid out pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.map_err(|_| AskError::NotRunning)?;
    let name = HSTRING::from(wire::pipe_name(session));
    // SAFETY: `name` outlives the calls.
    unsafe {
        if !WaitNamedPipeW(&name, 2_000).as_bool() {
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
    // Only NectarLink, installed next to this DLL, gets asked.
    let mut server = 0u32;
    // SAFETY: `pipe` is a connected pipe client handle.
    unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server) }.map_err(|_| AskError::NotRunning)?;
    match (process_image(server), expected_server()) {
        (Some(actual), Some(expected)) if actual == expected => {}
        _ => return Err(AskError::NotRunning),
    }
    let mode = PIPE_READMODE_MESSAGE;
    // SAFETY: `pipe` is valid; `mode` outlives the call.
    unsafe { SetNamedPipeHandleState(pipe.0, Some(&mode), None, None) }.map_err(|_| AskError::Broken)?;

    // SAFETY (both closures): `request` and `answer` outlive the operations,
    // which `overlapped` waits for (or cancels and waits for) before returning.
    overlapped(pipe.0, 5_000, |ov| unsafe { WriteFile(pipe.0, Some(&request), None, Some(ov)) })?;
    let mut answer = vec![0u8; 1 + 4 + wire::MAX_PASSWORD];
    let read = overlapped(pipe.0, wire::ANSWER_TIMEOUT_MS, |ov| unsafe {
        ReadFile(pipe.0, Some(&mut answer), None, Some(ov))
    });
    let result = match read {
        Ok(n) => wire::parse_answer(&answer[..n as usize]).ok_or(AskError::Broken),
        Err(e) => Err(e),
    };
    wire::wipe(&mut answer);
    result
}
