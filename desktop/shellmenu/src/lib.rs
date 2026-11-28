// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink_shellmenu`: "Send with NectarLink" in the first level of File
//! Explorer's right-click menu (#451). Windows 11 only shows apps there through
//! an `IExplorerCommand` in a package, so this DLL is declared in the app's
//! sparse package (`desktop/installer/AppxManifest.xml`) and Explorer loads it.
//!
//! One phone: the entry sends to it. More: the entry opens a list of them.
//! Either way it starts the app next to this DLL with `--send-to <device>
//! <files…>`, as the "Send to" shortcuts do, and the app sends and follows the
//! transfer. The phones and titles come from the registry ([`menu`]), which
//! the app keeps in step; with none there, the entry is hidden.

#![allow(linker_messages)]

pub mod menu;

#[cfg(windows)]
pub use self::com::*;

#[cfg(windows)]
#[allow(unsafe_code)]
mod com {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        path::PathBuf,
        sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    };

    use windows::{
        Win32::{
            Foundation::{
                CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER,
                E_UNEXPECTED, ERROR_SUCCESS, HMODULE, S_FALSE, S_OK,
            },
            System::{
                Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl},
                LibraryLoader::{
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                    GetModuleFileNameW, GetModuleHandleExW,
                },
                Registry::{HKEY_CURRENT_USER, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ, RegGetValueW},
            },
            UI::Shell::{
                ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand,
                IEnumExplorerCommand_Impl, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray,
                SHStrDupW, SIGDN_FILESYSPATH,
            },
        },
        core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement},
    };

    use crate::menu::{self, Phone};

    /// CLSID of the menu entry, as the package manifest declares it.
    pub const CLSID_NECTARLINK_SEND_MENU: GUID = GUID::from_u128(0x728fb82f_ebaf_4a6f_b8e2_64008ed454bf);

    /// The app the entry starts, next to this DLL.
    const APP_EXE: &str = "nectarlink-desktop.exe";

    /// Objects from this DLL still alive, for [`DllCanUnloadNow`].
    static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

    /// Counts one live object while it exists.
    #[derive(Debug)]
    struct ObjectGuard;

    impl ObjectGuard {
        fn new() -> ObjectGuard {
            ACTIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
            ObjectGuard
        }
    }

    impl Drop for ObjectGuard {
        fn drop(&mut self) {
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// What the menu shows right now.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct Menu {
        title: String,
        phones: Vec<Phone>,
    }

    impl Menu {
        /// From the registry, as the app last wrote it.
        fn load() -> Menu {
            let phones = reg_value(menu::PHONES, true).map(|v| menu::decode(&v)).unwrap_or_default();
            let title = reg_value(menu::TITLE, false)
                .and_then(|v| v.into_iter().next())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Send with NectarLink".into());
            Menu { title, phones }
        }
    }

    /// A string (or the strings of a multi-string) under [`menu::KEY`].
    fn reg_value(name: &str, multi: bool) -> Option<Vec<String>> {
        let key = HSTRING::from(menu::KEY);
        let name = HSTRING::from(name);
        let flags = if multi { RRF_RT_REG_MULTI_SZ } else { RRF_RT_REG_SZ };
        let mut size = 0u32;
        // SAFETY: a size query: no buffer, `size` is a live u32.
        let status =
            unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, None, Some(&mut size)) };
        if status != ERROR_SUCCESS || size == 0 || size > 1 << 20 {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                &name,
                flags,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize / 2);
        Some(buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect())
    }

    /// The folder this DLL is in, where the app is.
    fn own_dir() -> Option<PathBuf> {
        let mut module = HMODULE::default();
        // SAFETY: any address in this DLL names it; the reference count is
        // left alone, and the handle is only used while the DLL is loaded.
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                PCWSTR(own_dir as *const u16),
                &mut module,
            )
            .ok()?;
        }
        let mut buf = vec![0u16; 32 * 1024];
        // SAFETY: `buf` is a live buffer of that length.
        let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
        if len == 0 || len >= buf.len() {
            return None;
        }
        PathBuf::from(String::from_utf16_lossy(&buf[..len])).parent().map(PathBuf::from)
    }

    /// A string Explorer frees with `CoTaskMemFree`.
    fn shell_string(text: &str) -> windows::core::Result<PWSTR> {
        // SAFETY: the string outlives the call, which copies it.
        unsafe { SHStrDupW(&HSTRING::from(text)) }
    }

    /// The file system paths of the chosen items; items that aren't files
    /// (in a library's view or a zip, say) are left out.
    fn chosen_paths(items: &IShellItemArray) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        // SAFETY: plain COM calls on a live array; each name is freed once read.
        unsafe {
            let count = items.GetCount().unwrap_or(0);
            for i in 0..count {
                let Ok(item) = items.GetItemAt(i) else { continue };
                let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
                if let Ok(path) = name.to_string() {
                    paths.push(PathBuf::from(path));
                }
                CoTaskMemFree(Some(name.0 as *const _));
            }
        }
        paths
    }

    /// The app's arguments for sending `paths` to `device`.
    fn send_args(device: &str, paths: &[PathBuf]) -> Vec<std::ffi::OsString> {
        let mut args = vec!["--send-to".into(), device.into()];
        args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
        args
    }

    /// Starts the app to send what's chosen to `device`.
    fn send(device: &str, items: Option<&IShellItemArray>) -> windows::core::Result<()> {
        let paths = items.map(chosen_paths).unwrap_or_default();
        if paths.is_empty() {
            return Ok(());
        }
        let exe = own_dir().ok_or(E_FAIL)?.join(APP_EXE);
        std::process::Command::new(&exe)
            .args(send_args(device, &paths))
            .spawn()
            .map(drop)
            .map_err(|_| E_FAIL.into())
    }

    /// The entry in the menu's first level.
    #[implement(IExplorerCommand)]
    struct Root {
        /// A fixed menu (tests); otherwise the registry's, read each time.
        fixed: Option<Menu>,
        _guard: ObjectGuard,
    }

    impl Root {
        fn menu(&self) -> Menu {
            self.fixed.clone().unwrap_or_else(Menu::load)
        }
    }

    impl IExplorerCommand_Impl for Root_Impl {
        fn GetTitle(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            let menu = self.menu();
            match menu.phones.as_slice() {
                [only] => shell_string(&only.title),
                _ => shell_string(&menu.title),
            }
        }

        fn GetIcon(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            let exe = own_dir().ok_or(E_FAIL)?.join(APP_EXE);
            shell_string(&format!("{},0", exe.display()))
        }

        fn GetToolTip(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            Err(E_NOTIMPL.into())
        }

        fn GetCanonicalName(&self) -> windows::core::Result<GUID> {
            Ok(CLSID_NECTARLINK_SEND_MENU)
        }

        fn GetState(&self, _items: Ref<'_, IShellItemArray>, _slow_ok: BOOL) -> windows::core::Result<u32> {
            // No phones paired (or Send to turned off in the app): no entry.
            Ok(if self.menu().phones.is_empty() { ECS_HIDDEN.0 as u32 } else { ECS_ENABLED.0 as u32 })
        }

        fn Invoke(
            &self,
            items: Ref<'_, IShellItemArray>,
            _bind: Ref<'_, IBindCtx>,
        ) -> windows::core::Result<()> {
            match self.menu().phones.as_slice() {
                [only] => send(&only.id, items.as_ref()),
                _ => Ok(()),
            }
        }

        fn GetFlags(&self) -> windows::core::Result<u32> {
            Ok(if self.menu().phones.len() > 1 { ECF_HASSUBCOMMANDS.0 as u32 } else { ECF_DEFAULT.0 as u32 })
        }

        fn EnumSubCommands(&self) -> windows::core::Result<IEnumExplorerCommand> {
            let phones = self.menu().phones;
            if phones.len() < 2 {
                return Err(E_NOTIMPL.into());
            }
            let commands = phones
                .into_iter()
                .map(|phone| PhoneCommand { phone, _guard: ObjectGuard::new() }.into())
                .collect();
            Ok(Commands { commands, next: AtomicUsize::new(0), _guard: ObjectGuard::new() }.into())
        }
    }

    /// A phone in the list the entry opens.
    #[implement(IExplorerCommand)]
    struct PhoneCommand {
        phone: Phone,
        _guard: ObjectGuard,
    }

    impl IExplorerCommand_Impl for PhoneCommand_Impl {
        fn GetTitle(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            shell_string(&self.phone.name)
        }

        fn GetIcon(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            Err(E_NOTIMPL.into())
        }

        fn GetToolTip(&self, _items: Ref<'_, IShellItemArray>) -> windows::core::Result<PWSTR> {
            shell_string(&self.phone.title)
        }

        fn GetCanonicalName(&self) -> windows::core::Result<GUID> {
            Err(E_NOTIMPL.into())
        }

        fn GetState(&self, _items: Ref<'_, IShellItemArray>, _slow_ok: BOOL) -> windows::core::Result<u32> {
            Ok(ECS_ENABLED.0 as u32)
        }

        fn Invoke(
            &self,
            items: Ref<'_, IShellItemArray>,
            _bind: Ref<'_, IBindCtx>,
        ) -> windows::core::Result<()> {
            send(&self.phone.id, items.as_ref())
        }

        fn GetFlags(&self) -> windows::core::Result<u32> {
            Ok(ECF_DEFAULT.0 as u32)
        }

        fn EnumSubCommands(&self) -> windows::core::Result<IEnumExplorerCommand> {
            Err(E_NOTIMPL.into())
        }
    }

    /// The list of phones, as Explorer reads it.
    #[implement(IEnumExplorerCommand)]
    struct Commands {
        commands: Vec<IExplorerCommand>,
        next: AtomicUsize,
        _guard: ObjectGuard,
    }

    impl IEnumExplorerCommand_Impl for Commands_Impl {
        fn Next(&self, wanted: u32, out: *mut Option<IExplorerCommand>, fetched: *mut u32) -> HRESULT {
            if out.is_null() || (wanted > 1 && fetched.is_null()) {
                return E_POINTER;
            }
            let start = self.next.load(Ordering::Relaxed).min(self.commands.len());
            let given = &self.commands[start..(start + wanted as usize).min(self.commands.len())];
            for (i, command) in given.iter().enumerate() {
                // SAFETY: Explorer passes room for `wanted` commands.
                unsafe { out.add(i).write(Some(command.clone())) };
            }
            self.next.store(start + given.len(), Ordering::Relaxed);
            if !fetched.is_null() {
                // SAFETY: checked non-null.
                unsafe { *fetched = given.len() as u32 };
            }
            if given.len() == wanted as usize { S_OK } else { S_FALSE }
        }

        fn Skip(&self, count: u32) -> windows::core::Result<()> {
            let next = self.next.load(Ordering::Relaxed) + count as usize;
            self.next.store(next.min(self.commands.len()), Ordering::Relaxed);
            if next <= self.commands.len() { Ok(()) } else { Err(S_FALSE.into()) }
        }

        fn Reset(&self) -> windows::core::Result<()> {
            self.next.store(0, Ordering::Relaxed);
            Ok(())
        }

        fn Clone(&self) -> windows::core::Result<IEnumExplorerCommand> {
            Ok(Commands {
                commands: self.commands.clone(),
                next: AtomicUsize::new(self.next.load(Ordering::Relaxed)),
                _guard: ObjectGuard::new(),
            }
            .into())
        }
    }

    #[implement(IClassFactory)]
    struct Factory {
        _guard: ObjectGuard,
    }

    impl IClassFactory_Impl for Factory_Impl {
        fn CreateInstance(
            &self,
            outer: Ref<'_, IUnknown>,
            riid: *const GUID,
            object: *mut *mut std::ffi::c_void,
        ) -> windows::core::Result<()> {
            if object.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: checked non-null.
            unsafe { *object = std::ptr::null_mut() };
            if !outer.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            if riid.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let command: IExplorerCommand = Root { fixed: None, _guard: ObjectGuard::new() }.into();
            // SAFETY: `riid` and `object` are valid per the checks above.
            unsafe { command.query(riid, object).ok() }
        }

        fn LockServer(&self, lock: BOOL) -> windows::core::Result<()> {
            if lock.as_bool() {
                ACTIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
            } else {
                ACTIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
            }
            Ok(())
        }
    }

    /// Standard COM entry point: the class factory for [`CLSID_NECTARLINK_SEND_MENU`].
    ///
    /// # Safety
    /// `rclsid`, `riid` and `ppv` must be valid as COM specifies.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn DllGetClassObject(
        rclsid: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            if ppv.is_null() {
                return E_POINTER;
            }
            // SAFETY: checked non-null.
            unsafe { *ppv = std::ptr::null_mut() };
            if rclsid.is_null() || riid.is_null() {
                return E_INVALIDARG;
            }
            // SAFETY: checked non-null.
            if unsafe { *rclsid } != CLSID_NECTARLINK_SEND_MENU {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = Factory { _guard: ObjectGuard::new() }.into();
            // SAFETY: `riid` and `ppv` are valid per the checks above.
            unsafe { factory.query(riid, ppv) }
        }))
        .unwrap_or(E_UNEXPECTED)
    }

    /// Standard COM entry point: whether nothing from this DLL is still alive.
    #[unsafe(no_mangle)]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        if ACTIVE_OBJECTS.load(Ordering::Relaxed) == 0 { S_OK } else { S_FALSE }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn phone(id: &str, name: &str) -> Phone {
            Phone { id: id.into(), name: name.into(), title: format!("Send to {name} with NectarLink") }
        }

        fn root(phones: Vec<Phone>) -> IExplorerCommand {
            Root {
                fixed: Some(Menu { title: "Send with NectarLink".into(), phones }),
                _guard: ObjectGuard::new(),
            }
            .into()
        }

        fn text(s: PWSTR) -> String {
            // SAFETY: a string from SHStrDupW, freed once read.
            unsafe {
                let t = s.to_string().unwrap();
                CoTaskMemFree(Some(s.0 as *const _));
                t
            }
        }

        #[test]
        fn the_dll_makes_the_menu_entry() {
            let mut raw = std::ptr::null_mut();
            // SAFETY: valid pointers to locals.
            let hr = unsafe { DllGetClassObject(&CLSID_NECTARLINK_SEND_MENU, &IClassFactory::IID, &mut raw) };
            assert_eq!(hr, S_OK);
            // SAFETY: `raw` is the IClassFactory just returned.
            let factory = unsafe { IClassFactory::from_raw(raw) };
            // SAFETY: plain COM calls.
            unsafe {
                let command: IExplorerCommand = factory.CreateInstance(None).unwrap();
                assert_eq!(command.GetCanonicalName().unwrap(), CLSID_NECTARLINK_SEND_MENU);
                assert!(text(command.GetIcon(None).unwrap()).ends_with("nectarlink-desktop.exe,0"));
            }
            let other = GUID::from_u128(1);
            let mut raw = std::ptr::null_mut();
            // SAFETY: valid pointers to locals.
            let hr = unsafe { DllGetClassObject(&other, &IClassFactory::IID, &mut raw) };
            assert_eq!(hr, CLASS_E_CLASSNOTAVAILABLE);
        }

        #[test]
        fn no_phones_no_entry() {
            // SAFETY: plain COM call.
            assert_eq!(unsafe { root(vec![]).GetState(None, false) }.unwrap(), ECS_HIDDEN.0 as u32);
        }

        #[test]
        fn one_phone_is_sent_to_straight_away() {
            let command = root(vec![phone("0a", "Pixel 8")]);
            // SAFETY: plain COM calls.
            unsafe {
                assert_eq!(command.GetState(None, false).unwrap(), ECS_ENABLED.0 as u32);
                assert_eq!(text(command.GetTitle(None).unwrap()), "Send to Pixel 8 with NectarLink");
                assert_eq!(command.GetFlags().unwrap(), ECF_DEFAULT.0 as u32);
                assert!(command.EnumSubCommands().is_err());
            }
        }

        #[test]
        fn more_phones_open_a_list_of_them() {
            let command = root(vec![phone("0a", "Pixel 8"), phone("0b", "Galaxy Tab")]);
            // SAFETY: plain COM calls; Next gets room for as many as it's asked.
            unsafe {
                assert_eq!(text(command.GetTitle(None).unwrap()), "Send with NectarLink");
                assert_eq!(command.GetFlags().unwrap(), ECF_HASSUBCOMMANDS.0 as u32);
                let list = command.EnumSubCommands().unwrap();
                let mut got: [Option<IExplorerCommand>; 3] = Default::default();
                let mut fetched = 0;
                assert_eq!(list.Next(&mut got, Some(&mut fetched)), S_FALSE);
                assert_eq!(fetched, 2);
                let names: Vec<String> =
                    got[..2].iter().map(|c| text(c.as_ref().unwrap().GetTitle(None).unwrap())).collect();
                assert_eq!(names, ["Pixel 8", "Galaxy Tab"]);
                assert_eq!(list.Next(&mut got[..1], None), S_FALSE);
                list.Reset().unwrap();
                let mut one: [Option<IExplorerCommand>; 1] = Default::default();
                assert_eq!(list.Next(&mut one, None), S_OK);
                let tip = text(one[0].as_ref().unwrap().GetToolTip(None).unwrap());
                assert_eq!(tip, "Send to Pixel 8 with NectarLink");
            }
        }

        #[test]
        fn the_app_gets_the_phone_then_the_files() {
            let args = send_args("0a", &[PathBuf::from(r"C:\Users\me\a b.txt"), PathBuf::from(r"D:\photos")]);
            assert_eq!(args, ["--send-to", "0a", r"C:\Users\me\a b.txt", r"D:\photos"]);
        }
    }
}
