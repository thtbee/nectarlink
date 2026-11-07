// SPDX-License-Identifier: GPL-3.0-or-later
//! Opens the Windows Share sheet for files. The sheet belongs to a window, so
//! [`show`] has to run on Qt's thread.

use std::{cell::RefCell, path::PathBuf};

use windows::{
    ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager},
    Foundation::TypedEventHandler,
    Storage::{IStorageItem, StorageFile},
    Win32::{
        Foundation::{HWND, LPARAM},
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::GetActiveWindow,
            Shell::IDataTransferManagerInterop,
            WindowsAndMessaging::{EnumThreadWindows, GetWindowTextLengthW, IsWindowVisible},
        },
    },
    core::{BOOL, HSTRING, Interface, Ref, factory},
};
use windows_collections::IIterable;

thread_local! {
    /// The handler serving the last sheet, removed when the next one opens.
    static LAST: RefCell<Option<(DataTransferManager, i64)>> = const { RefCell::new(None) };
}

/// Shows the sheet for `paths`, which have to stay on disk until the app that's
/// picked in it has read them.
pub fn show(paths: &[PathBuf]) -> Result<(), String> {
    let window = owner_window().ok_or("no window to open it from")?;
    let files = paths.to_vec();
    let title = HSTRING::from(share_title(paths));

    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>().map_err(|e| e.to_string())?;
    // SAFETY: `window` is a live top-level window of this thread.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(window) }.map_err(|e| e.to_string())?;
    LAST.with_borrow_mut(|last| {
        if let Some((old, token)) = last.take() {
            let _ = old.RemoveDataRequested(token);
        }
    });
    let handler = TypedEventHandler::new(
        move |_: Ref<'_, DataTransferManager>, args: Ref<'_, DataRequestedEventArgs>| {
            let data = args.ok()?.Request()?.Data()?;
            data.Properties()?.SetTitle(&title)?;
            data.SetStorageItems(&storage_items(&files)?, true)
        },
    );
    let token = manager.DataRequested(&handler).map_err(|e| e.to_string())?;
    LAST.with_borrow_mut(|last| *last = Some((manager, token)));
    // SAFETY: as above.
    unsafe { interop.ShowShareUIForWindow(window) }.map_err(|e| e.to_string())
}

/// The window the sheet opens over: the active one, or else any of ours on
/// screen, since the user may have clicked elsewhere during a long download.
fn owner_window() -> Option<HWND> {
    // SAFETY: only reads the calling thread's state.
    let active = unsafe { GetActiveWindow() };
    if !active.is_invalid() {
        return Some(active);
    }
    unsafe extern "system" fn visit(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the `Option<HWND>` below, alive for the whole enumeration.
        unsafe {
            if IsWindowVisible(window).as_bool() && GetWindowTextLengthW(window) > 0 {
                *(found.0 as *mut Option<HWND>) = Some(window);
                return false.into();
            }
        }
        true.into()
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback only runs during this call. It returns FALSE on a match,
    // which makes this report a failure, so the result is `found`.
    let _ = unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(visit), LPARAM(&raw mut found as isize)) };
    found
}

/// Files in a local folder, so this is quick even though it waits on Qt's thread.
fn storage_items(paths: &[PathBuf]) -> windows::core::Result<IIterable<IStorageItem>> {
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join()?;
        items.push(Some(file.cast::<IStorageItem>()?));
    }
    Ok(IIterable::from(items))
}

/// A title is required, or the sheet shows an error instead of the apps.
fn share_title(paths: &[PathBuf]) -> String {
    match paths {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        many => format!("{} files", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_one_file_or_counts_many() {
        assert_eq!(share_title(&[PathBuf::from(r"C:\t\IMG_1.jpg")]), "IMG_1.jpg");
        assert_eq!(share_title(&[PathBuf::from("a"), PathBuf::from("b")]), "2 files");
    }
}
