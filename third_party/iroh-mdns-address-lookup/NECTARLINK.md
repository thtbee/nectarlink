# iroh-mdns-address-lookup, patched

This is [iroh-mdns-address-lookup](https://crates.io/crates/iroh-mdns-address-lookup)
0.6.1 by the n0 team (MIT OR Apache-2.0, see `LICENSE-MIT` and
`LICENSE-APACHE`), with one change, marked "NectarLink patch" in `src/lib.rs`.

## The change

`MdnsAddressLookupBuilder::multicast_interfaces(include)` makes mDNS send and
receive on every local IPv4 address that `include` accepts, following the
endpoint's addresses as they change (swarm-discovery's per-interface sockets).

Without it, swarm-discovery joins the mDNS group on one interface only, the
one the OS picks for multicast. On a PC with Hyper-V, WSL or VPN adapters
ranked above Wi-Fi, that's a virtual adapter, so the PC never hears or
answers phones on the LAN (thtbee/nectarlink#148).

The tests in `src/lib.rs` are upstream's and need its dev-dependencies, so
they aren't built here.

## Dropping it

Upstream is working on the same problem
(n0-computer/iroh-address-lookups#7 and #26). Once a release has it, remove
this folder and the `[patch.crates-io]` entry in the workspace `Cargo.toml`,
and switch `install_mdns` in `core/nectarlink-core/src/node.rs` to the
upstream option.
