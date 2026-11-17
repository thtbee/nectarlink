// SPDX-License-Identifier: GPL-3.0-or-later
//! Starting a program with administrator rights (the UAC prompt) and
//! following it (the installer and uninstaller, `crate::setup`).

use std::path::Path;

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        System::Threading::{GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject},
        UI::{
            Shell::{
                SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
                ShellExecuteExW,
            },
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

pub enum Error {
    Declined,
    Other(String),
}

pub struct Process(HANDLE);

// SAFETY: a process handle may be waited on from any thread.
unsafe impl Send for Process {}

impl Process {
    /// The exit code once it has exited.
    pub fn exit_code(&self) -> Option<u32> {
        if self.0.is_invalid() {
            return Some(0);
        }
        // SAFETY: a valid process handle, owned by this value.
        unsafe {
            if WaitForSingleObject(self.0, 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 1u32;
            GetExitCodeProcess(self.0, &mut code).ok()?;
            Some(code)
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: closed once, here.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Starts `program` with this process's own rights, `params` passed as written.
fn start_directly(program: &Path, params: &str) -> Result<Process, Error> {
    use std::os::windows::{io::IntoRawHandle, process::CommandExt};
    std::process::Command::new(program)
        .raw_arg(params)
        .spawn()
        .map(|child| Process(HANDLE(child.into_raw_handle())))
        .map_err(|e| Error::Other(e.to_string()))
}

/// Whether this process already has administrator rights.
pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: the current process's pseudo handle; the token is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: `elevation` is the size passed, and the token is valid.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    }
    .is_ok();
    // SAFETY: opened above, closed once.
    let _ = unsafe { CloseHandle(token) };
    ok && elevation.TokenIsElevated != 0
}

/// Starts `program` with administrator rights (the UAC prompt).
///
/// Already elevated, it's started directly: some Windows installs have lost
/// the "Run as administrator" verb for programs, so asking again fails there,
/// and starting setup from an administrator prompt is the way round it (#364).
pub fn start(program: &Path, params: &str) -> Result<Process, Error> {
    if is_elevated() {
        return start_directly(program, params);
    }
    // ShellExecuteEx may hand the verb to shell extensions, which need COM on
    // this thread; Microsoft says to set it up first. Without it, some PCs
    // answer "no app is associated" for runas on an .exe (#364).
    // SAFETY: paired with CoUninitialize below when it succeeded.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let result = shell_runas(program, params);
    if com {
        // SAFETY: balances the successful CoInitializeEx above, on this thread.
        unsafe { CoUninitialize() };
    }
    result
}

fn shell_runas(program: &Path, params: &str) -> Result<Process, Error> {
    let file = HSTRING::from(program.as_os_str());
    let parameters = HSTRING::from(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // No shell error box: its OK would come back as "cancelled" and read
        // as the person saying no, when Windows couldn't ask at all (#364).
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the structure and its strings outlive the call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(Process(info.hProcess)),
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err(Error::Declined),
        Err(e) => Err(Error::Other(e.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_started_directly_gets_its_arguments_as_written_and_reports_its_exit() {
        let cmd = std::env::var_os("ComSpec").map_or_else(|| "cmd.exe".into(), std::path::PathBuf::from);
        let Ok(process) = start_directly(&cmd, "/d /c exit 7") else { panic!("cmd didn't start") };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let code = loop {
            if let Some(code) = process.exit_code() {
                break code;
            }
            assert!(std::time::Instant::now() < deadline, "cmd didn't exit");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(code, 7);
    }
}
