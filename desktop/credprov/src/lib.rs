// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink_credprov`: unlock a locked Windows session with the phone
//! (docs/adr/0009-unlock-with-phone.md). A credential provider DLL that LogonUI
//! loads once the user turns the feature on, plus [`wire`], the unlock pipe's
//! messages, which the desktop app shares.
//!
//! The DLL knows nothing about phones: it asks the signed-in user's NectarLink
//! over a pipe only SYSTEM can open, and turns the password it gets back into
//! an unlock credential.

#![allow(linker_messages)]

pub mod wire;

#[cfg(windows)]
mod kerb;
#[cfg(windows)]
mod pipe;
#[cfg(windows)]
mod provider;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod text;
#[cfg(windows)]
mod trace;

pub use wire::CLSID_TEXT;

#[cfg(windows)]
pub use self::com::*;

#[cfg(windows)]
#[allow(unsafe_code)]
mod com {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::atomic::Ordering,
    };

    use windows::{
        Win32::{
            Foundation::{
                CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER, E_UNEXPECTED,
                S_FALSE, S_OK,
            },
            System::Com::{IClassFactory, IClassFactory_Impl},
        },
        core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, implement},
    };

    use crate::provider::{ACTIVE_OBJECTS, ObjectGuard};

    /// CLSID of the NectarLink unlock credential provider.
    pub const CLSID_NECTARLINK_UNLOCK: GUID = GUID::from_u128(0xbc7b4bcf_b24c_4c5b_9e55_cb09e19a8799);

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
            let provider: IUnknown = crate::provider::provider().cast()?;
            // SAFETY: `riid` and `object` are valid per the checks above.
            unsafe { provider.query(riid, object).ok() }
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

    /// Standard COM entry point: the class factory for [`CLSID_NECTARLINK_UNLOCK`].
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
            if unsafe { *rclsid } != CLSID_NECTARLINK_UNLOCK {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = Factory { _guard: ObjectGuard::new() }.into();
            match factory.cast::<IUnknown>() {
                // SAFETY: `riid` and `ppv` are valid per the checks above.
                Ok(unknown) => unsafe { unknown.query(riid, ppv) },
                Err(e) => e.code(),
            }
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
        use windows::Win32::UI::Shell::{
            CPUS_CREDUI, CPUS_LOGON, CPUS_UNLOCK_WORKSTATION, ICredentialProvider,
        };

        use super::*;

        #[test]
        fn the_dll_makes_a_provider_that_only_unlocks() {
            let mut raw = std::ptr::null_mut();
            // SAFETY: valid pointers to locals.
            let hr = unsafe { DllGetClassObject(&CLSID_NECTARLINK_UNLOCK, &IClassFactory::IID, &mut raw) };
            assert_eq!(hr, S_OK);
            // SAFETY: `raw` is the IClassFactory just returned.
            let factory = unsafe { IClassFactory::from_raw(raw) };
            // SAFETY: plain COM call.
            let provider: ICredentialProvider = unsafe { factory.CreateInstance(None) }.unwrap();
            // SAFETY: plain COM calls on a live object.
            unsafe {
                assert!(provider.SetUsageScenario(CPUS_CREDUI, 0).is_err(), "never other prompts");
                // The lock screen asks for either (Windows 10 merged them).
                assert!(provider.SetUsageScenario(CPUS_UNLOCK_WORKSTATION, 0).is_ok());
                assert!(provider.SetUsageScenario(CPUS_LOGON, 0).is_ok());
                assert_eq!(provider.GetFieldDescriptorCount().unwrap(), 4);
                let (mut count, mut default, mut auto) = (9, 0, BOOL(1));
                provider.GetCredentialCount(&mut count, &mut default, &mut auto).unwrap();
                // No user array yet, and no NectarLink pipe in a test run: no tile.
                assert_eq!((count, auto.as_bool()), (0, false));
            }
            drop(provider);
            drop(factory);
            assert_eq!(DllCanUnloadNow(), S_OK);

            let mut raw = std::ptr::null_mut();
            let other = GUID::from_u128(1);
            // SAFETY: valid pointers to locals.
            let hr = unsafe { DllGetClassObject(&other, &IClassFactory::IID, &mut raw) };
            assert_eq!(hr, CLASS_E_CLASSNOTAVAILABLE);
            assert!(raw.is_null());
        }
    }
}
