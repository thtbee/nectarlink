// SPDX-License-Identifier: GPL-3.0-or-later
//! What the Connection Doctor looks at on Windows: the firewall's rules for
//! this app, the network's profile (Public networks hide the PC), whether
//! there's a network at all, and VPNs, which often keep phones out.

use std::path::Path;

use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        NetworkManagement::{
            IpHelper::{
                GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
            },
            WindowsFirewall::{
                INetFwPolicy2, INetFwRule, NET_FW_ACTION_ALLOW, NET_FW_ACTION_BLOCK, NET_FW_RULE_DIR_IN,
                NetFwPolicy2,
            },
        },
        Networking::{
            NetworkListManager::{
                INetwork, INetworkListManager, NLM_ENUM_NETWORK_CONNECTED,
                NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED, NLM_NETWORK_CATEGORY_PRIVATE, NetworkListManager,
            },
            WinSock::AF_UNSPEC,
        },
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            Ole::IEnumVARIANT,
            Threading::{INFINITE, WaitForSingleObject},
            Variant::{VARIANT, VariantClear},
        },
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{HSTRING, IUnknown, Interface, PCWSTR, w},
};

use super::with_com;

/// The firewall's view of this app, for incoming connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Firewall {
    /// The firewall is on for the current network's profile.
    pub on: bool,
    /// An enabled rule lets the app in on the current profile.
    pub allowed: bool,
    /// An enabled rule blocks the app (Windows adds one when someone
    /// cancels its "allow access" prompt), which wins over allow rules.
    pub blocked: bool,
}

/// How Windows treats the network the PC is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub name: String,
    /// Private or domain: devices on it can find this PC.
    pub trusted: bool,
}

/// The firewall's rules for `exe`.
pub fn firewall(exe: &Path) -> windows::core::Result<Firewall> {
    let exe = exe.to_string_lossy().to_lowercase();
    with_com(|| {
        // SAFETY: COM calls on live objects; every VARIANT is cleared.
        unsafe {
            let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)?;
            let profiles = policy.CurrentProfileTypes()?;
            let mut on = false;
            for bit in [1, 2, 4] {
                if profiles & bit != 0
                    && policy
                        .get_FirewallEnabled(
                            windows::Win32::NetworkManagement::WindowsFirewall::NET_FW_PROFILE_TYPE2(bit),
                        )?
                        .as_bool()
                {
                    on = true;
                }
            }
            let mut result = Firewall { on, ..Firewall::default() };
            let rules = policy.Rules()?;
            let all: IEnumVARIANT = rules._NewEnum()?.cast()?;
            loop {
                let mut item = [VARIANT::default()];
                let mut fetched = 0;
                if all.Next(&mut item, &mut fetched).is_err() || fetched == 0 {
                    break;
                }
                let rule = IUnknown::try_from(&item[0]).ok().and_then(|u| u.cast::<INetFwRule>().ok());
                let _ = VariantClear(&mut item[0]);
                let Some(rule) = rule else { continue };
                let applies = rule.Enabled().is_ok_and(|e| e.as_bool())
                    && rule.Direction().is_ok_and(|d| d == NET_FW_RULE_DIR_IN)
                    && rule.Profiles().is_ok_and(|p| p & profiles != 0)
                    && rule.ApplicationName().is_ok_and(|a| a.to_string().to_lowercase() == exe);
                if !applies {
                    continue;
                }
                match rule.Action() {
                    Ok(a) if a == NET_FW_ACTION_ALLOW => result.allowed = true,
                    Ok(a) if a == NET_FW_ACTION_BLOCK => result.blocked = true,
                    _ => {}
                }
            }
            Ok(result)
        }
    })
}

