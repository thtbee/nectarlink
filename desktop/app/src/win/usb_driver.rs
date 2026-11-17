// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether an Android phone is plugged in whose USB debugging interface has
//! no driver. adb can't see such a phone at all, so the installer page says
//! why instead of waiting for nothing. Windows installs the driver for many
//! phones on its own (Google's, from Windows Update); others need their
//! maker's.

use windows::{
    Win32::Devices::DeviceAndDriverInstallation::{
        CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_PROB, CR_SUCCESS, DIGCF_ALLCLASSES, DIGCF_PRESENT,
        SP_DEVINFO_DATA, SPDRP_COMPATIBLEIDS, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
    },
    core::PCWSTR,
};

/// What every Android device's USB debugging interface reports as a
/// compatible ID (class 0xff, subclass 0x42, protocol 1).
const ADB_INTERFACE: &str = "usb\\class_ff&subclass_42&prot_01";

/// Whether a present device is an adb interface Windows has no working driver for.
pub fn android_without_driver() -> bool {
    // SAFETY: SetupAPI over a device list this function creates and destroys;
    // each buffer is sized by the call that fills it.
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
        else {
            return false;
        };
        let mut found = false;
        let mut index = 0;
        loop {
            let mut data =
                SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set, index, &mut data).is_err() {
                break;
            }
            index += 1;
            let mut buffer = [0u8; 2048];
            let mut needed = 0u32;
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                &data,
                SPDRP_COMPATIBLEIDS,
                None,
                Some(&mut buffer),
                Some(&mut needed),
            )
            .is_err()
            {
                continue;
            }
            if !compatible_ids(&buffer[..(needed as usize).min(buffer.len())])
                .iter()
                .any(|id| id == ADB_INTERFACE)
            {
                continue;
            }
            let mut status = CM_DEVNODE_STATUS_FLAGS::default();
            let mut problem = CM_PROB::default();
            let answer = CM_Get_DevNode_Status(&mut status, &mut problem, data.DevInst, 0);
            if answer != CR_SUCCESS || problem.0 != 0 {
                found = true;
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

/// The IDs in a REG_MULTI_SZ value (UTF-16, each ended by a nul), lowercased.
fn compatible_ids(raw: &[u8]) -> Vec<String> {
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    units
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf16_lossy(s).to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_ids_are_read_from_a_multi_string() {
        let ids = ["USB\\Class_ff&SubClass_42&Prot_01", "USB\\Class_ff&SubClass_42", "USB\\Class_ff"];
        let mut raw = Vec::new();
        for id in ids {
            raw.extend(id.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        }
        raw.extend([0, 0]);
        let read = compatible_ids(&raw);
        assert_eq!(read.len(), 3);
        assert_eq!(read[0], ADB_INTERFACE);
    }

    /// Runs against this PC's devices (nothing to assert; it mustn't fail).
    #[test]
    fn looking_at_this_pcs_devices_works() {
        let _ = android_without_driver();
    }
}
