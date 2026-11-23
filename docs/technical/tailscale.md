# Connecting over Tailscale

Two paired devices on the same tailnet can reach each other with Connect over
mobile data (Away mode) off. A Tailscale path never unlocks the PC, and the core
never confuses a carrier's CGNAT address with a tailnet one.

## Where addresses come from

The core keeps each peer's addresses in `peer_addrs` (store schema v10), one row
per address and source (`store::CandidateSource`):

| Source | Written by | Replaced when |
| --- | --- | --- |
| `Manual` | `Node::set_manual_addrs` (the user typed it) | the user edits the list |
| `Advertised` | the peer's `hello` / `hello.update` `direct_addrs` | the peer sends a new list |
| `Observed` | the direct paths a working session used | a newer session |
| `Discovery` | a QR pairing link, and `Node::add_tailscale_candidates` | pairing again; Tailscale additions merge |
| `Legacy` | the v10 migration of the old `last_addrs` column | never |

mDNS announcements stay in memory (`Shared::lan_seen`) and aren't stored.

Each source holds at most `MAX_CANDIDATES_PER_SOURCE` (16) addresses, so a noisy
source can't push out the others. `Store::replace_candidates` swaps one source's
set and leaves the rest alone.

The `direct_addrs` field in `hello` and `hello.update` follows three rules:

- missing (`None`) keeps the stored `Advertised` set;
- an empty list (`Some([])`) withdraws it;
- a list replaces only `Advertised`, never `Manual` or `Observed`.

`Shared::refresh_lookup` merges the stored candidates, short-lived hints and the
USB forward address, drops what the current mode doesn't allow, and hands the
result to iroh's `MemoryLookup` (`set_endpoint_info`, or `remove_endpoint_info`
when nothing is left). Loopback survives only as the USB forward.

## Telling Tailscale from carrier CGNAT

Tailscale uses `100.64.0.0/10` and `fd7a:115c:a1e0::/48`. Mobile carriers use
the same IPv4 range for CGNAT, so a `100.x` address alone proves nothing. The
core treats one as a tailnet address only when this device is on Tailscale
itself (`session::is_corroborated_tailnet_ip`). This device's Tailscale
addresses come from two places:

- the app, through `Node::set_local_tailscale_ips`;
- on Linux, any up interface whose name starts with `tailscale`, unless
  `NodeConfig::tailscale_interfaces` is off (the core's tests turn it off so a
  machine running Tailscale doesn't change the paths they see).

`Shared::refresh_tailscale_ips` merges them, keeping only tailnet-range
addresses. Without corroboration a `100.x` address ranks as an ordinary public
IPv4 address. It is dialed only in Away mode, and it never gets an unlock.

## Paths and unlock

`ConnectionPath` has four values, best first: `Usb`, `Lan`, `Tailscale`,
`Relay`. A session over a corroborated tailnet address is `Tailscale`.
`is_unlock_local_ip` and `is_private_lan_ip` both reject tailnet ranges, so
phone unlock stays LAN and USB only. One exception, for allow and unlock alike:
a peer address that is one of this machine's own addresses
(`Shared::is_own_ip`) means the peer runs on this machine, which counts as
loopback. Two test nodes on a host running Tailscale would otherwise meet over
that host's tailnet address and get refused.

While a peer sits on a Tailscale path and a LAN address for it shows up (mDNS,
a `hello.update`, a network change), `Shared::upgrade_path_soon` tries to move
the session:

1. wait up to 3 s (`UPGRADE_WAIT`) for iroh to switch paths on its own;
2. otherwise dial the LAN addresses only, with a 5 s limit (`UPGRADE_DIAL`);
3. on failure, keep the Tailscale session and don't retry that peer for 30 s
   (`UPGRADE_RETRY`).

The live session is never dropped for the attempt. Only the device with the
lower ID tries. When two connections to the same device meet,
`register_session` keeps the lower ID's dial on both sides and doesn't compare
paths, because each side sees its own selected path and the two could close
different connections.

## What this device advertises

`Shared::advertised_direct_addrs` builds the `hello` list: LAN addresses first
(IPv4, then IPv6), then corroborated Tailscale ones, and public addresses only
in Away mode. Never loopback, link-local or uncorroborated CGNAT.
`watch_local_addrs` waits 500 ms after iroh reports an address change, then
refreshes the Tailscale set and sends a `hello.update` if the list changed.

## Binding

With Away mode off the endpoint binds both `0.0.0.0` and `[::]`, so Tailscale's
IPv6 address answers as well as its IPv4 one. QUIC then often picks a global
IPv6 path on a home network. A peer address in the same `/64` as one of this
device's own global IPv6 addresses counts as LAN for connecting
(`Shared::is_on_link_ipv6`), because the router hands that prefix to this link
only. It still doesn't count for unlock. `NodeConfig::preferred_port`
asks for a fixed port (the desktop app passes 7718). A port saved from an
earlier run wins over it, so a restart keeps the address paired phones already
know.

## Pairing links

`pairing::link_addrs` fills a QR link's three address slots (`MAX_LINK_ADDRS`):

1. the best LAN address, always, so a phone on the same Wi-Fi can pair;
2. corroborated Tailscale addresses, IPv4 before IPv6;
3. more LAN addresses in the slots left, with at most one IPv6.

`start_host` takes these from `direct_addrs()` plus `Shared::tailscale_sockets`.
The joining side stores the link's addresses as `Discovery` candidates only
after pairing succeeds (`join_qr`), so a scanned link from a stranger leaves
nothing behind.

## Wireless ADB

A wireless ADB serial (`ip:port` or an `_adb-tls-connect._tcp` mDNS name) never
gets a USB worker (`usb::wants_usb_worker`, `nectarlink_adb::is_network_serial`).
Over Tailscale or Wi-Fi it would otherwise look like a USB path and claim USB's
priority and unlock rights.

## APIs for the apps

- `Node::set_local_tailscale_ips(ips)`: this device's Tailscale addresses.
- `Node::set_manual_addrs(peer, addrs)`: a user-entered address list.
- `Node::add_tailscale_candidates(peer, addrs)`: tailnet addresses for a peer,
  from the Tailscale client's peer list. Returns how many it kept.
- `Node::local_bound_addrs()`: the sockets the endpoint listens on.
- `Node::peer_addr_diagnostics(peer)`: every candidate with its source, kind
  (`lan`, `tailscale`, `cgnat`, `public`), first and last seen, whether it is
  dialable and whether the current path uses it, plus the path, whether this
  device has Tailscale, and Away mode.

The FFI exposes the same calls with addresses as strings.

## Privacy

No log line contains an address. Diagnostics return addresses to the app, which
shows them only on the device's own screens.

## Windows firewall

The installer's firewall rule covers Private and Domain networks. Windows often
files the Tailscale adapter as Public. I'd rather not add a Public-profile rule
for `100.64.0.0/10`, because carrier and hotspot networks use that range too.
Mark the Tailscale network Private once, in an administrator PowerShell:

```powershell
Set-NetConnectionProfile -InterfaceAlias Tailscale -NetworkCategory Private
```

The installer's existing rule then covers it.

## Limits

- mDNS doesn't cross Tailscale. A device paired before either side had a
  tailnet address learns it from the next `hello` on the LAN, or from
  `add_tailscale_candidates`.
- MagicDNS names aren't resolved. `set_manual_addrs` takes IP addresses only.
- Without corroboration (Tailscale off on this device), a peer's tailnet
  address is just a public address and needs Away mode.