/// The networks the PC is connected to.
pub fn networks() -> windows::core::Result<Vec<Network>> {
    with_com(|| {
        // SAFETY: COM calls on live objects.
        unsafe {
            let manager: INetworkListManager =
                CoCreateInstance(&NetworkListManager, None, CLSCTX_INPROC_SERVER)?;
            let list = manager.GetNetworks(NLM_ENUM_NETWORK_CONNECTED)?;
            let mut networks = Vec::new();
            loop {
                let mut item: [Option<INetwork>; 1] = [None];
                let mut fetched = 0;
                if list.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(network) = item[0].take() else { break };
                let category = network.GetCategory()?;
                networks.push(Network {
                    name: network.GetName().map(|n| n.to_string()).unwrap_or_default(),
                    trusted: category == NLM_NETWORK_CATEGORY_PRIVATE
                        || category == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED,
                });
            }
            Ok(networks)
        }
    })
}

/// What the network adapters say: whether one has a local address, and
/// whether a VPN is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Adapters {
    pub local_address: bool,
    pub vpn: bool,
}

pub fn adapters() -> Adapters {
    const IF_TYPE_PPP: u32 = 23;
    const IF_TYPE_TUNNEL: u32 = 131;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    let mut found = Adapters::default();
    let mut size: u32 = 32 * 1024;
    let mut buffer = vec![0u8; size as usize];
    // SAFETY: the buffer is sized as GetAdaptersAddresses asks (retried
    // once when too small) and only read as the linked list it fills in.
    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        let mut status = GetAdaptersAddresses(
            u32::from(AF_UNSPEC.0),
            flags,
            None,
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
        );
        if status == 111 {
            // ERROR_BUFFER_OVERFLOW
            buffer = vec![0u8; size as usize];
            status = GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
            );
        }
        if status != 0 {
            return found;
        }
        let mut adapter = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while let Some(a) = adapter.as_ref() {
            let up = a.OperStatus.0 == 1;
            let description = a.Description.to_string().unwrap_or_default().to_lowercase();
            let kind = a.IfType;
            if up && kind != IF_TYPE_SOFTWARE_LOOPBACK {
                let vpn_like = kind == IF_TYPE_PPP
                    || kind == IF_TYPE_TUNNEL
                    || ["vpn", "wireguard", "tap-windows", "openvpn", "tailscale", "zerotier"]
                        .iter()
                        .any(|word| description.contains(word));
                if vpn_like {
                    found.vpn = true;
                } else {
                    let mut unicast = a.FirstUnicastAddress;
                    while let Some(u) = unicast.as_ref() {
                        let addr = u.Address.lpSockaddr;
                        // AF_INET with a non link-local address.
                        if !addr.is_null() && (*addr).sa_family.0 == 2 {
                            let bytes = &(*addr).sa_data;
                            let first = bytes[2] as u8;
                            let second = bytes[3] as u8;
                            if !(first == 169 && second == 254) && first != 127 {
                                found.local_address = true;
                            }
                        }
                        unicast = u.Next;
                    }
                }
            }
            adapter = a.Next;
        }
    }
    found
}

/// Lets `exe` through the firewall on private and domain networks, removing
/// rules that block it. Asks for administrator rights (the UAC prompt) and
/// waits until it's done; `Ok(false)` if the user said no.
pub fn fix_firewall(exe: &Path) -> windows::core::Result<bool> {
    let exe = exe.to_string_lossy();
    let script = format!(
        "/c netsh advfirewall firewall delete rule name=all dir=in program=\"{exe}\" & \
         netsh advfirewall firewall add rule name=\"Nectarlink\" dir=in action=allow program=\"{exe}\" \
         enable=yes profile=private,domain"
    );
    let parameters = HSTRING::from(script);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: w!("cmd.exe"),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the structure and its strings outlive the call; the process
    // handle is waited on and closed.
    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            // Most often: the user declined the prompt.
            return Ok(false);
        }
        if !info.hProcess.is_invalid() {
            let waited = WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
            if waited != WAIT_OBJECT_0 {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_firewall_and_networks() {
        // Whatever this machine's state, the questions get answers.
        let exe = std::env::current_exe().unwrap();
        firewall(&exe).expect("firewall rules");
        networks().expect("networks");
        let _ = adapters();
    }
}
