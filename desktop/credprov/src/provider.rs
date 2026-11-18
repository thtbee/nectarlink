// SPDX-License-Identifier: GPL-3.0-or-later
//! The credential provider LogonUI loads (ADR 0009): a tile, "Unlock with
//! phone", for the account signed in to the locked session, and only while its
//! NectarLink is running (so never at the first sign-in after a restart). Its
//! button asks the user's NectarLink over the unlock pipe; with the password
//! the phone's owner released, it hands Windows a Kerberos unlock credential.
//! Every entry point catches panics: nothing may unwind into LogonUI.

#![allow(unsafe_code, non_snake_case)]

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU32, Ordering},
    },
};

use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_NOTIMPL, E_POINTER, E_UNEXPECTED, NTSTATUS},
        Globalization::GetUserDefaultUILanguage,
        Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP},
        Storage::EnhancedStorage::PKEY_Identity_QualifiedUserName,
        System::Com::{CoTaskMemAlloc, CoTaskMemFree},
        UI::Shell::{
            CPFG_CREDENTIAL_PROVIDER_LABEL, CPFG_CREDENTIAL_PROVIDER_LOGO, CPFIS_NONE, CPFS_DISPLAY_IN_BOTH,
            CPFS_DISPLAY_IN_SELECTED_TILE, CPFT_LARGE_TEXT, CPFT_SMALL_TEXT, CPFT_SUBMIT_BUTTON,
            CPFT_TILE_IMAGE, CPGSR_NO_CREDENTIAL_NOT_FINISHED, CPGSR_RETURN_CREDENTIAL_FINISHED, CPSI_ERROR,
            CPSI_NONE, CPUS_LOGON, CPUS_UNLOCK_WORKSTATION, CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
            CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR, CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE,
            CREDENTIAL_PROVIDER_FIELD_STATE, CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE,
            CREDENTIAL_PROVIDER_NO_DEFAULT, CREDENTIAL_PROVIDER_STATUS_ICON,
            CREDENTIAL_PROVIDER_USAGE_SCENARIO, ICredentialProvider, ICredentialProvider_Impl,
            ICredentialProviderCredential, ICredentialProviderCredential_Impl,
            ICredentialProviderCredential2, ICredentialProviderCredential2_Impl,
            ICredentialProviderCredentialEvents, ICredentialProviderEvents, ICredentialProviderSetUserArray,
            ICredentialProviderSetUserArray_Impl, ICredentialProviderUserArray, SHStrDupW,
        },
    },
    core::{BOOL, GUID, HSTRING, PCWSTR, PWSTR, Ref, implement},
};

use crate::{
    CLSID_NECTARLINK_UNLOCK, kerb, pipe, session,
    text::{self, Text},
    trace::note,
    wire::{self, Status},
};

/// Live COM objects and server locks, for `DllCanUnloadNow`.
pub static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

/// Counts one live object while it exists.
pub struct ObjectGuard;

impl ObjectGuard {
    pub fn new() -> ObjectGuard {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        ObjectGuard
    }
}

impl Default for ObjectGuard {
    fn default() -> ObjectGuard {
        ObjectGuard::new()
    }
}

impl Drop for ObjectGuard {
    fn drop(&mut self) {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

const FIELD_TITLE: u32 = 0;
const FIELD_STATUS: u32 = 1;
const FIELD_SUBMIT: u32 = 2;
/// The tile's picture, which Windows 10 and later also show as the icon
/// under "Sign-in options".
const FIELD_TILE: u32 = 3;
const FIELD_COUNT: u32 = 4;

/// The tile's picture: NectarLink's icon, 128 px square, as top-down BGRA rows.
const TILE: &[u8; 128 * 128 * 4] = include_bytes!("tile.bgra");

/// A new bitmap of [`TILE`]; LogonUI deletes it.
fn tile_bitmap() -> windows::core::Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 128,
            biHeight: -128,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: `info` describes a 128x128 32-bit DIB; Windows allocates `bits` for it.
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    if bits.is_null() {
        return Err(E_UNEXPECTED.into());
    }
    // SAFETY: `bits` holds exactly TILE.len() bytes for this bitmap.
    unsafe { std::ptr::copy_nonoverlapping(TILE.as_ptr(), bits.cast::<u8>(), TILE.len()) };
    Ok(bitmap)
}

/// Runs `f`, turning a panic into `E_UNEXPECTED` instead of unwinding into LogonUI.
fn guarded<T>(f: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_UNEXPECTED.into()))
}

fn say(t: Text) -> &'static str {
    // SAFETY: no arguments; returns the user's UI language ID.
    let lang = unsafe { GetUserDefaultUILanguage() };
    text::get(t, lang & 0x3ff)
}

fn dup(s: &str) -> windows::core::Result<PWSTR> {
    // SAFETY: SHStrDupW copies the string into memory LogonUI frees.
    unsafe { SHStrDupW(&HSTRING::from(s)) }
}

/// Reads a string LogonUI allocated and frees it.
fn take(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` is a NUL-terminated string LogonUI gave us to own.
    let s = unsafe { p.to_string() }.ok();
    // SAFETY: allocated with CoTaskMemAlloc by the caller's convention.
    unsafe { CoTaskMemFree(Some(p.0.cast())) };
    s
}

// ---- The provider ----

#[implement(ICredentialProvider, ICredentialProviderSetUserArray)]
pub struct Provider {
    credentials: Mutex<Vec<ICredentialProviderCredential>>,
    /// The scenario LogonUI set (`0` before it did, or for one it isn't for).
    scenario: Arc<AtomicI32>,
    _guard: ObjectGuard,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Provider")
    }
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            credentials: Mutex::default(),
            scenario: Arc::new(AtomicI32::new(0)),
            _guard: ObjectGuard::new(),
        }
    }
}

impl Default for Provider {
    fn default() -> Provider {
        Provider::new()
    }
}

impl ICredentialProvider_Impl for Provider_Impl {
    fn SetUsageScenario(
        &self,
        cpus: CREDENTIAL_PROVIDER_USAGE_SCENARIO,
        _dwflags: u32,
    ) -> windows::core::Result<()> {
        guarded(|| {
            // The lock screen: Windows 10 and later mostly ask for a logon there
            // (sign-in and unlock were merged), some setups for an unlock. Never
            // other password prompts (CredUI, changing a password). The tile is
            // then only for the account signed in to this session.
            note!("usage scenario {}", cpus.0);
            if cpus == CPUS_LOGON || cpus == CPUS_UNLOCK_WORKSTATION {
                self.scenario.store(cpus.0, Ordering::SeqCst);
                Ok(())
            } else {
                self.scenario.store(0, Ordering::SeqCst);
                Err(E_NOTIMPL.into())
            }
        })
    }

    fn SetSerialization(
        &self,
        _pcpcs: *const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Advise(
        &self,
        _pcpe: Ref<'_, ICredentialProviderEvents>,
        _context: usize,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn UnAdvise(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn GetFieldDescriptorCount(&self) -> windows::core::Result<u32> {
        Ok(FIELD_COUNT)
    }

    fn GetFieldDescriptorAt(
        &self,
        index: u32,
    ) -> windows::core::Result<*mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR> {
        guarded(|| {
            let (kind, label, guid) = match index {
                FIELD_TITLE => (CPFT_LARGE_TEXT, say(Text::Title), CPFG_CREDENTIAL_PROVIDER_LABEL),
                FIELD_STATUS => (CPFT_SMALL_TEXT, say(Text::Hint), GUID::zeroed()),
                FIELD_SUBMIT => (CPFT_SUBMIT_BUTTON, say(Text::Unlock), GUID::zeroed()),
                FIELD_TILE => (CPFT_TILE_IMAGE, say(Text::Title), CPFG_CREDENTIAL_PROVIDER_LOGO),
                _ => return Err(E_INVALIDARG.into()),
            };
            let label = dup(label)?;
            // SAFETY: LogonUI frees the descriptor (and its label) with CoTaskMemFree.
            let descriptor = unsafe { CoTaskMemAlloc(size_of::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>()) }
                .cast::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>();
            if descriptor.is_null() {
                // SAFETY: `label` came from SHStrDupW.
                unsafe { CoTaskMemFree(Some(label.0.cast())) };
                return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
            }
            // SAFETY: `descriptor` is freshly allocated and big enough.
            unsafe {
                descriptor.write(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR {
                    dwFieldID: index,
                    cpft: kind,
                    pszLabel: label,
                    guidFieldType: guid,
                });
            }
            Ok(descriptor)
        })
    }

    fn GetCredentialCount(
        &self,
        pdwcount: *mut u32,
        pdwdefault: *mut u32,
        pbautologonwithdefault: *mut BOOL,
    ) -> windows::core::Result<()> {
        guarded(|| {
            if pdwcount.is_null() || pdwdefault.is_null() || pbautologonwithdefault.is_null() {
                return Err(E_POINTER.into());
            }
            // Nothing to offer unless NectarLink is running in this session.
            let scenario = self.scenario.load(Ordering::SeqCst);
            let served = scenario != 0 && pipe::server_present();
            let tiles = self.credentials.lock().map_err(|_| E_UNEXPECTED)?.len() as u32;
            let count = if served { tiles } else { 0 };
            note!(
                "credential count: scenario {scenario}, NectarLink running {served}, tiles {tiles}, offering {count}"
            );
            // SAFETY: checked non-null above; LogonUI owns them.
            unsafe {
                *pdwcount = count;
                // Never the default: the user picks the tile under Sign-in options.
                *pdwdefault = CREDENTIAL_PROVIDER_NO_DEFAULT;
                *pbautologonwithdefault = BOOL(0);
            }
            Ok(())
        })
    }

    fn GetCredentialAt(&self, index: u32) -> windows::core::Result<ICredentialProviderCredential> {
        guarded(|| {
            self.credentials
                .lock()
                .map_err(|_| E_UNEXPECTED)?
                .get(index as usize)
                .cloned()
                .ok_or(E_INVALIDARG.into())
        })
    }
}

impl ICredentialProviderSetUserArray_Impl for Provider_Impl {
    fn SetUserArray(&self, users: Ref<'_, ICredentialProviderUserArray>) -> windows::core::Result<()> {
        guarded(|| {
            let users = users.ok()?;
            let mut credentials = Vec::new();
            // Only the account already signed in to this session: nobody at the
            // first sign-in after a restart, so no tile then.
            let Some(signed_in) = session::signed_in_sid() else {
                note!("user array: nobody is signed in to this session");
                *self.credentials.lock().map_err(|_| E_UNEXPECTED)? = credentials;
                return Ok(());
            };
            // SAFETY: a valid user array from LogonUI; each string is ours to free.
            unsafe {
                note!("user array: {} users", users.GetCount()?);
                for i in 0..users.GetCount()? {
                    let user = users.GetAt(i)?;
                    let (Some(sid), Some(qualified)) =
                        (take(user.GetSid()?), take(user.GetStringValue(&PKEY_Identity_QualifiedUserName)?))
                    else {
                        note!("user array: user {i} has no SID or name");
                        continue;
                    };
                    if !wire::user_ok(&sid) || sid != signed_in {
                        note!("user array: user {i} isn't the one signed in");
                        continue;
                    }
                    let credential: ICredentialProviderCredential2 =
                        Credential::new(sid, qualified, self.scenario.clone()).into();
                    credentials.push(credential.into());
                }
            }
            *self.credentials.lock().map_err(|_| E_UNEXPECTED)? = credentials;
            Ok(())
        })
    }
}

// ---- One user's tile ----

#[implement(ICredentialProviderCredential2, ICredentialProviderCredential)]
pub struct Credential {
    sid: String,
    qualified: String,
    scenario: Arc<AtomicI32>,
    status: Mutex<String>,
    /// Whether this lock screen already asked the phone by itself.
    asked: std::sync::atomic::AtomicBool,
    _events: Mutex<Option<ICredentialProviderCredentialEvents>>,
    _guard: ObjectGuard,
}

/// Never prints the user's account name.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential")
    }
}

impl Credential {
    fn new(sid: String, qualified: String, scenario: Arc<AtomicI32>) -> Credential {
        Credential {
            sid,
            qualified,
            scenario,
            status: Mutex::new(say(Text::Hint).to_owned()),
            asked: std::sync::atomic::AtomicBool::new(false),
            _events: Mutex::default(),
            _guard: ObjectGuard::new(),
        }
    }

    /// Asks the user's NectarLink and builds the serialization; `Err(text)`
    /// says why it didn't work.
    fn unlock(&self) -> Result<CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION, Text> {
        let (status, mut password) = pipe::ask(&self.sid).map_err(|e| match e {
            pipe::AskError::NotRunning => Text::NotRunning,
            pipe::AskError::Timeout => Text::Declined,
            pipe::AskError::Broken => Text::Failed,
        })?;
        let refusal = match status {
            Status::Ok => None,
            Status::Declined => Some(Text::Declined),
            Status::NotEnrolled => Some(Text::NotEnrolled),
            Status::PhoneAway => Some(Text::PhoneAway),
            Status::LockedOut => Some(Text::LockedOut),
            Status::Failed => Some(Text::Failed),
        };
        if let Some(text) = refusal {
            wire::wipe(&mut password);
            return Err(text);
        }
        let mut wide: Vec<u16> =
            std::str::from_utf8(&password).map(|p| p.encode_utf16().collect()).unwrap_or_default();
        wire::wipe(&mut password);
        if wide.is_empty() {
            return Err(Text::Failed);
        }
        let (domain, user) = wire::split_qualified(&self.qualified);
        let kind = if self.scenario.load(Ordering::SeqCst) == CPUS_UNLOCK_WORKSTATION.0 {
            kerb::Kind::Unlock
        } else {
            kerb::Kind::Logon
        };
        let packed = kerb::pack_unlock(kind, domain, user, &wide);
        wide.fill(0);
        std::hint::black_box(&wide);
        let mut packed = packed.map_err(|_| Text::Failed)?;
        let package = kerb::negotiate_package().map_err(|_| {
            wire::wipe(&mut packed);
            Text::Failed
        })?;
        // SAFETY: LogonUI takes ownership and frees it with CoTaskMemFree.
        let buffer = unsafe { CoTaskMemAlloc(packed.len()) }.cast::<u8>();
        if buffer.is_null() {
            wire::wipe(&mut packed);
            return Err(Text::Failed);
        }
        // SAFETY: `buffer` has room for `packed`.
        unsafe { std::ptr::copy_nonoverlapping(packed.as_ptr(), buffer, packed.len()) };
        let size = packed.len() as u32;
        wire::wipe(&mut packed);
        Ok(CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION {
            ulAuthenticationPackage: package,
            clsidCredentialProvider: CLSID_NECTARLINK_UNLOCK,
            cbSerialization: size,
            rgbSerialization: buffer,
        })
    }
}

fn set_status(text: *mut PWSTR, icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON, message: Option<&str>) {
    // SAFETY: optional out pointers from LogonUI, written only when non-null.
    unsafe {
        if !text.is_null() {
            *text = message.and_then(|m| dup(m).ok()).unwrap_or(PWSTR::null());
        }
        if !icon.is_null() {
            *icon = if message.is_some() { CPSI_ERROR } else { CPSI_NONE };
        }
    }
}

impl ICredentialProviderCredential_Impl for Credential_Impl {
    fn Advise(&self, pcpce: Ref<'_, ICredentialProviderCredentialEvents>) -> windows::core::Result<()> {
        guarded(|| {
            *self._events.lock().map_err(|_| E_UNEXPECTED)? = pcpce.ok().ok().cloned();
            Ok(())
        })
    }

    fn UnAdvise(&self) -> windows::core::Result<()> {
        guarded(|| {
            *self._events.lock().map_err(|_| E_UNEXPECTED)? = None;
            Ok(())
        })
    }

    fn SetSelected(&self) -> windows::core::Result<BOOL> {
        // Picking the tile asks the phone right away, once per lock screen:
        // after a no or a timeout the tile is selected again, and the arrow
        // asks again only when pressed.
        Ok(BOOL::from(!self.asked.swap(true, Ordering::SeqCst)))
    }

    fn SetDeselected(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn GetFieldState(
        &self,
        field: u32,
        pcpfs: *mut CREDENTIAL_PROVIDER_FIELD_STATE,
        pcpfis: *mut CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE,
    ) -> windows::core::Result<()> {
        if pcpfs.is_null() || pcpfis.is_null() {
            return Err(E_POINTER.into());
        }
        let state = match field {
            FIELD_TITLE | FIELD_TILE => CPFS_DISPLAY_IN_BOTH,
            FIELD_STATUS | FIELD_SUBMIT => CPFS_DISPLAY_IN_SELECTED_TILE,
            _ => return Err(E_INVALIDARG.into()),
        };
        // SAFETY: checked non-null above.
        unsafe {
            *pcpfs = state;
            *pcpfis = CPFIS_NONE;
        }
        Ok(())
    }

    fn GetStringValue(&self, field: u32) -> windows::core::Result<PWSTR> {
        guarded(|| match field {
            FIELD_TITLE => dup(say(Text::Title)),
            FIELD_STATUS => dup(&self.status.lock().map_err(|_| E_UNEXPECTED)?),
            _ => Err(E_INVALIDARG.into()),
        })
    }

    fn GetBitmapValue(&self, field: u32) -> windows::core::Result<HBITMAP> {
        guarded(|| if field == FIELD_TILE { tile_bitmap() } else { Err(E_INVALIDARG.into()) })
    }

    fn GetCheckboxValue(
        &self,
        _field: u32,
        _checked: *mut BOOL,
        _label: *mut PWSTR,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSubmitButtonValue(&self, field: u32) -> windows::core::Result<u32> {
        if field == FIELD_SUBMIT { Ok(FIELD_STATUS) } else { Err(E_INVALIDARG.into()) }
    }

    fn GetComboBoxValueCount(
        &self,
        _field: u32,
        _items: *mut u32,
        _selected: *mut u32,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueAt(&self, _field: u32, _item: u32) -> windows::core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn SetStringValue(&self, _field: u32, _psz: &PCWSTR) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetCheckboxValue(&self, _field: u32, _checked: BOOL) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetComboBoxSelectedValue(&self, _field: u32, _selected: u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn CommandLinkClicked(&self, _field: u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSerialization(
        &self,
        pcpgsr: *mut CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE,
        pcpcs: *mut CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
        ppszoptionalstatustext: *mut PWSTR,
        pcpsioptionalstatusicon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows::core::Result<()> {
        guarded(|| {
            if pcpgsr.is_null() || pcpcs.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: checked non-null; nothing is handed over until filled in.
            unsafe {
                *pcpgsr = CPGSR_NO_CREDENTIAL_NOT_FINISHED;
                *pcpcs = CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION::default();
            }
            match self.unlock() {
                Ok(serialization) => {
                    note!("unlock: the phone said yes, handing Windows the credential");
                    // SAFETY: checked non-null above.
                    unsafe {
                        *pcpcs = serialization;
                        *pcpgsr = CPGSR_RETURN_CREDENTIAL_FINISHED;
                    }
                    set_status(ppszoptionalstatustext, pcpsioptionalstatusicon, None);
                }
                Err(text) => {
                    note!("unlock: no credential ({text:?})");
                    set_status(ppszoptionalstatustext, pcpsioptionalstatusicon, Some(say(text)));
                }
            }
            Ok(())
        })
    }

    fn ReportResult(
        &self,
        ntsstatus: NTSTATUS,
        _ntssubstatus: NTSTATUS,
        ppszoptionalstatustext: *mut PWSTR,
        pcpsioptionalstatusicon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows::core::Result<()> {
        guarded(|| {
            note!("result from Windows: {:#x}", ntsstatus.0);
            // The password from the phone is wrong: it was changed since setting up.
            // NectarLink drops it, so this tile doesn't keep trying it (and lock
            // the account under a lockout policy).
            if wrong_password(ntsstatus) {
                let told = pipe::tell_rejected(&self.sid);
                note!("told NectarLink the password is out of date: {}", told.is_ok());
            }
            let message = ntsstatus.is_err().then(|| say(Text::Rejected));
            set_status(ppszoptionalstatustext, pcpsioptionalstatusicon, message);
            Ok(())
        })
    }
}

impl ICredentialProviderCredential2_Impl for Credential_Impl {
    fn GetUserSid(&self) -> windows::core::Result<PWSTR> {
        guarded(|| dup(&self.sid))
    }
}

/// Whether Windows refused the password itself: `STATUS_LOGON_FAILURE` or
/// `STATUS_WRONG_PASSWORD`, not a locked or disabled account.
fn wrong_password(status: NTSTATUS) -> bool {
    matches!(status.0 as u32, 0xC000_006D | 0xC000_006A)
}

/// The provider as LogonUI asks for it.
pub fn provider() -> ICredentialProvider {
    Provider::new().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HGDIOBJ};

    #[test]
    fn the_tile_has_its_picture() {
        let bitmap = tile_bitmap().unwrap();
        let mut info = BITMAP::default();
        // SAFETY: a live bitmap and a BITMAP-sized buffer; deleted below.
        unsafe {
            let read =
                GetObjectW(HGDIOBJ(bitmap.0), size_of::<BITMAP>() as i32, Some((&raw mut info).cast()));
            assert_eq!(read as usize, size_of::<BITMAP>());
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        assert_eq!((info.bmWidth, info.bmHeight, info.bmBitsPixel), (128, 128, 32));
        let provider = Provider::new();
        assert_eq!(Provider_Impl::GetFieldDescriptorCount(&provider.into_outer()).unwrap(), FIELD_COUNT);
    }
}
